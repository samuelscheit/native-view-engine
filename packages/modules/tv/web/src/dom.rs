//! Just-in-time DOM focusable snapshot (`getBoundingClientRect` on keypress only), focus-trap
//! scoping, DOM focus, and scroll-into-view for the `wasm32` controller. Every decision is a
//! pure [`crate::rules`] call over facts read here.

use std::sync::OnceLock;

use js_sys::{Function, Reflect};
use nve_backend_web::parse_dom_node_id;
use nve_tv::{
    FocusDirection, NodeId, SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::rules::{ActivationFacts, FocusableFacts, TrapFacts, is_hidden_visibility};
use crate::{ActivationTarget, SelectActivation};

/// Focusable NVE nodes. Comma list instead of `:is()` / complex `:not()` for the Chromium 56-79
/// engines shipped by Tizen and webOS TVs; [`FocusableFacts`] filters the matches.
const FOCUSABLE_SELECTOR: &str = "a[href][data-nve-id],button[data-nve-id],input[data-nve-id],\
    select[data-nve-id],textarea[data-nve-id],summary[data-nve-id],[tabindex][data-nve-id],\
    [contenteditable][data-nve-id]";
/// Searched in the whole document: modal dialogs may be portalled outside the surface.
const DIALOG_TRAP_SELECTOR: &str = "dialog[open]";
/// Searched inside the surface.
const EXPLICIT_TRAP_SELECTOR: &str = "[data-nve-focus-trap]";
const GROUP_ATTR: &str = "data-nve-focus-group";
const GROUP_SELECTOR: &str = "[data-nve-focus-group]";
const NEXT_ATTRS: [(FocusDirection, &str); 4] = [
    (FocusDirection::Up, "data-nve-next-up"),
    (FocusDirection::Down, "data-nve-next-down"),
    (FocusDirection::Left, "data-nve-next-left"),
    (FocusDirection::Right, "data-nve-next-right"),
];

/// Last collected focusables, parallel to the engine's arena (buffers reused across moves).
#[derive(Default)]
pub(crate) struct DomFocusables {
    root: Option<web_sys::Element>,
    elements: Vec<(NodeId, web_sys::Element)>,
}

impl DomFocusables {
    pub(crate) fn set_root(&mut self, root: &web_sys::Element) {
        if self.root.as_ref() != Some(root) {
            self.root = Some(root.clone());
        }
    }

    /// Surface root, else the document element (before the first mount / keydown).
    fn root(&self) -> Option<web_sys::Element> {
        self.root.clone().or_else(|| document()?.document_element())
    }

    /// Replaces the engine's nodes with the current layout of every eligible focusable inside
    /// [`Self::scope`]. `focused` is the node focus leaves (its `data-nve-next-*` are read).
    pub(crate) fn collect(&mut self, engine: &mut SpatialFocusEngine, focused: Option<NodeId>) {
        self.elements.clear();
        let scope = self.scope();
        let list = scope
            .as_ref()
            .and_then(|s| s.query_selector_all(FOCUSABLE_SELECTOR).ok());
        let len = list.as_ref().map_or(0, web_sys::NodeList::length);
        let style_source = web_sys::window();
        engine.set_viewport(viewport());
        let elements = &mut self.elements;
        engine.replace_nodes((0..len).filter_map(|i| {
            let el: web_sys::Element = list.as_ref()?.item(i)?.dyn_into().ok()?;
            let node = focusable_node(&el, scope.as_ref()?, style_source.as_ref()?, focused)?;
            elements.push((node.id, el));
            Some(node)
        }));
    }

    /// Topmost active modal `<dialog>` in the document, else the topmost active
    /// `[data-nve-focus-trap]` in the surface, else the surface root.
    fn scope(&self) -> Option<web_sys::Element> {
        let root = self.root()?;
        let dialogs = document().and_then(|d| d.query_selector_all(DIALOG_TRAP_SELECTOR).ok());
        topmost_trap(dialogs)
            .or_else(|| topmost_trap(root.query_selector_all(EXPLICIT_TRAP_SELECTOR).ok()))
            .or(Some(root))
    }

    pub(crate) fn element(&self, node: NodeId) -> Option<web_sys::Element> {
        self.elements
            .iter()
            .find(|(id, _)| *id == node)
            .map(|(_, el)| el.clone())
            .or_else(|| element_for_node(&self.root()?, node))
    }

    /// The element a Select on `node` activates: looked up inside the active scope (never
    /// behind a modal) and only when [`ActivationFacts::is_activatable`].
    pub(crate) fn activation_target(&self, node: NodeId) -> Option<web_sys::Element> {
        let el = element_for_node(&self.scope()?, node)?;
        let (tag, tabindex) = (el.tag_name(), el.get_attribute("tabindex"));
        let facts = ActivationFacts {
            tag: &tag,
            tabindex: tabindex.as_deref(),
            disabled: el.has_attribute("disabled") || el.matches(":disabled").unwrap_or(false),
            inert: has_ancestor(&el, "[inert]"),
        };
        facts.is_activatable().then_some(el)
    }
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn has_ancestor(el: &web_sys::Element, selector: &str) -> bool {
    el.closest(selector).ok().flatten().is_some()
}

/// Layout viewport in client coordinates (the space of `getBoundingClientRect`).
fn viewport() -> Option<SpatialRect> {
    let root = document()?.document_element()?;
    Some(SpatialRect::new(
        0.0,
        0.0,
        root.client_width() as f32,
        root.client_height() as f32,
    ))
}

fn topmost_trap(list: Option<web_sys::NodeList>) -> Option<web_sys::Element> {
    let list = list?;
    (0..list.length())
        .rev()
        .filter_map(|i| list.item(i)?.dyn_into::<web_sys::Element>().ok())
        .find(is_active_trap)
}

fn is_active_trap(el: &web_sys::Element) -> bool {
    let (aria_modal, focus_trap) = (
        el.get_attribute("aria-modal"),
        el.get_attribute("data-nve-focus-trap"),
    );
    let rect = el.get_bounding_client_rect();
    TrapFacts {
        open_dialog: el.tag_name() == "DIALOG" && el.has_attribute("open"),
        modal: el.matches(":modal").ok(),
        aria_modal: aria_modal.as_deref(),
        focus_trap: focus_trap.as_deref(),
        inert_or_hidden: has_ancestor(el, "[inert],[hidden]"),
        rendered: rect.width() > 0.0 || rect.height() > 0.0,
    }
    .is_active()
}

fn focusable_node(
    el: &web_sys::Element,
    scope: &web_sys::Element,
    window: &web_sys::Window,
    focused: Option<NodeId>,
) -> Option<SpatialFocusNode> {
    let id = parse_dom_node_id(&el.get_attribute("data-nve-id")?)?;
    let tag = el.tag_name();
    let (tabindex, contenteditable) = (
        el.get_attribute("tabindex"),
        el.get_attribute("contenteditable"),
    );
    let input_type = el
        .dyn_ref::<web_sys::HtmlInputElement>()
        .map(web_sys::HtmlInputElement::type_);
    let facts = FocusableFacts {
        tag: &tag,
        has_href: el.has_attribute("href"),
        input_type: input_type.as_deref().unwrap_or_default(),
        tabindex: tabindex.as_deref(),
        contenteditable: contenteditable.as_deref(),
        disabled: el.matches(":disabled").unwrap_or(false),
        aria_disabled: has_ancestor(el, "[aria-disabled=true]"),
        inert_or_hidden: has_ancestor(el, "[inert],[hidden]"),
    };
    if !facts.is_focusable() {
        return None;
    }
    let r = el.get_bounding_client_rect();
    let rect = SpatialRect::new(
        r.left() as f32,
        r.top() as f32,
        r.width() as f32,
        r.height() as f32,
    );
    // `display: none` / collapsed (0x0) and `visibility: hidden` never take focus.
    if !rect.is_valid()
        || window
            .get_computed_style(el)
            .ok()
            .flatten()
            .and_then(|s| s.get_property_value("visibility").ok())
            .is_some_and(|v| is_hidden_visibility(&v))
    {
        return None;
    }
    let mut node = SpatialFocusNode::new(id, rect);
    // Only groups inside the scope: a modal's items never overwrite a background shelf's memory.
    if let Some(group) = el.closest(GROUP_SELECTOR).ok().flatten()
        && scope.contains(Some(&group))
    {
        let name = group.get_attribute(GROUP_ATTR).unwrap_or_default();
        node = node.with_group(SpatialFocusGroupId(
            nve_hash::xxh64(name.as_bytes(), 0) as u32
        ));
    }
    // Explicit neighbors only matter for the node focus moves away from.
    if focused == Some(id) {
        for (direction, attr) in NEXT_ATTRS {
            if let Some(target) = el
                .get_attribute(attr)
                .and_then(|dom_id| explicit_target(&dom_id))
            {
                node = node.with_next(direction, target);
            }
        }
    }
    Some(node)
}

/// `data-nve-next-*` names a DOM element id; resolve it to that element's NVE node.
fn explicit_target(dom_id: &str) -> Option<NodeId> {
    let el = document()?.get_element_by_id(dom_id)?;
    parse_dom_node_id(&el.get_attribute("data-nve-id")?)
}

fn element_for_node(scope: &web_sys::Element, node: NodeId) -> Option<web_sys::Element> {
    scope
        .query_selector(&format!("[data-nve-id=\"{}\"]", node.raw()))
        .ok()
        .flatten()
}

/// Node holding DOM focus (`document.activeElement`'s nearest `data-nve-id`).
pub(crate) fn active_node() -> Option<NodeId> {
    let active = document()?.active_element()?;
    let owner = active.closest("[data-nve-id]").ok()??;
    parse_dom_node_id(&owner.get_attribute("data-nve-id")?)
}

/// Blurs the focused element (Back inside a text control closes the on-screen keyboard).
pub(crate) fn blur_active() {
    if let Some(active) = document().and_then(|d| d.active_element())
        && let Some(html) = active.dyn_ref::<web_sys::HtmlElement>()
    {
        let _ = html.blur();
    }
}

fn holds_dom_focus(el: &web_sys::Element) -> bool {
    document().and_then(|d| d.active_element()).as_ref() == Some(el)
}

/// `scrollIntoView` options and CSSOM `scroll-behavior` shipped together (Chromium 61); older
/// TV engines (Tizen 3-4: Chromium 47-56) coerce the options object to `true` (align to top).
fn supports_scroll_into_view_options() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        document()
            .and_then(|d| d.document_element())
            .and_then(|root| Reflect::get(&root, &JsValue::from_str("style")).ok())
            .is_some_and(|style| {
                Reflect::has(&style, &JsValue::from_str("scrollBehavior")).unwrap_or(false)
            })
    })
}

/// Scrolls every scroll container of `el` the minimum needed to show it.
fn reveal(el: &web_sys::Element) {
    if supports_scroll_into_view_options() {
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    } else if let Some(if_needed) = method(el, "scrollIntoViewIfNeeded") {
        let _ = if_needed.call1(el, &JsValue::FALSE);
    } else {
        el.scroll_into_view_with_bool(false);
    }
}

fn method(obj: &JsValue, name: &str) -> Option<Function> {
    Reflect::get(obj, &JsValue::from_str(name))
        .ok()?
        .dyn_into()
        .ok()
}

impl ActivationTarget for web_sys::Element {
    fn activation(&self, key: &str) -> SelectActivation {
        let input_type = self
            .dyn_ref::<web_sys::HtmlInputElement>()
            .map(web_sys::HtmlInputElement::type_)
            .unwrap_or_default();
        SelectActivation::for_target(key, &self.tag_name(), &input_type, holds_dom_focus(self))
    }

    fn click(&self) {
        if let Some(html) = self.dyn_ref::<web_sys::HtmlElement>() {
            html.click();
        }
    }

    /// `HTMLSelectElement.showPicker()` (Chromium 121+); throws without user activation.
    fn show_picker(&self) -> bool {
        let Some(show) = method(self, "showPicker") else {
            return false;
        };
        if !holds_dom_focus(self) && !self.focus_and_reveal() {
            return false;
        }
        show.call0(self).is_ok()
    }

    fn focus_and_reveal(&self) -> bool {
        let Some(html) = self.dyn_ref::<web_sys::HtmlElement>() else {
            return false;
        };
        let opts = web_sys::FocusOptions::new();
        opts.set_prevent_scroll(true);
        let _ = html.focus_with_options(&opts);
        let landed = holds_dom_focus(self);
        if landed {
            reveal(self);
        }
        landed
    }
}
