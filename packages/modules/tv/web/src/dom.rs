//! Just-in-time DOM focusable snapshot (`getBoundingClientRect` on keypress only), DOM focus,
//! and scroll-into-view for the `wasm32` controller.

use nve_backend_web::parse_dom_node_id;
use nve_tv::{
    FocusDirection, NodeId, SpatialFocusEngine, SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
};
use wasm_bindgen::JsCast;

use crate::{ActivationTarget, SelectActivation};

/// Focusable NVE nodes. Comma list instead of `:is()` / complex `:not()` for the Chromium 56-79
/// engines shipped by Tizen and webOS TVs.
const FOCUSABLE_SELECTOR: &str = "a[href][data-nve-id],button[data-nve-id],input[data-nve-id],\
    select[data-nve-id],textarea[data-nve-id],summary[data-nve-id],[tabindex][data-nve-id],\
    [contenteditable][data-nve-id]";
const TRAP_SELECTOR: &str = "dialog[open],[data-nve-focus-trap]";
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

    /// Replaces the engine's nodes with the current layout of every eligible focusable inside the
    /// topmost focus trap (else the surface root, else the document).
    pub(crate) fn collect(&mut self, engine: &mut SpatialFocusEngine) {
        self.elements.clear();
        let list = self
            .scope()
            .and_then(|s| s.query_selector_all(FOCUSABLE_SELECTOR).ok());
        let len = list.as_ref().map_or(0, web_sys::NodeList::length);
        let focused = engine.focused();
        let elements = &mut self.elements;
        engine.replace_nodes((0..len).filter_map(|i| {
            let el: web_sys::Element = list.as_ref()?.item(i)?.dyn_into().ok()?;
            let node = focusable_node(&el, focused)?;
            elements.push((node.id, el));
            Some(node)
        }));
    }

    fn scope(&self) -> Option<web_sys::Element> {
        let root = self
            .root
            .clone()
            .or_else(|| web_sys::window()?.document()?.document_element())?;
        let traps = root.query_selector_all(TRAP_SELECTOR).ok()?;
        let topmost_trap = (0..traps.length())
            .rev()
            .filter_map(|i| traps.item(i)?.dyn_into::<web_sys::Element>().ok())
            .find(is_active_trap);
        Some(topmost_trap.unwrap_or(root))
    }

    pub(crate) fn element(&self, node: NodeId) -> Option<web_sys::Element> {
        self.elements
            .iter()
            .find(|(id, _)| *id == node)
            .map(|(_, el)| el.clone())
            .or_else(|| element_for_node(self.root.as_ref()?, node))
    }

    /// Focuses `node` without the browser's own jump-scroll, then scrolls it (and its shelf)
    /// into view by the minimum amount.
    pub(crate) fn focus(&self, node: NodeId) {
        let Some(el) = self.element(node) else { return };
        if let Some(html) = el.dyn_ref::<web_sys::HtmlElement>() {
            let opts = web_sys::FocusOptions::new();
            opts.set_prevent_scroll(true);
            let _ = html.focus_with_options(&opts);
        }
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
        opts.set_inline(web_sys::ScrollLogicalPosition::Nearest);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    }
}

/// A modal `<dialog>` (`:modal`, or `aria-modal` where `:modal` is unsupported) or an explicit
/// `[data-nve-focus-trap]` container.
fn is_active_trap(el: &web_sys::Element) -> bool {
    if el.has_attribute("data-nve-focus-trap") {
        return true;
    }
    el.matches(":modal")
        .unwrap_or_else(|_| el.get_attribute("aria-modal").as_deref() == Some("true"))
}

fn focusable_node(el: &web_sys::Element, focused: Option<NodeId>) -> Option<SpatialFocusNode> {
    if el.has_attribute("disabled")
        || el.get_attribute("tabindex").as_deref() == Some("-1")
        || el.closest("[inert],[hidden]").ok().flatten().is_some()
    {
        return None;
    }
    let id = parse_dom_node_id(&el.get_attribute("data-nve-id")?)?;
    let r = el.get_bounding_client_rect();
    let mut node = SpatialFocusNode::new(
        id,
        SpatialRect::new(
            r.left() as f32,
            r.top() as f32,
            r.width() as f32,
            r.height() as f32,
        ),
    );
    if let Some(group) = el.closest(GROUP_SELECTOR).ok().flatten() {
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
    let el = web_sys::window()?.document()?.get_element_by_id(dom_id)?;
    parse_dom_node_id(&el.get_attribute("data-nve-id")?)
}

fn element_for_node(root: &web_sys::Element, node: NodeId) -> Option<web_sys::Element> {
    root.query_selector(&format!("[data-nve-id=\"{}\"]", node.raw()))
        .ok()
        .flatten()
}

/// Node holding DOM focus (`document.activeElement`'s nearest `data-nve-id`).
pub(crate) fn active_node() -> Option<NodeId> {
    let active = web_sys::window()?.document()?.active_element()?;
    let owner = active.closest("[data-nve-id]").ok()??;
    parse_dom_node_id(&owner.get_attribute("data-nve-id")?)
}

impl ActivationTarget for web_sys::Element {
    fn activation(&self, key: &str) -> SelectActivation {
        let input_type = self
            .dyn_ref::<web_sys::HtmlInputElement>()
            .map(web_sys::HtmlInputElement::type_)
            .unwrap_or_default();
        SelectActivation::for_target(key, &self.tag_name(), &input_type)
    }

    fn click(&self) {
        if let Some(html) = self.dyn_ref::<web_sys::HtmlElement>() {
            html.click();
        }
    }
}
