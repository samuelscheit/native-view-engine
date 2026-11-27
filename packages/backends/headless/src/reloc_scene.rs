//! Zero-copy `NVERELOC` Frame-0 scene export from [`HeadlessBackend`] + [`NodeStore`], plus the
//! generic [`prerender_native_frame0`] pipeline for `NveApp` (`NVE2`) instant-boot baking.

use std::collections::HashMap;
use std::sync::Arc;

use nve_abi::{
    ColorRgba, FrameId, NveDirectEvent, NveEventBatchRef, SurfaceId, ROOT_NODE_ID,
};
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};
use nve_components::tables::{
    core_realization_order, write_box_shadows_css, BoxShadows, Display as NveDisplay, Image,
    ImageFit, ImageSrc, Interaction, Layout, Overflow, TextAlign, TextContent, Typography, Visual,
};
use nve_engine::{
    encode_scene_reloc, BackendError, LayoutRect, NveApp, RelocBuf, RelocError, SceneRelocInput,
    SceneRelocNodeInput, StoreBackend, StoreLayoutEngine, StoreWireReplica, TextMeasurer, Viewport,
    SCENE_NODE_CLIP_CHILDREN, SCENE_NODE_HAS_EXPLICIT_STYLE, SCENE_NODE_HIDDEN,
    SCENE_NODE_INTERACTIVE,
};
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::{HeadlessBackend, TaffyStoreLayout, MAX_TREE_DEPTH};

struct OwnedNodeData {
    node_id: u32,
    parent_index: i32,
    primary_slot: u8,
    flags: u8,
    text_align: u8,
    image_resize_mode: u8,
    font_weight: u16,
    event_mask: u64,
    computed_rect: LayoutRect,
    style: VisualSnapshot,
    layout: LayoutSnapshot,
    text: String,
    tag: String,
    image_src: String,
    box_shadow: String,
    children: Vec<u32>,
}

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    /// Exports the current laid-out `NodeStore` into a validated, 64-byte-aligned `NVERELOC`
    /// Frame-0 scene (`SceneRelocView`) ready for zero-allocation native instant-boot painting.
    pub fn export_scene_reloc(
        &self,
        store: &NodeStore,
        schema_version: u32,
        frame_id: FrameId,
        app_tag: &str,
        clear_color: ColorRgba,
    ) -> Result<RelocBuf, RelocError> {
        let mut ordered_ids: Vec<(StoreNodeId, i32)> = Vec::new();
        let mut id_to_idx: HashMap<StoreNodeId, u32> = HashMap::new();
        let mut stack: Vec<(StoreNodeId, i32, usize)> = vec![(StoreNodeId::ROOT, -1, 0)];

        while let Some((id, parent_idx, depth)) = stack.pop() {
            if depth > MAX_TREE_DEPTH || !store.is_live(id) || id_to_idx.contains_key(&id) {
                continue;
            }
            let idx = ordered_ids.len() as u32;
            id_to_idx.insert(id, idx);
            ordered_ids.push((id, parent_idx));
            for &cid in store.children(id).iter().rev() {
                stack.push((cid, idx as i32, depth + 1));
            }
        }

        let mut owned: Vec<OwnedNodeData> = Vec::with_capacity(ordered_ids.len());
        for &(id, parent_index) in &ordered_ids {
            let (layout, style) = self.effective_layout_and_visual(store, id);
            let mut flags = 0u8;
            if store.try_row::<Visual>(id).is_some_and(|v| {
                matches!(
                    v.overflow_x(),
                    Overflow::Hidden | Overflow::Scroll | Overflow::Auto
                ) || matches!(
                    v.overflow_y(),
                    Overflow::Hidden | Overflow::Scroll | Overflow::Auto
                )
            }) {
                flags |= SCENE_NODE_CLIP_CHILDREN;
            }
            if store.has::<Typography>(id) {
                flags |= SCENE_NODE_HAS_EXPLICIT_STYLE;
            }
            let event_mask = u64::from(
                store
                    .try_row::<Interaction>(id)
                    .map_or(0, |i| i.event_mask()),
            );
            if event_mask != 0 {
                flags |= SCENE_NODE_INTERACTIVE;
            }
            if store
                .try_row::<Layout>(id)
                .is_some_and(|l| l.display() == NveDisplay::None)
            {
                flags |= SCENE_NODE_HIDDEN;
            }
            let (text_align, font_weight) = store
                .try_row::<Typography>(id)
                .map(|t| {
                    let ta = match t.text_align() {
                        TextAlign::Start | TextAlign::Left => 0,
                        TextAlign::Center => 1,
                        TextAlign::End | TextAlign::Right => 2,
                        TextAlign::Justify => 3,
                    };
                    (ta, t.font_weight())
                })
                .unwrap_or((0, 400));
            let (image_src, image_resize_mode) = store
                .try_row::<Image>(id)
                .map(|img| {
                    let rm = match img.fit() {
                        ImageFit::Cover => 0,
                        ImageFit::Contain => 1,
                        ImageFit::Fill => 2,
                        ImageFit::None => 3,
                        ImageFit::ScaleDown => 4,
                    };
                    let url = match store.resource::<ImageSrc>(img.src()) {
                        Some(ImageSrc::Url(u)) => u.clone(),
                        _ => String::new(),
                    };
                    (url, rm)
                })
                .unwrap_or_default();
            let text = store
                .try_row::<TextContent>(id)
                .map(|t| t.text().to_owned())
                .unwrap_or_default();
            let box_shadow = store
                .try_row::<Visual>(id)
                .map(|v| v.box_shadows())
                .filter(|r| !r.is_none())
                .and_then(|r| store.resource::<BoxShadows>(r))
                .filter(|bs| !bs.0.is_empty())
                .map(|bs| {
                    let mut css = String::new();
                    write_box_shadows_css(&mut css, &bs.0);
                    css
                })
                .unwrap_or_default();
            let children: Vec<u32> = store
                .children(id)
                .iter()
                .filter_map(|cid| id_to_idx.get(cid).copied())
                .collect();

            owned.push(OwnedNodeData {
                node_id: id.raw(),
                parent_index,
                primary_slot: store.primary_slot(id).0,
                flags,
                text_align,
                image_resize_mode,
                font_weight,
                event_mask,
                computed_rect: self.layout.layout_of(id).unwrap_or_default(),
                style,
                layout,
                text,
                tag: String::new(),
                image_src,
                box_shadow,
                children,
            });
        }

        let inputs: Vec<SceneRelocNodeInput<'_>> = owned
            .iter()
            .map(|n| SceneRelocNodeInput {
                node_id: n.node_id,
                parent_index: n.parent_index,
                primary_slot: n.primary_slot,
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
                draw_list: &[],
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
}

/// Generic, app-agnostic Native Instant-Boot Frame-0 prerenderer (`NVE2` -> `frame0.nvereloc`).
pub fn prerender_native_frame0<A: NveApp>(
    app: &mut A,
    surface_id: SurfaceId,
    viewport: Viewport,
    schema_version: u32,
    app_tag: &str,
    clear_color: ColorRgba,
) -> Result<RelocBuf, BackendError> {
    prerender_native_frame0_into(
        app,
        HeadlessBackend::<TaffyStoreLayout>::new(),
        surface_id,
        viewport,
        schema_version,
        app_tag,
        clear_color,
    )
}

/// Variant of [`prerender_native_frame0`] accepting a custom [`TextMeasurer`].
pub fn prerender_native_frame0_with_text_measurer<A: NveApp>(
    app: &mut A,
    surface_id: SurfaceId,
    viewport: Viewport,
    schema_version: u32,
    app_tag: &str,
    clear_color: ColorRgba,
    text_measurer: Arc<dyn TextMeasurer>,
) -> Result<RelocBuf, BackendError> {
    let mut backend = HeadlessBackend::<TaffyStoreLayout>::new();
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

fn prerender_native_frame0_into<A: NveApp, L: StoreLayoutEngine>(
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
        let layout_ev = [NveDirectEvent::layout(
            surface_id,
            ROOT_NODE_ID,
            0,
            0.0,
            0.0,
            viewport.width,
            viewport.height,
        )];
        let _ = app.dispatch_events(unsafe { NveEventBatchRef::from_slice(&layout_ev) });
    }

    let mut replica = StoreWireReplica::new(backend, core_realization_order());
    let frame0_nve2 = app.render(1).to_vec();
    if !frame0_nve2.is_empty() {
        replica.submit_nve2_frame(&frame0_nve2)?;
    }
    replica
        .backend()
        .export_scene_reloc(replica.store(), schema_version, 1, app_tag, clear_color)
        .map_err(|e| BackendError::Internal(e.to_string()))
}
