//! `RenderBackend` implementation for [`WebBackend`]: binary `CommandBuffer` frame application,
//! batched style/canvas flushing, and hydration completion.

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, EventMask, NativeEvent, NodeId, SurfaceId,
    ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

use crate::canvas;
use crate::css::{is_semantic_tag_prop, resolve_semantic_tag, MirrorElement};
#[cfg(target_arch = "wasm32")]
use crate::dom;
use crate::dom::{raw_prop_route, RawPropRoute};
#[cfg(target_arch = "wasm32")]
use crate::events::now_us;
use crate::events::NodeEventMeta;
#[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
use crate::hydrate::NativeSsrNode;
#[cfg(any(
    feature = "nv-blur",
    feature = "nv-webview",
    feature = "nv-video",
    feature = "nv-web-element",
    feature = "camera",
))]
#[cfg(target_arch = "wasm32")]
use crate::native_view;
use crate::WebBackend;

#[cfg(not(target_arch = "wasm32"))]
struct FrameTimer(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl FrameTimer {
    fn start() -> Self {
        Self(std::time::Instant::now())
    }
    fn elapsed_us(&self) -> u64 {
        self.0.elapsed().as_micros() as u64
    }
}

#[cfg(target_arch = "wasm32")]
struct FrameTimer(u64);

#[cfg(target_arch = "wasm32")]
impl FrameTimer {
    fn start() -> Self {
        Self(now_us())
    }
    fn elapsed_us(&self) -> u64 {
        now_us().saturating_sub(self.0)
    }
}

impl RenderBackend for WebBackend {
    fn name(&self) -> &'static str {
        if cfg!(target_arch = "wasm32") {
            "web-dom-wasm"
        } else {
            "web-dom-mirror"
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id.set(surface_id);
        self.viewport = viewport;
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let t_dec = FrameTimer::start();
        let (header, commands) = CommandDecoder::new(command_bytes)?.decode_all()?;
        let decode_us = t_dec.elapsed_us();
        self.surface_id.set(header.surface_id);
        self.frame_counter = self.frame_counter.saturating_add(1);
        let current_frame = self.frame_counter;

        let t_mount = FrameTimer::start();
        let mut mutations = 0u32;
        let mut replaced_existing = false;
        let mut dirty_styles: Vec<NodeId> = Vec::new();
        #[cfg(feature = "hydrate")]
        let mut hydrated_seen: Vec<NodeId> = Vec::new();
        #[cfg(target_arch = "wasm32")]
        let mut dirty_canvases: Vec<NodeId> = Vec::new();

        for cmd in commands {
            match cmd {
                // The root is the host's mount container: never replaced or removed.
                Command::CreateNode { node_id: ROOT_NODE_ID, .. }
                | Command::RemoveNode { node_id: ROOT_NODE_ID } => {}
                Command::CreateNode { node_id, kind, custom_name } => {
                    if self.nodes.contains_key(&node_id) {
                        replaced_existing = true;
                    }
                    #[cfg(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    ))]
                    self.native_views.remove(&node_id);
                    #[cfg(feature = "hydrate")]
                    let mut mirror = MirrorElement::new(kind);
                    #[cfg(not(feature = "hydrate"))]
                    let mirror = MirrorElement::new(kind);
                    #[cfg(feature = "hydrate")]
                    let adopting_ssr = self.hydrating && self.has_ssr_dom_node(node_id);
                    #[cfg(not(feature = "hydrate"))]
                    let adopting_ssr = false;
                    #[cfg(feature = "hydrate")]
                    if adopting_ssr {
                        mirror.has_inline_style = self.hydrated_inline_nodes.contains(&node_id);
                        hydrated_seen.push(node_id);
                    }
                    self.nodes.insert(node_id, mirror);
                    #[cfg(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    ))]
                    if kind == ComponentKind::Custom
                        && let Some(name) = custom_name.as_deref()
                    {
                        self.init_custom_native_view(node_id, name);
                    }
                    #[cfg(not(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    )))]
                    let _ = custom_name;
                    #[cfg(any(target_arch = "wasm32", feature = "hydrate"))]
                    let eff_tag = self
                        .nodes
                        .get(&node_id)
                        .map(|m| self.effective_html_tag(node_id, m).to_owned())
                        .unwrap_or_else(|| "div".to_owned());
                    self.node_event_meta
                        .borrow_mut()
                        .insert(node_id, NodeEventMeta { kind, mask: EventMask::NONE });
                    if !adopting_ssr {
                        #[cfg(target_arch = "wasm32")]
                        {
                            if let Some(old_el) = self.dom_nodes.remove(&node_id) {
                                old_el.remove();
                            }
                            self.ime_guards.borrow_mut().remove(&node_id);
                            if let Some(doc) = web_sys::window().and_then(|w| w.document())
                                && let Some(el) = dom::create_dom_element_for_tag(&doc, &eff_tag)
                            {
                                self.attr_scratch.clear();
                                let _ = write!(&mut self.attr_scratch, "{node_id}");
                                let _ = el.set_attribute("data-nve-id", &self.attr_scratch);
                                #[cfg(any(
                                    feature = "nv-blur",
                                    feature = "nv-webview",
                                    feature = "nv-video",
                                    feature = "nv-web-element",
                                    feature = "camera",
                                ))]
                                if let Some(nv) = self.native_views.get_mut(&node_id) {
                                    native_view::sync_native_view_dom_attrs(&el, nv);
                                }
                                self.dom_nodes.insert(node_id, el);
                            }
                        }
                        #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
                        {
                            let new_id = self.next_native_instance_id;
                            self.next_native_instance_id += 1;
                            self.native_ssr_nodes.insert(
                                node_id,
                                NativeSsrNode {
                                    tag: eff_tag,
                                    instance_id: new_id,
                                    has_inline_style: false,
                                    control_value: None,
                                },
                            );
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    if kind == ComponentKind::SkiaCanvas {
                        dirty_canvases.push(node_id);
                    }
                    dirty_styles.push(node_id);
                    mutations += 1;
                }
                Command::RemoveNode { node_id } => {
                    self.detach_from_parent(node_id);
                    self.nodes.remove(&node_id);
                    #[cfg(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    ))]
                    self.native_views.remove(&node_id);
                    self.remove_node_event_meta(node_id);
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = self.dom_nodes.remove(&node_id) {
                        el.remove();
                    }
                    #[cfg(all(not(target_arch = "wasm32"), feature = "hydrate"))]
                    self.native_ssr_nodes.remove(&node_id);
                    mutations += 1;
                }
                Command::SetChildren { parent_id, children } => {
                    let Some(children) = self.adopt_children(parent_id, &children) else {
                        continue;
                    };
                    let is_text_parent = self
                        .nodes
                        .get(&parent_id)
                        .is_some_and(|p| p.kind == ComponentKind::Text);
                    if is_text_parent {
                        for &cid in &children {
                            dirty_styles.push(cid);
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    self.reconcile_dom_children(parent_id, &children);
                    if let Some(p) = self.nodes.get_mut(&parent_id) {
                        p.children = children;
                    }
                    mutations += 1;
                }
                Command::SetText { node_id, text } => {
                    self.apply_text_mutation(node_id, &text);
                    mutations += 1;
                }
                Command::SetLayout { node_id, layout } => {
                    #[cfg(target_arch = "wasm32")]
                    let mut is_canvas = false;
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.layout = layout;
                        el.record_style_mutation(current_frame);
                        #[cfg(target_arch = "wasm32")]
                        {
                            is_canvas = el.kind == ComponentKind::SkiaCanvas;
                        }
                    }
                    dirty_styles.push(node_id);
                    #[cfg(target_arch = "wasm32")]
                    if is_canvas {
                        dirty_canvases.push(node_id);
                    }
                    mutations += 1;
                }
                Command::SetStyle { node_id, style } => {
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.style = style;
                        el.record_style_mutation(current_frame);
                    }
                    dirty_styles.push(node_id);
                    self.queue_leaf_text_children(node_id, &mut dirty_styles);
                    mutations += 1;
                }
                Command::SetRawProp { node_id, key, value } => {
                    match raw_prop_route(&key) {
                        RawPropRoute::DrawList => {
                            let decoded_ops = canvas::decode_draw_list_prop(&value).unwrap_or_default();
                            if let Some(el) = self.nodes.get_mut(&node_id) {
                                el.draw_op_count = decoded_ops.len();
                                el.draw_ops = decoded_ops;
                                el.raw_props.insert(key, value);
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                self.sync_dom_draw_ops_attr(node_id);
                                dirty_canvases.push(node_id);
                            }
                        }
                        RawPropRoute::NativeView => {
                            #[cfg(any(
                                feature = "nv-blur",
                                feature = "nv-webview",
                                feature = "nv-video",
                                feature = "nv-web-element",
                                feature = "camera",
                            ))]
                            {
                                if let Some(el) = self.nodes.get_mut(&node_id) {
                                    el.raw_props.insert(key.clone(), value.clone());
                                }
                                self.apply_native_view_prop(node_id, &key, &value, &mut dirty_styles);
                            }
                        }
                        RawPropRoute::Style => {
                            if let Some(el) = self.nodes.get_mut(&node_id) {
                                el.raw_style.apply_prop(&key, &value);
                                el.raw_props.insert(key, value);
                                el.record_style_mutation(current_frame);
                            }
                            dirty_styles.push(node_id);
                            self.queue_leaf_text_children(node_id, &mut dirty_styles);
                        }
                        RawPropRoute::Reserved => {}
                        RawPropRoute::Attribute => {
                            let mut tag_changed = false;
                            if let Some(el) = self.nodes.get_mut(&node_id) {
                                el.raw_props.insert(key.clone(), value.clone());
                                // The root container keeps its tag.
                                if node_id != ROOT_NODE_ID && is_semantic_tag_prop(&key) {
                                    let sem = resolve_semantic_tag(el.kind, &el.raw_props);
                                    if el.tag != sem.tag || el.wrap_code != sem.wrap_code {
                                        el.tag = sem.tag;
                                        el.wrap_code = sem.wrap_code;
                                        tag_changed = true;
                                    }
                                }
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                #[cfg(feature = "hydrate")]
                                let skip_upgrade = self.hydrating;
                                #[cfg(not(feature = "hydrate"))]
                                let skip_upgrade = false;
                                if tag_changed && !skip_upgrade {
                                    self.upgrade_dom_element_tag(node_id);
                                    dirty_styles.push(node_id);
                                } else {
                                    self.sync_dom_raw_prop(node_id, &key, &value);
                                }
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            let _ = tag_changed;
                        }
                    }
                    mutations += 1;
                }
                Command::SetEventMask { node_id, mask } => {
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.event_mask = mask;
                        let kind = el.kind;
                        self.node_event_meta
                            .borrow_mut()
                            .insert(node_id, NodeEventMeta { kind, mask });
                    }
                    dirty_styles.push(node_id);
                    mutations += 1;
                }
                Command::DispatchCommand { node_id, command_name, payload } => {
                    #[cfg(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    ))]
                    if self.dispatch_native_view_command(node_id, &command_name, &payload) {
                        mutations += 1;
                    }
                    #[cfg(not(any(
                        feature = "nv-blur",
                        feature = "nv-webview",
                        feature = "nv-video",
                        feature = "nv-web-element",
                        feature = "camera",
                    )))]
                    let _ = (node_id, command_name, payload);
                }
                Command::Commit { .. } => {}
            }
        }

        if replaced_existing {
            self.prune_unreachable_nodes();
        }

        #[cfg(feature = "hydrate")]
        {
            if !self.pending_responsive_dirty.is_empty() {
                dirty_styles.append(&mut self.pending_responsive_dirty);
            }
            if self.hydrating {
                self.finalize_hydration(&hydrated_seen, &mut dirty_styles);
            }
        }

        // Flush surviving dirty nodes ONCE after RemoveNode / prune_unreachable_nodes.
        if !dirty_styles.is_empty() {
            dirty_styles.sort_unstable();
            dirty_styles.dedup();
            self.flush_dirty_styles(&dirty_styles);
        }

        #[cfg(target_arch = "wasm32")]
        if !dirty_canvases.is_empty() {
            dirty_canvases.sort_unstable();
            dirty_canvases.dedup();
            for nid in dirty_canvases {
                self.repaint_canvas_node(nid);
            }
        }

        let mount_us = t_mount.elapsed_us();
        Ok(BackendApplyStats {
            op_count: header.op_count,
            mutation_count: mutations,
            decode_us,
            layout_us: 0,
            diff_us: 0,
            mount_us,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        #[cfg(target_arch = "wasm32")]
        {
            self.dom_nodes.get(&node_id).map(|el| {
                let r = el.get_bounding_client_rect();
                LayoutRect {
                    x: r.x() as f32,
                    y: r.y() as f32,
                    width: r.width() as f32,
                    height: r.height() as f32,
                }
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = node_id;
            None
        }
    }
}
