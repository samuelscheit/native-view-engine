//! Zero-copy `NVERELOC` pre-rendered Frame-0 scene export, hydration, and generic
//! [`prerender_native_frame0`] pipeline for [`HeadlessBackend`].

use std::collections::HashMap;
use std::sync::Arc;

use nve_abi::{
    encode_events, ColorRgba, ComponentKind, EventPayload, FrameId, NativeEvent, NativeEventKind,
    NodeId, RawPropValue, SurfaceId, ROOT_NODE_ID,
};
use nve_components::canvas_ir::{DrawListDecoder, ImageResizeMode};
use nve_components::style_patch::{Display, Overflow, TextAlign as PatchTextAlign};
use nve_engine::{
    encode_scene_reloc, BackendError, LayoutEngine, NveApp, RelocBuf, RelocError, RenderBackend,
    SceneRelocInput, SceneRelocNodeInput, SceneRelocView, TextMeasurer, VerifyMode, Viewport,
    SCENE_NODE_CLIP_CHILDREN, SCENE_NODE_HAS_EXPLICIT_STYLE, SCENE_NODE_HIDDEN,
    SCENE_NODE_INTERACTIVE,
};

use crate::{HeadlessBackend, HeadlessNode, TaffyLayoutEngine, MAX_TREE_DEPTH};

fn extract_font_weight(node: &HeadlessNode) -> u16 {
    let raw = node
        .raw_props
        .get("font-weight")
        .or_else(|| node.raw_props.get("fontWeight"));
    match raw {
        Some(RawPropValue::I64(n)) => (*n).clamp(100, 900) as u16,
        Some(RawPropValue::F64(n)) if n.is_finite() => (n.round() as i64).clamp(100, 900) as u16,
        Some(RawPropValue::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "thin" => 100,
            "extralight" | "ultralight" => 200,
            "light" => 300,
            "normal" | "regular" => 400,
            "medium" => 500,
            "semibold" | "demibold" => 600,
            "bold" => 700,
            "extrabold" | "ultrabold" => 800,
            "black" | "heavy" => 900,
            other => other.parse::<u16>().unwrap_or(400).clamp(100, 900),
        },
        _ => 400,
    }
}

fn extract_text_align(node: &HeadlessNode) -> u8 {
    let raw = node
        .raw_props
        .get("text-align")
        .or_else(|| node.raw_props.get("textAlign"));
    match raw {
        Some(RawPropValue::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "center" => 1,
            "right" | "end" => 2,
            _ => 0,
        },
        _ => 0,
    }
}

fn extract_image_resize_mode(node: &HeadlessNode) -> u8 {
    let raw = node
        .raw_props
        .get("resizeMode")
        .or_else(|| node.raw_props.get("resize_mode"));
    let mode = match raw {
        Some(RawPropValue::String(s)) => ImageResizeMode::parse(s).unwrap_or_default(),
        Some(RawPropValue::I64(n)) => ImageResizeMode::from_u8(*n as u8).unwrap_or_default(),
        _ => ImageResizeMode::default(),
    };
    mode as u8
}

fn should_clip_children(node: &HeadlessNode) -> bool {
    if node.kind == ComponentKind::ScrollView {
        return true;
    }
    if let Some(RawPropValue::String(ov)) = node.raw_props.get("overflow") {
        let lower = ov.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "hidden" | "clip" | "scroll" | "auto") {
            return true;
        }
    }
    matches!(node.raw_props.get("clip"), Some(RawPropValue::Bool(true)))
}

struct OwnedNodeData {
    node_id: NodeId,
    parent_index: i32,
    kind: ComponentKind,
    flags: u8,
    text_align: u8,
    image_resize_mode: u8,
    font_weight: u16,
    event_mask: u64,
    computed_rect: nve_engine::LayoutRect,
    style: nve_abi::StyleProps,
    layout: nve_abi::LayoutProps,
    text: String,
    tag: String,
    image_src: String,
    box_shadow: String,
    draw_list: Vec<u8>,
    children: Vec<u32>,
}

impl<L: LayoutEngine> HeadlessBackend<L> {
    /// Exports the current laid-out tree into a validated, 64-byte-aligned `NVERELOC` Frame-0
    /// scene (`SceneRelocView`) ready for zero-allocation native instant-boot painting.
    pub fn export_scene_reloc(
        &self,
        schema_version: u32,
        frame_id: FrameId,
        app_tag: &str,
        clear_color: ColorRgba,
    ) -> Result<RelocBuf, RelocError> {
        // Collect reachable tree nodes in pre-order starting from ROOT_NODE_ID.
        let mut ordered_ids: Vec<(NodeId, i32)> = Vec::new();
        let mut id_to_idx: HashMap<NodeId, u32> = HashMap::new();
        let mut stack: Vec<(NodeId, i32, usize)> = vec![(ROOT_NODE_ID, -1, 0)];

        while let Some((id, parent_idx, depth)) = stack.pop() {
            if depth > MAX_TREE_DEPTH || id_to_idx.contains_key(&id) {
                continue;
            }
            let Some(node) = self.nodes.get(id) else {
                continue;
            };
            let idx = ordered_ids.len() as u32;
            id_to_idx.insert(id, idx);
            ordered_ids.push((id, parent_idx));
            for &cid in node.children.iter().rev() {
                stack.push((cid, idx as i32, depth + 1));
            }
        }

        let mut owned: Vec<OwnedNodeData> = Vec::with_capacity(ordered_ids.len());
        for &(id, parent_index) in &ordered_ids {
            let node = self.nodes.get(id).expect("collected node exists");
            let patch = self.effective_patch(id);
            let mut style = node.style;
            if !patch.is_empty() {
                patch.apply_style_to(&mut style);
            }

            let mut flags = 0u8;
            let clips = match patch.overflow {
                Some(Overflow::Visible) => node.kind == ComponentKind::ScrollView,
                Some(Overflow::Hidden | Overflow::Scroll | Overflow::Auto) => true,
                None => should_clip_children(node),
            };
            if clips {
                flags |= SCENE_NODE_CLIP_CHILDREN;
            }
            if node.has_explicit_style {
                flags |= SCENE_NODE_HAS_EXPLICIT_STYLE;
            }
            if node.event_mask != 0 || node.kind == ComponentKind::Pressable {
                flags |= SCENE_NODE_INTERACTIVE;
            }
            if patch.display == Some(Display::None) {
                flags |= SCENE_NODE_HIDDEN;
            }

            let font_weight = patch.font_weight.unwrap_or_else(|| extract_font_weight(node));
            let text_align = patch
                .text_align
                .map(|a| match a {
                    PatchTextAlign::Center => 1,
                    PatchTextAlign::Right => 2,
                    PatchTextAlign::Left | PatchTextAlign::Justify => 0,
                })
                .unwrap_or_else(|| extract_text_align(node));

            let text = if let Some(t) = node.text.as_deref() {
                t.to_owned()
            } else if node.kind == ComponentKind::TextInput {
                match node
                    .raw_props
                    .get("value")
                    .or_else(|| node.raw_props.get("placeholder"))
                {
                    Some(RawPropValue::String(s)) => s.clone(),
                    _ => String::new(),
                }
            } else {
                String::new()
            };

            let tag = match node.raw_props.get("testid").or_else(|| node.raw_props.get("testID")) {
                Some(RawPropValue::String(s)) => s.clone(),
                _ => node.custom_name.clone().unwrap_or_default(),
            };

            let image_src = match node.raw_props.get("src") {
                Some(RawPropValue::String(s)) => s.clone(),
                _ => String::new(),
            };

            let box_shadow = match node
                .raw_props
                .get("box-shadow")
                .or_else(|| node.raw_props.get("boxShadow"))
            {
                Some(RawPropValue::String(s)) => s.clone(),
                _ => String::new(),
            };

            let draw_list = match node
                .raw_props
                .get("drawList")
                .or_else(|| node.raw_props.get("draw_list"))
            {
                Some(RawPropValue::Bytes(bytes)) => bytes.clone(),
                Some(RawPropValue::String(s)) => {
                    DrawListDecoder::decode_attr_bytes(s).unwrap_or_default()
                }
                _ => Vec::new(),
            };

            let children: Vec<u32> = node
                .children
                .iter()
                .filter_map(|cid| id_to_idx.get(cid).copied())
                .collect();

            owned.push(OwnedNodeData {
                node_id: id,
                parent_index,
                kind: node.kind,
                flags,
                text_align,
                image_resize_mode: extract_image_resize_mode(node),
                font_weight,
                event_mask: node.event_mask,
                computed_rect: node.computed_rect,
                style,
                layout: node.layout,
                text,
                tag,
                image_src,
                box_shadow,
                draw_list,
                children,
            });
        }

        let inputs: Vec<SceneRelocNodeInput<'_>> = owned
            .iter()
            .map(|n| SceneRelocNodeInput {
                node_id: n.node_id,
                parent_index: n.parent_index,
                kind: n.kind,
                flags: n.flags,
                text_align: n.text_align,
                image_resize_mode: n.image_resize_mode,
                font_weight: n.font_weight,
                event_mask: n.event_mask,
                computed_rect: n.computed_rect,
                style: n.style,
                layout: n.layout,
                text: &n.text,
                tag: &n.tag,
                image_src: &n.image_src,
                box_shadow: &n.box_shadow,
                draw_list: &n.draw_list,
                children: &n.children,
            })
            .collect();

        encode_scene_reloc(&SceneRelocInput {
            schema_version,
            surface_id: self.surface_id,
            frame_id,
            viewport: self.viewport,
            clear_color,
            root_node_index: 0,
            app_tag,
            nodes: &inputs,
        })
    }

    /// Hydrates this [`HeadlessBackend`] directly from a pre-rendered [`SceneRelocView`] without
    /// running Taffy layout (using pre-computed `computed_rect`s from `frame0.nvereloc`).
    ///
    /// Returns the number of hydrated nodes.
    pub fn load_reloc_scene(&mut self, view: SceneRelocView<'_>) -> Result<usize, BackendError> {
        let root = view.root();
        self.surface_id = root.surface_id;
        self.viewport = view.viewport();
        self.nodes.clear();
        self.prev_rects.clear();
        self.last_mutations.clear();
        self.layout.reset();

        let scene_nodes = view.nodes();
        if scene_nodes.is_empty() {
            self.ensure_node(ROOT_NODE_ID, ComponentKind::Root, None);
            return Ok(0);
        }

        for (idx, sn) in scene_nodes.iter().enumerate() {
            let id = sn.node_id;
            let kind = sn.component_kind();
            let text_str = view
                .text_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let tag_str = view
                .tag_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let image_src = view
                .image_src_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let box_shadow = view
                .box_shadow_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let draw_list = view
                .draw_list_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let child_indices = view
                .children_of(sn)
                .map_err(|e| BackendError::Internal(e.to_string()))?;

            let mut raw_props = HashMap::new();
            if !tag_str.is_empty() {
                raw_props.insert("testid".into(), RawPropValue::String(tag_str.to_owned()));
            }
            if !image_src.is_empty() {
                raw_props.insert("src".into(), RawPropValue::String(image_src.to_owned()));
                raw_props.insert(
                    "resizeMode".into(),
                    RawPropValue::I64(i64::from(sn.image_resize_mode)),
                );
            }
            if !box_shadow.is_empty() {
                raw_props.insert(
                    "box-shadow".into(),
                    RawPropValue::String(box_shadow.to_owned()),
                );
            }
            if !draw_list.is_empty() {
                raw_props.insert("drawList".into(), RawPropValue::Bytes(draw_list.to_vec()));
            }
            if sn.font_weight != 400 {
                raw_props.insert(
                    "font-weight".into(),
                    RawPropValue::I64(i64::from(sn.font_weight)),
                );
            }
            match sn.text_align {
                1 => {
                    raw_props.insert("text-align".into(), RawPropValue::String("center".into()));
                }
                2 => {
                    raw_props.insert("text-align".into(), RawPropValue::String("right".into()));
                }
                _ => {}
            }
            if sn.clips_children() && kind != ComponentKind::ScrollView {
                raw_props.insert("overflow".into(), RawPropValue::String("hidden".into()));
            }
            if kind == ComponentKind::TextInput && !text_str.is_empty() {
                raw_props.insert("value".into(), RawPropValue::String(text_str.to_owned()));
            }

            let parent = if sn.parent_index >= 0 {
                scene_nodes.get(sn.parent_index as usize).map(|p| p.node_id)
            } else {
                None
            };
            let children: Vec<NodeId> = child_indices
                .iter()
                .filter_map(|&cidx| scene_nodes.get(cidx as usize).map(|c| c.node_id))
                .collect();

            let layout = view.layout_of_index(idx).unwrap_or_default();
            let style = sn.style_props();
            let text_arc = (!text_str.is_empty()).then(|| Arc::<str>::from(text_str));

            self.layout.upsert_node(id, kind);
            self.layout.set_layout(id, &layout);
            self.layout.set_style(id, &style);
            if let Some(t) = text_arc.as_deref() {
                self.layout.set_text(id, t);
            }

            self.nodes.insert(
                id,
                HeadlessNode {
                    id,
                    kind,
                    custom_name: (kind == ComponentKind::Custom && !tag_str.is_empty())
                        .then(|| tag_str.to_owned()),
                    text: text_arc,
                    layout,
                    layout_ext: None,
                    transform: None,
                    style,
                    has_explicit_style: sn.has_explicit_style(),
                    raw_props,
                    draw_list: None,
                    native_view_props: None,
                    event_mask: sn.event_mask,
                    cursor: None,
                    state_rules: Vec::new(),
                    parent,
                    children,
                    computed_rect: sn.computed_rect(),
                },
            );
        }

        // Wire children into layout engine so a later `recompute_layout` or live diff stays exact.
        for sn in scene_nodes {
            if let Some(node) = self.nodes.get(sn.node_id)
                && !node.children.is_empty()
            {
                self.layout.set_children(sn.node_id, &node.children);
            }
        }

        Ok(scene_nodes.len())
    }

    /// Convenience entrypoint that validates `bytes` with `mode` and hydrates this backend.
    pub fn load_reloc_scene_bytes(
        &mut self,
        bytes: &[u8],
        mode: VerifyMode,
    ) -> Result<usize, BackendError> {
        let view =
            SceneRelocView::from_bytes(bytes, mode).map_err(|e| BackendError::Internal(e.to_string()))?;
        self.load_reloc_scene(view)
    }
}

/// Generic, app-agnostic Native Instant-Boot Frame-0 prerenderer.
///
/// Executes Frame 0 of any [`NveApp`] at `viewport`, computes Flexbox layout via
/// [`HeadlessBackend`] + [`TaffyLayoutEngine`], and emits a validated `frame0.nvereloc` buffer.
pub fn prerender_native_frame0<A: NveApp>(
    app: &mut A,
    surface_id: SurfaceId,
    viewport: Viewport,
    schema_version: u32,
    app_tag: &str,
    clear_color: ColorRgba,
) -> Result<RelocBuf, BackendError> {
    let backend = HeadlessBackend::<TaffyLayoutEngine>::new();
    prerender_native_frame0_into(
        app,
        backend,
        surface_id,
        viewport,
        schema_version,
        app_tag,
        clear_color,
    )
}

/// Variant of [`prerender_native_frame0`] accepting a custom [`TextMeasurer`] (such as
/// `nve_backend_skia::CosmicTextEngine`) for pixel-exact pre-baked text rects.
pub fn prerender_native_frame0_with_text_measurer<A: NveApp>(
    app: &mut A,
    surface_id: SurfaceId,
    viewport: Viewport,
    schema_version: u32,
    app_tag: &str,
    clear_color: ColorRgba,
    text_measurer: Arc<dyn TextMeasurer>,
) -> Result<RelocBuf, BackendError> {
    let mut backend = HeadlessBackend::<TaffyLayoutEngine>::new();
    backend.set_text_measurer(text_measurer);
    prerender_native_frame0_into(
        app,
        backend,
        surface_id,
        viewport,
        schema_version,
        app_tag,
        clear_color,
    )
}

fn prerender_native_frame0_into<A: NveApp, L: LayoutEngine>(
    app: &mut A,
    mut backend: HeadlessBackend<L>,
    surface_id: SurfaceId,
    viewport: Viewport,
    schema_version: u32,
    app_tag: &str,
    clear_color: ColorRgba,
) -> Result<RelocBuf, BackendError> {
    backend.set_viewport(surface_id, viewport);
    if viewport.width.is_finite()
        && viewport.height.is_finite()
        && viewport.width > 0.0
        && viewport.height > 0.0
    {
        let layout_ev = encode_events(&[NativeEvent {
            surface_id,
            node_id: ROOT_NODE_ID,
            timestamp_us: 0,
            kind: NativeEventKind::Layout,
            payload: EventPayload::Layout {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
        }]);
        let _ = app.dispatch_events(&layout_ev);
    }

    let frame0_cmd = app.render(1).to_vec();
    if !frame0_cmd.is_empty() {
        backend.submit_frame(&frame0_cmd)?;
    }
    backend
        .export_scene_reloc(schema_version, 1, app_tag, clear_color)
        .map_err(|e| BackendError::Internal(e.to_string()))
}
