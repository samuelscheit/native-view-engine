//! Direct zero-overhead [`RenderSink`] and binary [`RenderBackend`] frame application for
//! [`WebBackend`]: single-pass DOM/CSS mutation batching, canvas/SVG/native-view/responsive/state
//! delivery, and hydration completion.

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;

use nve_abi::{
    ComponentKind, EventMask, FrameId, LayoutProps, NativeEvent, NodeId, RawPropValueRef,
    StyleProps, SurfaceId, ROOT_NODE_ID,
};
use nve_components::canvas_ir::DrawList;
use nve_components::interaction::{CursorKind, StateStyleRule};
use nve_components::layout_ext::LayoutExt;
use nve_components::media::{MediaQuery, ResponsiveRule};
use nve_components::motion::TransitionSpec;
use nve_components::native_view::TypedNativeViewProps;
use nve_components::paint::BackgroundFill;
use nve_components::svg::SvgPicture;
use nve_components::transform::Transform2D;
use nve_engine::{
    replay_command_buffer_into, BackendApplyStats, BackendError, LayoutRect, RenderBackend,
    RenderSink, Viewport,
};

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

impl WebBackend {
    fn finalize_web_frame(&mut self, committed: bool) -> Result<BackendApplyStats, BackendError> {
        let t_mount = FrameTimer::start();

        if self.frame_replaced_existing {
            self.prune_unreachable_nodes();
        }

        let mut dirty_styles = std::mem::take(&mut self.frame_dirty_styles);

        #[cfg(feature = "hydrate")]
        {
            if !self.pending_responsive_dirty.is_empty() {
                dirty_styles.append(&mut self.pending_responsive_dirty);
            }
            if self.hydrating {
                let hydrated_seen = std::mem::take(&mut self.frame_hydrated_seen);
                self.finalize_hydration(&hydrated_seen, &mut dirty_styles);
                self.frame_hydrated_seen = hydrated_seen;
                self.frame_hydrated_seen.clear();
            }
        }

        // Flush surviving dirty nodes ONCE after RemoveNode / prune_unreachable_nodes / extended props.
        if !dirty_styles.is_empty() {
            dirty_styles.sort_unstable();
            dirty_styles.dedup();
            self.flush_dirty_styles(&dirty_styles);
        }
        dirty_styles.clear();
        self.frame_dirty_styles = dirty_styles;

        #[cfg(target_arch = "wasm32")]
        {
            let mut dirty_dirs = std::mem::take(&mut self.frame_dirty_dirs);
            if !dirty_dirs.is_empty() {
                dirty_dirs.sort_unstable();
                dirty_dirs.dedup();
                for &nid in &dirty_dirs {
                    self.sync_dom_dir_attr(nid);
                }
                dirty_dirs.clear();
            }
            self.frame_dirty_dirs = dirty_dirs;

            let mut dirty_svgs = std::mem::take(&mut self.frame_dirty_svgs);
            if !dirty_svgs.is_empty() {
                dirty_svgs.sort_unstable();
                dirty_svgs.dedup();
                for &nid in &dirty_svgs {
                    self.sync_dom_svg_node(nid);
                }
                dirty_svgs.clear();
            }
            self.frame_dirty_svgs = dirty_svgs;

            let mut dirty_canvases = std::mem::take(&mut self.frame_dirty_canvases);
            if !dirty_canvases.is_empty() {
                dirty_canvases.sort_unstable();
                dirty_canvases.dedup();
                for &nid in &dirty_canvases {
                    self.sync_dom_draw_ops_attr(nid);
                    self.repaint_canvas_node(nid);
                }
                dirty_canvases.clear();
            }
            self.frame_dirty_canvases = dirty_canvases;
        }

        let mount_us = t_mount.elapsed_us();
        let op_count = if committed {
            self.frame_op_count.saturating_add(1)
        } else {
            self.frame_op_count
        };
        Ok(BackendApplyStats {
            op_count,
            mutation_count: self.frame_mutations,
            decode_us: 0,
            layout_us: 0,
            diff_us: 0,
            mount_us,
        })
    }
}

impl RenderSink for WebBackend {
    fn begin_frame(&mut self, surface_id: SurfaceId, _frame_id: FrameId) {
        self.surface_id.set(surface_id);
        self.frame_counter = self.frame_counter.saturating_add(1);
        self.frame_op_count = 0;
        self.frame_mutations = 0;
        self.frame_replaced_existing = false;
        self.frame_dirty_styles.clear();
        #[cfg(feature = "hydrate")]
        self.frame_hydrated_seen.clear();
        #[cfg(target_arch = "wasm32")]
        {
            self.frame_dirty_canvases.clear();
            self.frame_dirty_svgs.clear();
            self.frame_dirty_dirs.clear();
        }
    }

    fn create_node(&mut self, node_id: NodeId, kind: ComponentKind, custom_name: Option<&str>) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        // The root is the host's mount container: never replaced or removed.
        if node_id == ROOT_NODE_ID {
            return;
        }
        if self.nodes.contains_key(&node_id) {
            self.frame_replaced_existing = true;
        }
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        self.native_views.remove(&node_id);
        let mut mirror = MirrorElement::new(kind);
        let is_svg_custom = kind == ComponentKind::Custom && custom_name == Some("svg");
        if is_svg_custom {
            mirror.tag = "svg";
        }
        #[cfg(feature = "hydrate")]
        let adopting_ssr = self.hydrating && self.has_ssr_dom_node(node_id);
        #[cfg(not(feature = "hydrate"))]
        let adopting_ssr = false;
        #[cfg(feature = "hydrate")]
        if adopting_ssr {
            mirror.has_inline_style = self.hydrated_inline_nodes.contains(&node_id);
            self.frame_hydrated_seen.push(node_id);
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
            && !is_svg_custom
            && let Some(name) = custom_name
        {
            self.init_custom_native_view(node_id, name);
        }
        #[cfg(any(target_arch = "wasm32", feature = "hydrate"))]
        let eff_tag = self
            .nodes
            .get(&node_id)
            .map(|m| self.effective_html_tag(node_id, m).to_owned())
            .unwrap_or_else(|| "div".to_owned());
        self.node_event_meta.borrow_mut().insert(
            node_id,
            NodeEventMeta {
                kind,
                mask: EventMask::NONE,
            },
        );
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
            self.frame_dirty_canvases.push(node_id);
        }
        self.frame_dirty_styles.push(node_id);
        self.frame_mutations += 1;
    }

    fn remove_node(&mut self, node_id: NodeId) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        if node_id == ROOT_NODE_ID {
            return;
        }
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
        self.frame_mutations += 1;
    }

    fn set_children(&mut self, parent_id: NodeId, children: &[NodeId]) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let Some(adopted) = self.adopt_children(parent_id, children) else {
            return;
        };
        let is_text_parent = self
            .nodes
            .get(&parent_id)
            .is_some_and(|p| p.kind == ComponentKind::Text);
        if is_text_parent {
            self.frame_dirty_styles.extend_from_slice(&adopted);
        }
        #[cfg(target_arch = "wasm32")]
        self.reconcile_dom_children(parent_id, &adopted);
        if let Some(p) = self.nodes.get_mut(&parent_id) {
            p.children = adopted;
        }
        self.frame_mutations += 1;
    }

    fn set_text(&mut self, node_id: NodeId, text: &str) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        self.apply_text_mutation(node_id, text);
        self.frame_mutations += 1;
    }

    fn set_layout(&mut self, node_id: NodeId, layout: &LayoutProps) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let current_frame = self.frame_counter;
        #[cfg(target_arch = "wasm32")]
        let mut is_canvas = false;
        if let Some(el) = self.nodes.get_mut(&node_id) {
            el.layout = *layout;
            el.record_style_mutation(current_frame);
            #[cfg(target_arch = "wasm32")]
            {
                is_canvas = el.kind == ComponentKind::SkiaCanvas;
            }
        }
        self.frame_dirty_styles.push(node_id);
        #[cfg(target_arch = "wasm32")]
        if is_canvas {
            self.frame_dirty_canvases.push(node_id);
        }
        self.frame_mutations += 1;
    }

    fn set_style(&mut self, node_id: NodeId, style: &StyleProps) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let current_frame = self.frame_counter;
        #[cfg(target_arch = "wasm32")]
        let mut has_svg = false;
        if let Some(el) = self.nodes.get_mut(&node_id) {
            el.style = *style;
            el.record_style_mutation(current_frame);
            #[cfg(target_arch = "wasm32")]
            {
                has_svg = el.svg.is_some();
            }
        }
        #[cfg(target_arch = "wasm32")]
        if has_svg {
            self.frame_dirty_svgs.push(node_id);
        }
        self.frame_dirty_styles.push(node_id);
        let mut dirty = std::mem::take(&mut self.frame_dirty_styles);
        self.queue_leaf_text_children(node_id, &mut dirty);
        self.frame_dirty_styles = dirty;
        self.frame_mutations += 1;
    }

    fn set_raw_prop(&mut self, node_id: NodeId, key: &str, value_ref: RawPropValueRef<'_>) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        let current_frame = self.frame_counter;
        match raw_prop_route(key) {
            RawPropRoute::DrawList => {
                let value = value_ref.to_owned();
                let decoded_ops = canvas::decode_draw_list_prop(&value)
                    .map(DrawList::from_ops)
                    .unwrap_or_default();
                if let Some(el) = self.nodes.get_mut(&node_id) {
                    el.draw_op_count = decoded_ops.len();
                    el.draw_ops = decoded_ops;
                    el.raw_props.insert(key.to_owned(), value);
                }
                #[cfg(target_arch = "wasm32")]
                self.frame_dirty_canvases.push(node_id);
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
                    let value = value_ref.to_owned();
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.raw_props.insert(key.to_owned(), value.clone());
                    }
                    let mut dirty = std::mem::take(&mut self.frame_dirty_styles);
                    self.apply_native_view_prop(node_id, key, &value, &mut dirty);
                    self.frame_dirty_styles = dirty;
                }
            }
            RawPropRoute::Style => {
                let value = value_ref.to_owned();
                if let Some(el) = self.nodes.get_mut(&node_id) {
                    el.raw_style.apply_prop(key, &value);
                    el.raw_props.insert(key.to_owned(), value);
                    el.record_style_mutation(current_frame);
                }
                self.frame_dirty_styles.push(node_id);
                let mut dirty = std::mem::take(&mut self.frame_dirty_styles);
                self.queue_leaf_text_children(node_id, &mut dirty);
                self.frame_dirty_styles = dirty;
            }
            RawPropRoute::Reserved => {}
            RawPropRoute::Attribute => {
                let value = value_ref.to_owned();
                let mut tag_changed = false;
                if let Some(el) = self.nodes.get_mut(&node_id) {
                    el.raw_props.insert(key.to_owned(), value.clone());
                    if node_id != ROOT_NODE_ID && is_semantic_tag_prop(key) {
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
                        self.frame_dirty_styles.push(node_id);
                    } else {
                        self.sync_dom_raw_prop(node_id, key, &value);
                    }
                    if key == "tint"
                        && self.nodes.get(&node_id).is_some_and(|n| n.svg.is_some())
                    {
                        self.frame_dirty_svgs.push(node_id);
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                let _ = tag_changed;
            }
        }
        self.frame_mutations += 1;
    }

    fn set_event_mask(&mut self, node_id: NodeId, mask: u64) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        if let Some(el) = self.nodes.get_mut(&node_id) {
            el.event_mask = mask;
            let kind = el.kind;
            self.node_event_meta
                .borrow_mut()
                .insert(node_id, NodeEventMeta { kind, mask });
        }
        self.frame_dirty_styles.push(node_id);
        self.frame_mutations += 1;
    }

    fn dispatch_command(&mut self, node_id: NodeId, command_name: &str, payload: &[u8]) {
        self.frame_op_count = self.frame_op_count.saturating_add(1);
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if self.dispatch_native_view_command(node_id, command_name, payload) {
            self.frame_mutations += 1;
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

    fn set_layout_ext(&mut self, node_id: NodeId, ext: Option<&LayoutExt>) {
        self.apply_frame_layout_ext(node_id, ext);
    }

    fn set_transform(&mut self, node_id: NodeId, transform: Option<Transform2D>) {
        self.apply_frame_transform(node_id, transform);
    }

    fn set_cursor(&mut self, node_id: NodeId, cursor: Option<CursorKind>) {
        self.apply_frame_cursor(node_id, cursor);
    }

    fn set_state_rules(&mut self, node_id: NodeId, rules: &[StateStyleRule]) {
        self.apply_frame_state_rules(node_id, rules);
    }

    fn set_transitions(&mut self, node_id: NodeId, transitions: &[TransitionSpec]) {
        self.apply_frame_transitions(node_id, transitions);
    }

    fn set_show_on(&mut self, node_id: NodeId, query: Option<MediaQuery>) {
        self.apply_frame_show_on(node_id, query);
    }

    fn set_responsive_rules(&mut self, node_id: NodeId, rules: &[ResponsiveRule]) {
        self.apply_frame_responsive_rules(node_id, rules);
    }

    fn set_background_fill(&mut self, node_id: NodeId, fill: Option<&BackgroundFill>) {
        self.apply_frame_background_fill(node_id, fill);
    }

    fn set_svg(&mut self, node_id: NodeId, svg: Option<&SvgPicture>) {
        self.apply_frame_svg(node_id, svg);
    }

    fn set_draw_list(&mut self, node_id: NodeId, draw_list: Option<&DrawList>) {
        self.apply_frame_draw_list(node_id, draw_list);
    }

    fn set_native_view_props(&mut self, node_id: NodeId, props: Option<&TypedNativeViewProps>) {
        self.apply_frame_native_view_props(node_id, props);
    }

    fn commit_frame(&mut self, _sync_mount: bool) -> Result<BackendApplyStats, BackendError> {
        self.finalize_web_frame(true)
    }

    fn finish_uncommitted(&mut self) -> Result<BackendApplyStats, BackendError> {
        self.finalize_web_frame(false)
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
        replay_command_buffer_into(command_bytes, self)
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
