//! Web DOM / WASM RenderBackend consuming the exact same `nve-abi` `CommandBuffer`.
//!
//! - On `wasm32`: creates and patches real browser DOM elements + CSS Flexbox directly via `web-sys`,
//!   attaching live `click` (`EventMask::PRESS`), `input` + `compositionstart`/`compositionend`
//!   IME guards (`EventMask::TEXT_CHANGE`), `scroll` (`ScrollView` / `EventMask::SCROLL`),
//!   `focus`/`blur` (`TextInput` / `EventMask::FOCUS|BLUR`), `TextInput` `Enter` (`submit`),
//!   and global `window` `keydown` (zero-JSON `NK1` binary payload), plus ARIA accessibility
//!   attributes, `<img>` `object-fit` mapping, and live `<canvas>` `NVE_DRAW1` vector rendering.
//! - On native targets: runs `WebDomMirror` so CSS translation, ARIA/HTML SSR attributes,
//!   IME composition guards, event masks, `<img>` fit mapping, `<canvas>` `NVE_DRAW1` draw-op
//!   decoding, incremental patches, and DOM mutations are unit-tested headlessly in milliseconds.

pub mod canvas;
mod css;
mod dom;
mod events;
mod hot_patch;
#[cfg(test)]
mod tests;

pub use css::{
    component_to_html_tag, is_image_fit_prop, layout_to_css, layout_to_css_into,
    raw_prop_to_object_fit, style_to_css, style_to_css_into, MirrorElement,
};
pub use dom::{is_truthy_raw_prop, map_a11y_or_input_attr, MappedHtmlAttr};
pub use events::{encode_keydown_payload, ImeCompositionGuard};

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, EventMask, NativeEvent, NodeId, RawPropValue,
    SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::events::{now_us, NodeListeners};

pub(crate) type RenderCallback = Rc<dyn Fn()>;

/// Native-testable Web DOM backend (mirrors exact DOM ops; mounts real `web_sys::Element`s on `wasm32`).
pub struct WebBackend {
    pub(crate) surface_id: Rc<Cell<SurfaceId>>,
    pub(crate) viewport: Viewport,
    pub(crate) nodes: HashMap<NodeId, MirrorElement>,
    pub(crate) events: Rc<RefCell<Vec<NativeEvent>>>,
    pub(crate) on_event: Rc<RefCell<Option<RenderCallback>>>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) css_scratch: String,
    #[cfg(target_arch = "wasm32")]
    pub(crate) dom_nodes: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) listeners: HashMap<NodeId, NodeListeners>,
    #[cfg(target_arch = "wasm32")]
    pub(crate) window_keydown: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for WebBackend {
    fn drop(&mut self) {
        if let Some(window_keydown) = self.window_keydown.take() {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "keydown",
                    window_keydown.as_ref().unchecked_ref(),
                );
            }
        }
        let dom_nodes = std::mem::take(&mut self.dom_nodes);
        for (node_id, dom_el) in dom_nodes {
            self.detach_listeners(node_id, &dom_el);
        }
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
        Self {
            surface_id: Rc::new(Cell::new(1)),
            viewport: Viewport::default(),
            nodes,
            events: Rc::new(RefCell::new(Vec::new())),
            on_event: Rc::new(RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            css_scratch: String::with_capacity(512),
            #[cfg(target_arch = "wasm32")]
            dom_nodes: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            listeners: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            window_keydown: None,
        }
    }

    /// Returns the most recently configured [`Viewport`] for this backend.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Mounts `ROOT_NODE_ID` into an existing browser DOM container (e.g. `#app`),
    /// and attaches a global `window` `keydown` listener for `Cmd+K`/`Ctrl+K`,
    /// `ArrowUp`, `ArrowDown`, `Enter`, and `Escape`.
    #[cfg(target_arch = "wasm32")]
    pub fn mount_into(container: web_sys::Element) -> Self {
        let mut backend = Self::new();
        container.set_text_content(None);
        backend.css_scratch.clear();
        let _ = write!(&mut backend.css_scratch, "{ROOT_NODE_ID}");
        let _ = container.set_attribute("data-nve-id", &backend.css_scratch);
        let root_css =
            "display:flex;flex-direction:column;box-sizing:border-box;width:100%;height:100%;";
        if let Some(root) = backend.nodes.get_mut(&ROOT_NODE_ID) {
            root.layout_css.clear();
            root.layout_css.push_str(root_css);
        }
        if let Some(html_el) = container.dyn_ref::<web_sys::HtmlElement>() {
            html_el.style().set_css_text(root_css);
        }
        backend.dom_nodes.insert(ROOT_NODE_ID, container);
        backend.attach_window_keydown_listener();
        backend
    }

    /// Registers a callback invoked immediately whenever an interactive DOM event is queued.
    pub fn set_render_callback<F: Fn() + 'static>(&mut self, callback: F) {
        *self.on_event.borrow_mut() = Some(Rc::new(callback));
    }

    /// Builder-style variant of `set_render_callback`.
    pub fn with_render_callback<F: Fn() + 'static>(mut self, callback: F) -> Self {
        self.set_render_callback(callback);
        self
    }

    /// Returns a handle to the shared event queue (also used by DOM closures on `wasm32`).
    pub fn event_queue(&self) -> Rc<RefCell<Vec<NativeEvent>>> {
        Rc::clone(&self.events)
    }

    /// Pushes a `NativeEvent` into the shared queue and triggers the render callback if registered.
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

    /// Iterates over every live `(NodeId, &MirrorElement)` currently mounted in the DOM mirror.
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

        let t_mount = FrameTimer::start();
        let mut mutations = 0u32;
        let mut replaced_existing = false;
        #[cfg(target_arch = "wasm32")]
        let mut dirty_canvases: Vec<NodeId> = Vec::new();

        for cmd in commands {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    if self.nodes.contains_key(&node_id) {
                        replaced_existing = true;
                    }
                    let mirror = MirrorElement::new(kind);
                    let tag = mirror.tag;
                    self.nodes.insert(node_id, mirror);
                    #[cfg(target_arch = "wasm32")]
                    if let Some(old_el) = self.dom_nodes.remove(&node_id) {
                        self.detach_listeners(node_id, &old_el);
                        old_el.remove();
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        if let Ok(el) = doc.create_element(tag) {
                            self.css_scratch.clear();
                            let _ = write!(&mut self.css_scratch, "{node_id}");
                            let _ = el.set_attribute("data-nve-id", &self.css_scratch);
                            self.dom_nodes.insert(node_id, el);
                            self.sync_dom_style(node_id);
                            if matches!(kind, ComponentKind::ScrollView | ComponentKind::TextInput)
                            {
                                self.sync_event_listeners(node_id, EventMask::NONE);
                            }
                            if kind == ComponentKind::SkiaCanvas {
                                dirty_canvases.push(node_id);
                            }
                        }
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    let _ = tag;
                    mutations += 1;
                }
                Command::RemoveNode { node_id } => {
                    self.nodes.remove(&node_id);
                    for parent in self.nodes.values_mut() {
                        parent.children.retain(|&cid| cid != node_id);
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = self.dom_nodes.remove(&node_id) {
                        self.detach_listeners(node_id, &el);
                        el.remove();
                    }
                    mutations += 1;
                }
                Command::SetChildren {
                    parent_id,
                    children,
                } => {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(parent_el) = self.dom_nodes.get(&parent_id) {
                        parent_el.set_text_content(None);
                        for cid in &children {
                            if let Some(child_el) = self.dom_nodes.get(cid) {
                                let _ = parent_el.append_child(child_el);
                            }
                        }
                    }
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
                        el.layout_css.clear();
                        layout_to_css_into(&layout, &mut el.layout_css);
                        #[cfg(target_arch = "wasm32")]
                        {
                            is_canvas = el.kind == ComponentKind::SkiaCanvas;
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        self.sync_dom_style(node_id);
                        if is_canvas {
                            dirty_canvases.push(node_id);
                        }
                    }
                    mutations += 1;
                }
                Command::SetStyle { node_id, style } => {
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.style = style;
                        el.style_css.clear();
                        style_to_css_into(&style, &mut el.style_css);
                    }
                    #[cfg(target_arch = "wasm32")]
                    self.sync_dom_style(node_id);
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
                                self.css_scratch.clear();
                                let _ = write!(&mut self.css_scratch, "{op_count}");
                                let _ =
                                    dom_el.set_attribute("data-nve-draw-ops", &self.css_scratch);
                            }
                            dirty_canvases.push(node_id);
                        }
                    } else if is_image_fit_prop(&key) {
                        let fit = if matches!(value, RawPropValue::Null) {
                            None
                        } else {
                            raw_prop_to_object_fit(&value)
                        };
                        if let Some(el) = self.nodes.get_mut(&node_id) {
                            el.object_fit = fit;
                            el.raw_props.insert(key, value);
                        }
                        #[cfg(target_arch = "wasm32")]
                        self.sync_dom_style(node_id);
                    } else {
                        #[cfg(target_arch = "wasm32")]
                        self.sync_dom_raw_prop(node_id, &key, &value);
                        if let Some(el) = self.nodes.get_mut(&node_id) {
                            el.raw_props.insert(key, value);
                        }
                    }
                    mutations += 1;
                }
                Command::SetEventMask { node_id, mask } => {
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.event_mask = mask;
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        self.sync_event_listeners(node_id, mask);
                        self.sync_dom_style(node_id);
                    }
                    mutations += 1;
                }
                Command::DispatchCommand { .. } | Command::Commit { .. } => {}
            }
        }

        if replaced_existing {
            self.prune_unreachable_nodes();
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
