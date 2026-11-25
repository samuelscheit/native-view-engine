//! Web DOM / WASM RenderBackend consuming the exact same `nve-abi` `CommandBuffer`.
//!
//! - **Option 5 Hybrid CSS**: static `.nve` base reset (`NVE_BASE_RESET_CSS`), bounded
//!   content-addressed structural stylesheet cache (`.s<base36(fnv1a64)>`), volatile/continuous
//!   inline lane, `.nve-tl` leaf-text inheritance, and deferred once-per-frame style flushing.
//! - **Keyed Minimal-Diff `SetChildren`**: O(N) head/tail-trimmed + LIS DOM reconciler preserving
//!   element identity, focus, `<input>` cursor/IME, and `ScrollView` scroll position.
//! - **Semantic HTML5 SSR**: `render_ssr()` and `render_ssr_document()` mapping semantic ARIA/link
//!   props onto real HTML5 tags with deterministic CSS rules.
//! - **Zero-Flicker Hydration & Root Event Delegation**: `hydrate_into()` adopts SSR DOM nodes
//!   in-place with a single root delegated listener set and structured `HydrationMismatch` reports.

pub mod canvas;
mod css;
mod dom;
mod events;
mod hot_patch;
mod hydrate;
mod reconcile;
mod ssr;
mod style_hash;
mod stylesheet;
#[cfg(test)]
mod ssr_hydrate_tests;
#[cfg(test)]
mod tests;

pub use css::{
    canonical_f32_bits, component_to_html_tag, is_image_fit_prop, is_raw_style_prop,
    is_semantic_tag_prop, layout_to_css, layout_to_css_into, raw_prop_to_object_fit,
    resolve_semantic_tag, style_to_css, style_to_css_into, write_compact_color,
    write_compact_edges, MirrorElement, RawStyleProps, SemanticTag, NVE_BASE_RESET_CSS,
};
pub use dom::{is_truthy_raw_prop, map_a11y_or_input_attr, MappedHtmlAttr};
pub use events::{
    encode_keydown_payload, resolve_delegated_node, ImeCompositionGuard, NodeEventMeta,
};
pub use hydrate::{HydrationMismatch, HydrationReport};
pub use reconcile::{plan_keyed_child_ops, ChildDiffOp};
pub use ssr::{SsrDocumentOptions, SsrOutput};
pub use style_hash::{
    format_structural_class, hash_structural_style, is_inheriting_leaf_text,
    is_volatile_dimension, is_volatile_opacity, VOLATILE_DIMENSION_LIMIT,
};
pub use stylesheet::{
    resolve_node_style, ResolvedNodeStyle, StyleSheetCache, INCREMENTAL_RULE_BATCH_THRESHOLD,
    MAX_DYNAMIC_RULES,
};

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, EventMask, NativeEvent, NodeId, SurfaceId,
    ROOT_NODE_ID,
};
#[cfg(target_arch = "wasm32")]
use nve_abi::Dimension;
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::events::{now_us, RootEventDelegator};
#[cfg(not(target_arch = "wasm32"))]
use crate::hydrate::NativeSsrNode;

pub(crate) type RenderCallback = Rc<dyn Fn()>;

/// Native-testable Web DOM backend (mirrors exact DOM ops; mounts real `web_sys::Element`s on `wasm32`).
pub struct WebBackend {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) viewport: Viewport,
    pub(crate) frame_counter: u64,
    pub(crate) nodes: HashMap<NodeId, MirrorElement>,
    pub(crate) parent_of: HashMap<NodeId, NodeId>,
    pub(crate) stylesheet: StyleSheetCache,
    pub(crate) events: Rc<RefCell<Vec<NativeEvent>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
    pub(crate) node_event_meta: Rc<RefCell<HashMap<NodeId, NodeEventMeta>>>,
    pub(crate) hydrating: bool,
    pub(crate) hydrated_dom_tags: HashMap<NodeId, String>,
    pub(crate) hydrated_inline_nodes: HashSet<NodeId>,
    pub(crate) hydration_report: HydrationReport,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) native_ssr_nodes: HashMap<NodeId, NativeSsrNode>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) next_native_instance_id: u64,
    #[cfg(target_arch = "wasm32")]
    pub(crate) attr_scratch: String,
    #[cfg(target_arch = "wasm32")]
    pub(crate) dom_nodes: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) ime_guards: Rc<RefCell<HashMap<NodeId, ImeCompositionGuard>>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) root_delegator: Option<RootEventDelegator>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) window_keydown: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WebBackend {
    fn drop(&mut self) {
        if let Some(window_keydown) = self.window_keydown.take()
            && let Some(window) = web_sys::window()
        {
            let _ = window.remove_event_listener_with_callback(
                "keydown",
                window_keydown.as_ref().unchecked_ref(),
            );
        }
        // Dropping `root_delegator` detaches all 8 delegated container listeners cleanly.
        self.root_delegator = None;
        self.dom_nodes.clear();
    }
}

impl Default for WebBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebBackend {
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(ROOT_NODE_ID, MirrorElement::new(ComponentKind::Root));
        let mut meta = HashMap::new();
        meta.insert(
            ROOT_NODE_ID,
            NodeEventMeta {
                kind: ComponentKind::Root,
                mask: EventMask::NONE,
            },
        );
        Self {
            surface_id: Rc::new(Cell::new(1)),
            viewport: Viewport::default(),
            frame_counter: 0,
            nodes,
            parent_of: HashMap::new(),
            stylesheet: StyleSheetCache::new(),
            events: Rc::new(RefCell::new(Vec::new())),
            on_event: Rc::new(RefCell::new(None)),
            node_event_meta: Rc::new(RefCell::new(meta)),
            hydrating: false,
            hydrated_dom_tags: HashMap::new(),
            hydrated_inline_nodes: HashSet::new(),
            hydration_report: HydrationReport::default(),
            #[cfg(not(target_arch = "wasm32"))]
            native_ssr_nodes: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            next_native_instance_id: 1,
            #[cfg(target_arch = "wasm32")]
            attr_scratch: String::with_capacity(64),
            #[cfg(target_arch = "wasm32")]
            dom_nodes: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            ime_guards: Rc::new(RefCell::new(HashMap::new())),
            #[cfg(target_arch = "wasm32")]
            root_delegator: None,
            #[cfg(target_arch = "wasm32")]
            window_keydown: None,
        }
    }

    /// Returns the most recently configured [`Viewport`] for this backend.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Returns a read-only handle to the Option 5 Hybrid [`StyleSheetCache`].
    pub fn stylesheet(&self) -> &StyleSheetCache {
        &self.stylesheet
    }

    /// Mounts `ROOT_NODE_ID` into an existing browser DOM container (e.g. `#app`),
    /// adopts the static `.nve` base reset stylesheet, and attaches a single root event delegator
    /// plus a global `window` `keydown` listener.
    #[cfg(target_arch = "wasm32")]
    pub fn mount_into(container: web_sys::Element) -> Self {
        let mut backend = Self::new();
        container.set_text_content(None);
        backend.attr_scratch.clear();
        let _ = write!(&mut backend.attr_scratch, "{ROOT_NODE_ID}");
        let _ = container.set_attribute("data-nve-id", &backend.attr_scratch);
        if let Some(root) = backend.nodes.get_mut(&ROOT_NODE_ID) {
            root.layout.width = Dimension::Percent(100.0);
            root.layout.height = Dimension::Percent(100.0);
        }
        backend.dom_nodes.insert(ROOT_NODE_ID, container.clone());
        backend.flush_dirty_styles(&[ROOT_NODE_ID]);
        backend.attach_root_event_delegator(container);
        backend.attach_window_keydown_listener();
        backend
    }

    pub fn set_render_callback<F: Fn() + 'static>(&mut self, callback: F) {
        *self.on_event.borrow_mut() = Some(Rc::new(callback));
    }

    pub fn with_render_callback<F: Fn() + 'static>(mut self, callback: F) -> Self {
        self.set_render_callback(callback);
        self
    }

    pub fn event_queue(&self) -> Rc<RefCell<Vec<NativeEvent>>> {
        Rc::clone(&self.events)
    }

    pub fn push_event(&self, event: NativeEvent) {
        self.events.borrow_mut().push(event);
        let cb = self.on_event.borrow().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    pub fn node(&self, node_id: NodeId) -> Option<&MirrorElement> {
        self.nodes.get(&node_id)
    }

    pub fn iter_nodes(&self) -> impl Iterator<Item = (NodeId, &MirrorElement)> {
        self.nodes.iter().map(|(&id, el)| (id, el))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn event_mask_of(&self, node_id: NodeId) -> u64 {
        self.nodes
            .get(&node_id)
            .map(|n| n.event_mask)
            .unwrap_or(EventMask::NONE)
    }

    pub(crate) fn remove_node_event_meta(&mut self, node_id: NodeId) {
        self.node_event_meta.borrow_mut().remove(&node_id);
        #[cfg(target_arch = "wasm32")]
        self.ime_guards.borrow_mut().remove(&node_id);
    }

    fn queue_leaf_text_children(&self, parent_id: NodeId, dirty_styles: &mut Vec<NodeId>) {
        if let Some(parent) = self.nodes.get(&parent_id)
            && parent.kind == ComponentKind::Text
        {
            for &cid in &parent.children {
                if self
                    .nodes
                    .get(&cid)
                    .is_some_and(|c| c.kind == ComponentKind::Text && c.children.is_empty())
                {
                    dirty_styles.push(cid);
                }
            }
        }
    }
}

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
        let mut hydrated_seen: Vec<NodeId> = Vec::new();
        #[cfg(target_arch = "wasm32")]
        let mut dirty_canvases: Vec<NodeId> = Vec::new();

        for cmd in commands {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    if self.nodes.contains_key(&node_id) {
                        replaced_existing = true;
                    }
                    let mut mirror = MirrorElement::new(kind);
                    let tag = mirror.tag;
                    let adopting_ssr = self.hydrating && self.has_ssr_dom_node(node_id);
                    if adopting_ssr {
                        mirror.has_inline_style = self.hydrated_inline_nodes.contains(&node_id);
                        hydrated_seen.push(node_id);
                    }
                    self.nodes.insert(node_id, mirror);
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
                                && let Ok(el) = doc.create_element(tag)
                            {
                                self.attr_scratch.clear();
                                let _ = write!(&mut self.attr_scratch, "{node_id}");
                                let _ = el.set_attribute("data-nve-id", &self.attr_scratch);
                                self.dom_nodes.insert(node_id, el);
                            }
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            let new_id = self.next_native_instance_id;
                            self.next_native_instance_id += 1;
                            self.native_ssr_nodes.insert(
                                node_id,
                                NativeSsrNode {
                                    tag: tag.to_owned(),
                                    instance_id: new_id,
                                    has_inline_style: false,
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
                    self.nodes.remove(&node_id);
                    self.parent_of.remove(&node_id);
                    self.remove_node_event_meta(node_id);
                    for parent in self.nodes.values_mut() {
                        parent.children.retain(|&cid| cid != node_id);
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = self.dom_nodes.remove(&node_id) {
                        el.remove();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    self.native_ssr_nodes.remove(&node_id);
                    mutations += 1;
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    if let Some(old_children) = self.nodes.get(&parent_id).map(|p| p.children.clone())
                    {
                        for old_cid in old_children {
                            if self.parent_of.get(&old_cid) == Some(&parent_id) {
                                self.parent_of.remove(&old_cid);
                            }
                        }
                    }
                    for &cid in &children {
                        self.parent_of.insert(cid, parent_id);
                    }
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
                Command::SetRawProp {
                    node_id,
                    key,
                    value,
                } => {
                    if canvas::is_draw_list_prop(&key) {
                        let decoded_ops = canvas::decode_draw_list_prop(&value).unwrap_or_default();
                        let op_count = decoded_ops.len();
                        if let Some(el) = self.nodes.get_mut(&node_id) {
                            el.draw_op_count = op_count;
                            el.draw_ops = decoded_ops;
                            el.raw_props.insert(key, value);
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            if let Some(dom_el) = self.dom_nodes.get(&node_id) {
                                self.attr_scratch.clear();
                                let _ = write!(&mut self.attr_scratch, "{op_count}");
                                let _ = dom_el
                                    .set_attribute("data-nve-draw-ops", &self.attr_scratch);
                            }
                            dirty_canvases.push(node_id);
                        }
                    } else if is_raw_style_prop(&key) {
                        if let Some(el) = self.nodes.get_mut(&node_id) {
                            el.raw_style.apply_prop(&key, &value);
                            el.raw_props.insert(key, value);
                            el.record_style_mutation(current_frame);
                        }
                        dirty_styles.push(node_id);
                        self.queue_leaf_text_children(node_id, &mut dirty_styles);
                    } else {
                        let mut tag_changed = false;
                        if let Some(el) = self.nodes.get_mut(&node_id) {
                            el.raw_props.insert(key.clone(), value.clone());
                            if is_semantic_tag_prop(&key) {
                                let sem = resolve_semantic_tag(el.kind, &el.raw_props);
                                if el.tag != sem.tag || el.wrap_code != sem.wrap_code {
                                    el.tag = sem.tag;
                                    el.wrap_code = sem.wrap_code;
                                    tag_changed = true;
                                }
                            }
                        }
                        #[cfg(target_arch = "wasm32")]
                        if tag_changed && !self.hydrating {
                            self.upgrade_dom_element_tag(node_id);
                            dirty_styles.push(node_id);
                        } else {
                            self.sync_dom_raw_prop(node_id, &key, &value);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        let _ = tag_changed;
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
                Command::DispatchCommand { .. } | Command::Commit { .. } => {}
            }
        }

        if replaced_existing {
            self.prune_unreachable_nodes();
        }

        if self.hydrating {
            self.finalize_hydration(&hydrated_seen, &mut dirty_styles);
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
