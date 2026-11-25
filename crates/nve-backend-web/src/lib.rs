//! Web DOM / WASM RenderBackend consuming the exact same `nve-abi` `CommandBuffer`.
//!
//! - On `wasm32`: creates and patches real browser DOM elements + CSS Flexbox directly via `web-sys`,
//!   attaching live `click` (`EventMask::PRESS`) and `input` (`EventMask::TEXT_CHANGE`) listeners.
//! - On native targets: runs `WebDomMirror` so CSS translation, event masks, incremental patches,
//!   and DOM mutations are unit-tested headlessly in `cargo test` in milliseconds.

use core::fmt::Write as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nve_abi::{
    ColorRgba, Command, CommandDecoder, ComponentKind, Dimension, EventMask, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, NativeEvent, NodeId, RawPropValue, StyleProps,
    SurfaceId, ROOT_NODE_ID,
};
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, RenderBackend, Viewport};

#[cfg(target_arch = "wasm32")]
use nve_abi::{EventPayload, NativeEventKind};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::closure::Closure;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

type RenderCallback = Rc<dyn Fn()>;

pub fn component_to_html_tag(kind: ComponentKind) -> &'static str {
    match kind {
        ComponentKind::Root | ComponentKind::View | ComponentKind::ScrollView => "div",
        ComponentKind::Text => "span",
        ComponentKind::Image => "img",
        ComponentKind::TextInput => "input",
        ComponentKind::Pressable => "button",
        ComponentKind::SkiaCanvas => "canvas",
        ComponentKind::Custom => "div",
    }
}

#[inline]
fn write_dim(d: Dimension, out: &mut String) {
    match d {
        Dimension::Auto => out.push_str("auto"),
        Dimension::Points(v) => {
            let _ = write!(out, "{v}px");
        }
        Dimension::Percent(v) => {
            let _ = write!(out, "{v}%");
        }
    }
}

#[inline]
fn write_rgba(c: ColorRgba, out: &mut String) {
    let _ = write!(
        out,
        "rgba({},{},{},{:.3})",
        c.r,
        c.g,
        c.b,
        c.a as f32 / 255.0
    );
}

/// Appends Flexbox CSS declarations for `l` directly into `out` with zero intermediate allocations.
pub fn layout_to_css_into(l: &LayoutProps, out: &mut String) {
    let dir = match l.flex_direction {
        FlexDirection::Column => "column",
        FlexDirection::Row => "row",
        FlexDirection::ColumnReverse => "column-reverse",
        FlexDirection::RowReverse => "row-reverse",
    };
    let justify = match l.justify_content {
        FlexJustify::FlexStart => "flex-start",
        FlexJustify::Center => "center",
        FlexJustify::FlexEnd => "flex-end",
        FlexJustify::SpaceBetween => "space-between",
        FlexJustify::SpaceAround => "space-around",
        FlexJustify::SpaceEvenly => "space-evenly",
    };
    let align = match l.align_items {
        FlexAlign::Stretch => "stretch",
        FlexAlign::FlexStart => "flex-start",
        FlexAlign::Center => "center",
        FlexAlign::FlexEnd => "flex-end",
        FlexAlign::Baseline => "baseline",
    };

    out.push_str("display:flex;box-sizing:border-box;flex-direction:");
    out.push_str(dir);
    out.push_str(";justify-content:");
    out.push_str(justify);
    out.push_str(";align-items:");
    out.push_str(align);
    let _ = write!(
        out,
        ";flex-grow:{};flex-shrink:{};gap:{}px;width:",
        l.flex_grow, l.flex_shrink, l.gap
    );
    write_dim(l.width, out);
    out.push_str(";height:");
    write_dim(l.height, out);
    let _ = write!(
        out,
        ";padding:{}px {}px {}px {}px;margin:{}px {}px {}px {}px;",
        l.padding.top,
        l.padding.right,
        l.padding.bottom,
        l.padding.left,
        l.margin.top,
        l.margin.right,
        l.margin.bottom,
        l.margin.left,
    );

    if l.min_width != Dimension::Auto {
        out.push_str("min-width:");
        write_dim(l.min_width, out);
        out.push(';');
    }
    if l.min_height != Dimension::Auto {
        out.push_str("min-height:");
        write_dim(l.min_height, out);
        out.push(';');
    }
    if l.max_width != Dimension::Auto {
        out.push_str("max-width:");
        write_dim(l.max_width, out);
        out.push(';');
    }
    if l.max_height != Dimension::Auto {
        out.push_str("max-height:");
        write_dim(l.max_height, out);
        out.push(';');
    }
}

pub fn layout_to_css(l: &LayoutProps) -> String {
    let mut out = String::with_capacity(256);
    layout_to_css_into(l, &mut out);
    out
}

/// Appends visual style CSS declarations for `s` directly into `out` with zero intermediate allocations.
pub fn style_to_css_into(s: &StyleProps, out: &mut String) {
    out.push_str("background-color:");
    write_rgba(s.background, out);
    out.push_str(";color:");
    write_rgba(s.text_color, out);
    let _ = write!(out, ";border:{}px solid ", s.border_width);
    write_rgba(s.border_color, out);
    let _ = write!(
        out,
        ";border-radius:{}px;opacity:{};font-size:{}px;z-index:{};",
        s.border_radius, s.opacity, s.font_size, s.z_index,
    );
}

pub fn style_to_css(s: &StyleProps) -> String {
    let mut out = String::with_capacity(192);
    style_to_css_into(s, &mut out);
    out
}

fn kind_default_css(kind: ComponentKind, event_mask: u64) -> &'static str {
    match kind {
        ComponentKind::ScrollView => "overflow:auto;",
        ComponentKind::Pressable => "cursor:pointer;appearance:none;font:inherit;",
        ComponentKind::TextInput => "font:inherit;outline:none;",
        _ if (event_mask & EventMask::PRESS) != 0 => "cursor:pointer;",
        _ => "",
    }
}

#[derive(Debug, Clone)]
pub struct MirrorElement {
    pub kind: ComponentKind,
    pub tag: &'static str,
    pub layout: LayoutProps,
    pub style: StyleProps,
    pub layout_css: String,
    pub style_css: String,
    pub patch_css: String,
    pub text: Option<String>,
    pub children: Vec<NodeId>,
    pub event_mask: u64,
    pub raw_props: HashMap<String, RawPropValue>,
}

impl MirrorElement {
    fn new(kind: ComponentKind) -> Self {
        Self {
            kind,
            tag: component_to_html_tag(kind),
            layout: LayoutProps::default(),
            style: StyleProps::default(),
            layout_css: String::new(),
            style_css: String::new(),
            patch_css: String::new(),
            text: None,
            children: Vec::new(),
            event_mask: EventMask::NONE,
            raw_props: HashMap::new(),
        }
    }

    /// Appends full inline `style` text into `out` without allocating an intermediate `String`.
    #[inline]
    pub fn combined_css_into(&self, out: &mut String) {
        out.push_str(kind_default_css(self.kind, self.event_mask));
        out.push_str(&self.layout_css);
        out.push_str(&self.style_css);
        out.push_str(&self.patch_css);
    }

    /// Full inline `style` text combining kind defaults, typed layout, typed style, and Tier-1 patches.
    pub fn combined_css(&self) -> String {
        let base = kind_default_css(self.kind, self.event_mask);
        let mut out = String::with_capacity(
            base.len() + self.layout_css.len() + self.style_css.len() + self.patch_css.len(),
        );
        self.combined_css_into(&mut out);
        out
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct NodeListeners {
    click: Option<Closure<dyn FnMut(web_sys::MouseEvent)>>,
    input: Option<Closure<dyn FnMut(web_sys::Event)>>,
}

/// Native-testable Web DOM backend (mirrors exact DOM ops; mounts real `web_sys::Element`s on `wasm32`).
pub struct WebBackend {
    surface_id: Rc<Cell<SurfaceId>>,
    nodes: HashMap<NodeId, MirrorElement>,
    events: Rc<RefCell<Vec<NativeEvent>>>,
    on_event: Rc<RefCell<Option<RenderCallback>>>,
    #[cfg(target_arch = "wasm32")]
    css_scratch: String,
    #[cfg(target_arch = "wasm32")]
    dom_nodes: HashMap<NodeId, web_sys::Element>,
    #[cfg(target_arch = "wasm32")]
    listeners: HashMap<NodeId, NodeListeners>,
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
            nodes,
            events: Rc::new(RefCell::new(Vec::new())),
            on_event: Rc::new(RefCell::new(None)),
            #[cfg(target_arch = "wasm32")]
            css_scratch: String::with_capacity(512),
            #[cfg(target_arch = "wasm32")]
            dom_nodes: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            listeners: HashMap::new(),
        }
    }

    /// Mounts `ROOT_NODE_ID` into an existing browser DOM container (e.g. `#app`).
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

    /// Applies a Tier-1 `HotPatchMsg::TemplatePatch` directly to targeted live DOM + mirror nodes (<10ms).
    ///
    /// Respects `target_test_id`, `node_path`, and exact `text_replacements` so live WebSocket
    /// patches update ONLY the matching DOM node(s) without spraying unrelated elements.
    pub fn apply_template_patch(&mut self, patch: &nve_engine::HotPatchMsg) -> u32 {
        let nve_engine::HotPatchMsg::TemplatePatch {
            template_id,
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
            ..
        } = patch
        else {
            return 0;
        };

        let mut sorted_ids: Vec<NodeId> = self.nodes.keys().copied().collect();
        sorted_ids.sort_unstable();
        let mut mutations = 0u32;

        let matches_test_id = |el: &MirrorElement, tid: &str| -> bool {
            ["testID", "data-testid", "test_id"].iter().any(|k| {
                matches!(el.raw_props.get(*k), Some(RawPropValue::String(v)) if v == tid)
            })
        };

        let find_by_test_id = |nodes: &HashMap<NodeId, MirrorElement>, tid: &str| -> Vec<NodeId> {
            let mut ids: Vec<NodeId> = nodes
                .iter()
                .filter_map(|(&id, el)| {
                    (id != ROOT_NODE_ID && matches_test_id(el, tid)).then_some(id)
                })
                .collect();
            ids.sort_unstable();
            ids
        };

        let collect_subtree =
            |nodes: &HashMap<NodeId, MirrorElement>, roots: &[NodeId]| -> Vec<NodeId> {
                let mut out = Vec::new();
                let mut stack = roots.to_vec();
                while let Some(nid) = stack.pop() {
                    out.push(nid);
                    if let Some(el) = nodes.get(&nid) {
                        for &cid in &el.children {
                            stack.push(cid);
                        }
                    }
                }
                out.sort_unstable();
                out.dedup();
                out
            };

        let walk_path = |nodes: &HashMap<NodeId, MirrorElement>, path: &[usize]| -> Option<NodeId> {
            let mut cur = ROOT_NODE_ID;
            for &idx in path {
                cur = *nodes.get(&cur)?.children.get(idx)?;
            }
            (cur != ROOT_NODE_ID).then_some(cur)
        };

        let mut matched_text_nodes: Vec<NodeId> = Vec::new();

        // 1. Text updates: prefer exact `text_replacements` over positional `text_updates`.
        if !text_replacements.is_empty() {
            let scoped_ids = target_test_id
                .as_deref()
                .map(|tid| {
                    let roots = find_by_test_id(&self.nodes, tid);
                    collect_subtree(&self.nodes, &roots)
                })
                .unwrap_or_default();

            for (old_text, new_text) in text_replacements {
                let mut targets: Vec<NodeId> = Vec::new();
                if !scoped_ids.is_empty() {
                    for &nid in &scoped_ids {
                        if self
                            .nodes
                            .get(&nid)
                            .is_some_and(|el| el.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }
                if targets.is_empty() {
                    for &nid in &sorted_ids {
                        if nid != ROOT_NODE_ID
                            && self
                                .nodes
                                .get(&nid)
                                .is_some_and(|el| el.text.as_deref() == Some(old_text.as_str()))
                        {
                            targets.push(nid);
                        }
                    }
                }
                for nid in targets {
                    self.apply_text_mutation(nid, new_text);
                    matched_text_nodes.push(nid);
                    mutations += 1;
                }
            }
        } else if !text_updates.is_empty() {
            let is_targeted = target_test_id.is_some() || !node_path.is_empty();
            let candidate_pool: Vec<NodeId> = if let Some(tid) = target_test_id.as_deref() {
                let roots = find_by_test_id(&self.nodes, tid);
                collect_subtree(&self.nodes, &roots)
            } else if !node_path.is_empty() {
                walk_path(&self.nodes, node_path)
                    .map(|nid| collect_subtree(&self.nodes, &[nid]))
                    .unwrap_or_default()
            } else {
                sorted_ids.clone()
            };

            let text_node_ids: Vec<NodeId> = candidate_pool
                .iter()
                .copied()
                .filter(|id| {
                    self.nodes
                        .get(id)
                        .is_some_and(|n| n.kind == ComponentKind::Text && n.text.is_some())
                })
                .collect();

            for (idx, text) in text_updates {
                let target = text_node_ids.get(*idx).copied().or_else(|| {
                    if is_targeted {
                        None
                    } else {
                        sorted_ids.get(*idx).copied()
                    }
                });
                if let Some(nid) = target {
                    self.apply_text_mutation(nid, text);
                    mutations += 1;
                }
            }
        }

        // 2. Attribute / CSS style patches: target exact node(s), never spray unrelated nodes.
        if !attr_updates.is_empty() {
            let mut css_fragment = String::with_capacity(attr_updates.len() * 32);
            for (k, v) in attr_updates {
                if k == "style" {
                    css_fragment.push_str(v.trim());
                    if !css_fragment.ends_with(';') {
                        css_fragment.push(';');
                    }
                } else {
                    let _ = write!(&mut css_fragment, "{k}:{v};");
                }
            }

            let attr_targets: Vec<NodeId> = if let Some(tid) = target_test_id.as_deref() {
                find_by_test_id(&self.nodes, tid)
            } else if !node_path.is_empty() {
                walk_path(&self.nodes, node_path).into_iter().collect()
            } else if !text_replacements.is_empty() {
                matched_text_nodes
            } else {
                let by_tpl = if !template_id.is_empty() {
                    find_by_test_id(&self.nodes, template_id)
                } else {
                    Vec::new()
                };
                if !by_tpl.is_empty() {
                    by_tpl
                } else {
                    sorted_ids
                        .iter()
                        .copied()
                        .filter(|&id| id != ROOT_NODE_ID)
                        .collect()
                }
            };

            for nid in attr_targets {
                if let Some(el) = self.nodes.get_mut(&nid) {
                    el.patch_css.push_str(&css_fragment);
                }
                #[cfg(target_arch = "wasm32")]
                self.sync_dom_style(nid);
                mutations += 1;
            }
        }

        mutations
    }

    /// Serializes the DOM mirror subtree rooted at `id` directly into `out` with zero intermediate allocations.
    pub fn fmt_html_into(&self, id: NodeId, out: &mut String) {
        let Some(el) = self.nodes.get(&id) else {
            return;
        };
        out.push('<');
        out.push_str(el.tag);
        out.push_str(" data-nve-id=\"");
        let _ = write!(out, "{id}");
        out.push_str("\" style=\"");
        el.combined_css_into(out);
        out.push_str("\">");
        if let Some(t) = &el.text {
            out.push_str(t);
        }
        for &child in &el.children {
            self.fmt_html_into(child, out);
        }
        out.push_str("</");
        out.push_str(el.tag);
        out.push('>');
    }

    pub fn render_html(&self) -> String {
        let mut out = String::with_capacity(self.nodes.len().saturating_mul(384).max(64));
        self.fmt_html_into(ROOT_NODE_ID, &mut out);
        out
    }

    fn apply_text_mutation(&mut self, node_id: NodeId, text: &str) {
        #[cfg(target_arch = "wasm32")]
        let is_input = self
            .nodes
            .get(&node_id)
            .is_some_and(|n| n.kind == ComponentKind::TextInput);

        if let Some(el) = self.nodes.get_mut(&node_id) {
            match &mut el.text {
                Some(existing) => {
                    existing.clear();
                    existing.push_str(text);
                }
                None => el.text = Some(text.to_owned()),
            }
        }

        #[cfg(target_arch = "wasm32")]
        if let Some(dom_el) = self.dom_nodes.get(&node_id) {
            if is_input {
                if let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>() {
                    if inp.value() != text {
                        inp.set_value(text);
                    }
                }
            } else {
                dom_el.set_text_content(Some(text));
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_dom_style(&mut self, node_id: NodeId) {
        let Some(mirror) = self.nodes.get(&node_id) else {
            return;
        };
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        if let Some(html_el) = dom_el.dyn_ref::<web_sys::HtmlElement>() {
            self.css_scratch.clear();
            mirror.combined_css_into(&mut self.css_scratch);
            html_el.style().set_css_text(&self.css_scratch);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_dom_raw_prop(&mut self, node_id: NodeId, key: &str, value: &RawPropValue) {
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        match value {
            RawPropValue::Null => {
                let _ = dom_el.remove_attribute(key);
            }
            RawPropValue::Bool(true) => {
                let _ = dom_el.set_attribute(key, "");
            }
            RawPropValue::Bool(false) => {
                let _ = dom_el.remove_attribute(key);
            }
            RawPropValue::String(s) => {
                if key == "value" {
                    if let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>() {
                        if inp.value() != *s {
                            inp.set_value(s);
                        }
                        return;
                    }
                }
                let _ = dom_el.set_attribute(key, s);
            }
            RawPropValue::I64(n) => {
                self.css_scratch.clear();
                let _ = write!(&mut self.css_scratch, "{n}");
                let _ = dom_el.set_attribute(key, &self.css_scratch);
            }
            RawPropValue::F64(n) => {
                self.css_scratch.clear();
                let _ = write!(&mut self.css_scratch, "{n}");
                let _ = dom_el.set_attribute(key, &self.css_scratch);
            }
            RawPropValue::Bytes(_) => {}
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn sync_event_listeners(&mut self, node_id: NodeId, mask: u64) {
        let Some(dom_el) = self.dom_nodes.get(&node_id).cloned() else {
            return;
        };
        let want_click = (mask & EventMask::PRESS) != 0;
        let want_input = (mask & EventMask::TEXT_CHANGE) != 0;

        let entry = self.listeners.entry(node_id).or_default();

        if want_click && entry.click.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(
                move |ev: web_sys::MouseEvent| {
                    events.borrow_mut().push(NativeEvent {
                        surface_id: surface_id.get(),
                        node_id,
                        timestamp_us: now_us(),
                        kind: NativeEventKind::Press,
                        payload: EventPayload::Point {
                            x: ev.client_x() as f32,
                            y: ev.client_y() as f32,
                        },
                    });
                    let cb = on_event.borrow().clone();
                    if let Some(cb) = cb {
                        cb();
                    }
                },
            );
            let _ = dom_el
                .add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
            entry.click = Some(closure);
        } else if !want_click {
            if let Some(old) = entry.click.take() {
                let _ = dom_el
                    .remove_event_listener_with_callback("click", old.as_ref().unchecked_ref());
            }
        }

        if want_input && entry.input.is_none() {
            let events = Rc::clone(&self.events);
            let on_event = Rc::clone(&self.on_event);
            let surface_id = Rc::clone(&self.surface_id);
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
                let text = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|inp| inp.value())
                    .unwrap_or_default();
                events.borrow_mut().push(NativeEvent {
                    surface_id: surface_id.get(),
                    node_id,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::TextChange,
                    payload: EventPayload::Text(text),
                });
                let cb = on_event.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
            });
            let _ = dom_el
                .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref());
            entry.input = Some(closure);
        } else if !want_input {
            if let Some(old) = entry.input.take() {
                let _ = dom_el
                    .remove_event_listener_with_callback("input", old.as_ref().unchecked_ref());
            }
        }

        if entry.click.is_none() && entry.input.is_none() {
            self.listeners.remove(&node_id);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn detach_listeners(&mut self, node_id: NodeId, dom_el: &web_sys::Element) {
        if let Some(listeners) = self.listeners.remove(&node_id) {
            if let Some(click) = listeners.click {
                let _ = dom_el
                    .remove_event_listener_with_callback("click", click.as_ref().unchecked_ref());
            }
            if let Some(input) = listeners.input {
                let _ = dom_el
                    .remove_event_listener_with_callback("input", input.as_ref().unchecked_ref());
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

#[cfg(target_arch = "wasm32")]
fn now_us() -> u64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| (p.now() * 1000.0) as u64)
        .unwrap_or(0)
}

impl RenderBackend for WebBackend {
    fn name(&self) -> &'static str {
        if cfg!(target_arch = "wasm32") {
            "web-dom-wasm"
        } else {
            "web-dom-mirror"
        }
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, _viewport: Viewport) {
        self.surface_id.set(surface_id);
    }

    fn submit_frame(&mut self, command_bytes: &[u8]) -> Result<BackendApplyStats, BackendError> {
        let t_dec = FrameTimer::start();
        let (header, commands) = CommandDecoder::new(command_bytes)?.decode_all()?;
        let decode_us = t_dec.elapsed_us();
        self.surface_id.set(header.surface_id);

        let t_mount = FrameTimer::start();
        let mut mutations = 0u32;

        for cmd in commands {
            match cmd {
                Command::CreateNode { node_id, kind, .. } => {
                    let mirror = MirrorElement::new(kind);
                    let tag = mirror.tag;
                    self.nodes.insert(node_id, mirror);
                    #[cfg(target_arch = "wasm32")]
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        if let Ok(el) = doc.create_element(tag) {
                            self.css_scratch.clear();
                            let _ = write!(&mut self.css_scratch, "{node_id}");
                            let _ = el.set_attribute("data-nve-id", &self.css_scratch);
                            self.dom_nodes.insert(node_id, el);
                            self.sync_dom_style(node_id);
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
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.layout = layout;
                        el.layout_css.clear();
                        layout_to_css_into(&layout, &mut el.layout_css);
                    }
                    #[cfg(target_arch = "wasm32")]
                    self.sync_dom_style(node_id);
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
                    #[cfg(target_arch = "wasm32")]
                    self.sync_dom_raw_prop(node_id, &key, &value);
                    if let Some(el) = self.nodes.get_mut(&node_id) {
                        el.raw_props.insert(key, value);
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

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{CommandBufferBuilder, EdgeInsets, EventPayload, NativeEventKind};
    use std::cell::Cell;

    #[test]
    fn renders_flexbox_html_from_command_buffer() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Pressable, None);
        b.set_text(2, "Click");
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();
        let html = web.render_html();
        assert!(html.contains("<button data-nve-id=\"2\""));
        assert!(html.contains("Click</button>"));

        // Reusing a scratch String via `fmt_html_into` matches `render_html` identically.
        let mut scratch = String::with_capacity(256);
        web.fmt_html_into(ROOT_NODE_ID, &mut scratch);
        assert_eq!(scratch, html);
    }

    #[test]
    fn css_into_writers_match_wrappers_and_reuse_buffers() {
        let layout = LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Points(64.0),
            min_width: Dimension::Points(120.0),
            min_height: Dimension::Points(32.0),
            max_width: Dimension::Points(480.0),
            max_height: Dimension::Percent(90.0),
            flex_direction: FlexDirection::RowReverse,
            justify_content: FlexJustify::SpaceBetween,
            align_items: FlexAlign::Baseline,
            flex_grow: 1.0,
            flex_shrink: 0.0,
            gap: 8.0,
            padding: EdgeInsets::symmetric(10.0, 14.0),
            margin: EdgeInsets::all(4.0),
        };
        let style = StyleProps {
            background: ColorRgba::rgba(17, 24, 39, 128),
            text_color: ColorRgba::WHITE,
            border_color: ColorRgba::rgb(56, 189, 248),
            border_width: 2.0,
            border_radius: 12.0,
            opacity: 0.9,
            font_size: 18.0,
            z_index: 5,
        };

        let mut buf = String::with_capacity(512);
        layout_to_css_into(&layout, &mut buf);
        assert_eq!(buf, layout_to_css(&layout));
        assert!(buf.contains("min-width:120px;"));
        assert!(buf.contains("min-height:32px;"));
        assert!(buf.contains("max-width:480px;"));
        assert!(buf.contains("max-height:90%;"));

        let cap_before = buf.capacity();
        buf.clear();
        style_to_css_into(&style, &mut buf);
        assert_eq!(buf, style_to_css(&style));
        assert_eq!(buf.capacity(), cap_before);
        assert!(buf.contains("background-color:rgba(17,24,39,0.502);"));
        assert!(buf.contains("border:2px solid rgba(56,189,248,1.000);"));
        assert!(buf.contains("z-index:5;"));
    }

    #[test]
    fn updates_style_and_layout_incrementally() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Points(64.0),
                min_width: Dimension::Points(120.0),
                max_width: Dimension::Points(480.0),
                flex_direction: FlexDirection::Row,
                justify_content: FlexJustify::Center,
                align_items: FlexAlign::Center,
                gap: 12.0,
                padding: EdgeInsets::all(16.0),
                ..Default::default()
            },
        );
        b.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(37, 99, 235),
                text_color: ColorRgba::WHITE,
                border_radius: 12.0,
                font_size: 18.0,
                ..Default::default()
            },
        );
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();

        let css1 = web.node(2).unwrap().combined_css();
        assert!(css1.contains("flex-direction:row;"));
        assert!(css1.contains("min-width:120px;"));
        assert!(css1.contains("max-width:480px;"));
        assert!(css1.contains("background-color:rgba(37,99,235,1.000);"));
        assert!(css1.contains("border-radius:12px;"));

        // Incremental style-only patch must preserve previously set layout CSS.
        let mut b2 = CommandBufferBuilder::new(1, 2);
        b2.set_style(
            2,
            &StyleProps {
                background: ColorRgba::rgb(15, 23, 42),
                text_color: ColorRgba::rgb(56, 189, 248),
                border_radius: 20.0,
                opacity: 0.75,
                font_size: 24.0,
                ..Default::default()
            },
        );
        b2.commit(true);
        let stats = web.submit_frame(b2.finish()).unwrap();
        assert_eq!(stats.mutation_count, 1);

        let css2 = web.node(2).unwrap().combined_css();
        assert!(css2.contains("flex-direction:row;"));
        assert!(css2.contains("padding:16px 16px 16px 16px;"));
        assert!(css2.contains("background-color:rgba(15,23,42,1.000);"));
        assert!(css2.contains("border-radius:20px;"));
        assert!(css2.contains("opacity:0.75;"));
    }

    #[test]
    fn removes_nodes_and_detaches_from_parent() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Text, None);
        b.set_text(2, "Keep");
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "Drop");
        b.set_children(ROOT_NODE_ID, &[2, 3]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();
        assert_eq!(web.node_count(), 3);
        assert!(web.render_html().contains("Drop</span>"));

        let mut b2 = CommandBufferBuilder::new(1, 2);
        b2.remove_node(3);
        b2.commit(true);
        web.submit_frame(b2.finish()).unwrap();

        assert_eq!(web.node_count(), 2);
        assert!(web.node(3).is_none());
        assert_eq!(web.node(ROOT_NODE_ID).unwrap().children, vec![2]);
        let html = web.render_html();
        assert!(html.contains("Keep</span>"));
        assert!(!html.contains("Drop"));
    }

    #[test]
    fn tracks_event_masks_and_fires_render_callback() {
        let callback_hits = Rc::new(Cell::new(0u32));
        let hits_clone = Rc::clone(&callback_hits);

        let mut web = WebBackend::new().with_render_callback(move || {
            hits_clone.set(hits_clone.get() + 1);
        });

        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::Pressable, None);
        b.set_event_mask(2, EventMask::PRESS);
        b.create_node(3, ComponentKind::TextInput, None);
        b.set_event_mask(3, EventMask::TEXT_CHANGE | EventMask::FOCUS);
        b.set_children(ROOT_NODE_ID, &[2, 3]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();

        assert_eq!(web.event_mask_of(2), EventMask::PRESS);
        assert_eq!(
            web.event_mask_of(3),
            EventMask::TEXT_CHANGE | EventMask::FOCUS
        );

        web.push_event(NativeEvent {
            surface_id: 1,
            node_id: 2,
            timestamp_us: 42,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 10.0, y: 20.0 },
        });
        web.push_event(NativeEvent {
            surface_id: 1,
            node_id: 3,
            timestamp_us: 99,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text("hello".into()),
        });

        assert_eq!(callback_hits.get(), 2);
        let drained = web.drain_events();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].node_id, 2);
        assert_eq!(drained[1].payload, EventPayload::Text("hello".into()));
        assert!(web.drain_events().is_empty());

        // Clearing event mask updates mirror state cleanly.
        let mut b2 = CommandBufferBuilder::new(1, 2);
        b2.set_event_mask(2, EventMask::NONE);
        b2.commit(true);
        web.submit_frame(b2.finish()).unwrap();
        assert_eq!(web.event_mask_of(2), EventMask::NONE);
    }

    #[test]
    fn applies_incremental_patches_and_raw_props() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "Count: 0");
        b.create_node(4, ComponentKind::TextInput, None);
        b.set_raw_prop(4, "placeholder", &RawPropValue::String("Type here".into()));
        b.set_children(2, &[3, 4]);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();

        assert_eq!(
            web.node(4).unwrap().raw_props.get("placeholder"),
            Some(&RawPropValue::String("Type here".into()))
        );
        assert!(web.render_html().contains("Count: 0</span>"));

        // Incremental text patch only mutates node 3.
        let mut b2 = CommandBufferBuilder::new(1, 2);
        b2.set_text(3, "Count: 1");
        b2.commit(true);
        let stats = web.submit_frame(b2.finish()).unwrap();
        assert_eq!(stats.mutation_count, 1);
        assert!(web.render_html().contains("Count: 1</span>"));
        assert_eq!(
            web.node(4).unwrap().raw_props.get("placeholder"),
            Some(&RawPropValue::String("Type here".into()))
        );

        // Tier-1 live template patch updates text and style immediately.
        let patched = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
            generation: 1,
            template_id: "counter".into(),
            attr_updates: vec![("style".into(), "background-color: #111827".into())],
            text_updates: vec![(0, "Count: 99".into())],
            target_test_id: None,
            node_path: vec![],
            text_replacements: vec![],
        });
        assert!(patched >= 2);
        assert!(web.render_html().contains("Count: 99</span>"));
        assert!(web
            .node(2)
            .unwrap()
            .combined_css()
            .contains("background-color: #111827;"));
    }

    #[test]
    fn targeted_template_patch_updates_only_matching_dom_mirror_nodes() {
        let mut web = WebBackend::new();
        let mut b = CommandBufferBuilder::new(1, 1);
        b.create_node(2, ComponentKind::View, None);
        b.set_raw_prop(2, "testID", &RawPropValue::String("nav-eng".into()));
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "# engineering");
        b.set_children(2, &[3]);

        b.create_node(4, ComponentKind::View, None);
        b.set_raw_prop(4, "testID", &RawPropValue::String("nav-design".into()));
        b.create_node(5, ComponentKind::Text, None);
        b.set_text(5, "# design-sync");
        b.set_children(4, &[5]);

        b.set_children(ROOT_NODE_ID, &[2, 4]);
        b.commit(true);
        web.submit_frame(b.finish()).unwrap();

        // Targeted style + text replacement mutates ONLY `nav-eng` and `# engineering`.
        let mutations = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
            generation: 2,
            template_id: "ui.rs".into(),
            attr_updates: vec![("style".into(), "padding: 14px; background-color: #1d4ed8;".into())],
            text_updates: vec![(0, "# ergoiherg".into())],
            target_test_id: Some("nav-eng".into()),
            node_path: vec![0],
            text_replacements: vec![("# engineering".into(), "# ergoiherg".into())],
        });
        assert_eq!(mutations, 2);
        assert_eq!(web.node(3).unwrap().text.as_deref(), Some("# ergoiherg"));
        assert_eq!(web.node(5).unwrap().text.as_deref(), Some("# design-sync"));
        assert!(web
            .node(2)
            .unwrap()
            .combined_css()
            .contains("background-color: #1d4ed8;"));
        assert!(!web
            .node(4)
            .unwrap()
            .combined_css()
            .contains("background-color: #1d4ed8;"));

        // Unknown `target_test_id` yields 0 mutations and leaves all nodes untouched.
        let noop = web.apply_template_patch(&nve_engine::HotPatchMsg::TemplatePatch {
            generation: 3,
            template_id: "ui.rs".into(),
            attr_updates: vec![("opacity".into(), "0.2".into())],
            text_updates: vec![],
            target_test_id: Some("missing-id".into()),
            node_path: vec![],
            text_replacements: vec![],
        });
        assert_eq!(noop, 0);
        assert!(!web.node(2).unwrap().combined_css().contains("opacity:0.2;"));
        assert!(!web.node(4).unwrap().combined_css().contains("opacity:0.2;"));
    }
}
