//! Web DOM synchronization, ARIA accessibility & input attribute mapping,
//! HTML SSR rendering (`fmt_html_into`/`render_html`), canvas repainting, and orphan pruning.

use core::fmt::Write as _;
use std::collections::HashSet;

use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};
#[cfg(target_arch = "wasm32")]
use nve_abi::Dimension;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::canvas;
use crate::WebBackend;

/// Evaluates whether a [`RawPropValue`] represents a boolean-true flag (e.g. `disabled`,
/// `secureTextEntry`, `readOnly`).
#[inline]
pub fn is_truthy_raw_prop(value: &RawPropValue) -> bool {
    match value {
        RawPropValue::Bool(b) => *b,
        RawPropValue::String(s) => {
            let t = s.trim();
            !t.is_empty()
                && !matches!(
                    t.to_ascii_lowercase().as_str(),
                    "false" | "0" | "no" | "off"
                )
        }
        RawPropValue::I64(n) => *n != 0,
        RawPropValue::F64(n) => *n != 0.0,
        RawPropValue::Null | RawPropValue::Bytes(_) => false,
    }
}

/// Canonical HTML / ARIA attribute mutation produced from an NVE raw prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedHtmlAttr<'a> {
    /// Set a single string HTML attribute (`name`, `value`).
    Set(&'static str, &'a str),
    /// Remove a single HTML attribute.
    Remove(&'static str),
    /// Set or clear both `disabled` and `aria-disabled="true"`.
    Disabled(bool),
    /// Set `<input type="password">` or restore `<input type="text">`.
    InputPassword(bool),
    /// Set or clear boolean `readonly`.
    ReadOnly(bool),
}

/// Maps NVE accessibility and `<input>` semantics onto canonical HTML/ARIA attributes:
/// - `accessibilityLabel` -> `aria-label`
/// - `accessibilityRole` -> `role`
/// - `accessibilityHint` -> `aria-description`
/// - `disabled` -> `disabled` + `aria-disabled="true"`
/// - `secureTextEntry` -> `type="password"` (on `<input>`)
/// - `readOnly` -> `readonly`
pub fn map_a11y_or_input_attr<'a>(
    kind: ComponentKind,
    key: &str,
    value: &'a RawPropValue,
) -> Option<MappedHtmlAttr<'a>> {
    match key {
        "accessibilityLabel" | "accessibility_label" => match value {
            RawPropValue::String(s) if !s.is_empty() => Some(MappedHtmlAttr::Set("aria-label", s)),
            _ => Some(MappedHtmlAttr::Remove("aria-label")),
        },
        "accessibilityRole" | "accessibility_role" => match value {
            RawPropValue::String(s) if !s.is_empty() => Some(MappedHtmlAttr::Set("role", s)),
            _ => Some(MappedHtmlAttr::Remove("role")),
        },
        "accessibilityHint" | "accessibility_hint" => match value {
            RawPropValue::String(s) if !s.is_empty() => {
                Some(MappedHtmlAttr::Set("aria-description", s))
            }
            _ => Some(MappedHtmlAttr::Remove("aria-description")),
        },
        "disabled" => Some(MappedHtmlAttr::Disabled(is_truthy_raw_prop(value))),
        "secureTextEntry" | "secure_text_entry" if kind == ComponentKind::TextInput => {
            Some(MappedHtmlAttr::InputPassword(is_truthy_raw_prop(value)))
        }
        "readOnly" | "read_only" | "readonly" => {
            Some(MappedHtmlAttr::ReadOnly(is_truthy_raw_prop(value)))
        }
        _ => None,
    }
}

fn push_html_attr_escaped(val: &str, out: &mut String) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn write_attr(name: &str, val: &str, out: &mut String) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_html_attr_escaped(val, out);
    out.push('"');
}

impl WebBackend {
    /// Serializes the DOM mirror subtree rooted at `id` directly into `out` with zero intermediate allocations.
    pub fn fmt_html_into(&self, id: NodeId, out: &mut String) {
        let Some(el) = self.nodes.get(&id) else {
            return;
        };
        out.push('<');
        out.push_str(el.tag);
        out.push_str(" data-nve-id=\"");
        let _ = write!(out, "{id}");
        out.push('"');

        if el.kind == ComponentKind::Image {
            if let Some(RawPropValue::String(src)) = el
                .raw_props
                .get("src")
                .or_else(|| el.raw_props.get("uri"))
            {
                write_attr("src", src, out);
            }
            if let Some(RawPropValue::String(alt)) = el.raw_props.get("alt") {
                write_attr("alt", alt, out);
            }
        } else if el.kind == ComponentKind::SkiaCanvas && el.draw_op_count > 0 {
            out.push_str(" data-nve-draw-ops=\"");
            let _ = write!(out, "{}", el.draw_op_count);
            out.push('"');
        }

        // Emit canonical ARIA accessibility and input attributes in deterministic order.
        if let Some(RawPropValue::String(label)) = el
            .raw_props
            .get("accessibilityLabel")
            .or_else(|| el.raw_props.get("accessibility_label"))
            && !label.is_empty()
        {
            write_attr("aria-label", label, out);
        }
        if let Some(RawPropValue::String(role)) = el
            .raw_props
            .get("accessibilityRole")
            .or_else(|| el.raw_props.get("accessibility_role"))
            && !role.is_empty()
        {
            write_attr("role", role, out);
        }
        if let Some(RawPropValue::String(hint)) = el
            .raw_props
            .get("accessibilityHint")
            .or_else(|| el.raw_props.get("accessibility_hint"))
            && !hint.is_empty()
        {
            write_attr("aria-description", hint, out);
        }
        if el.raw_props.get("disabled").is_some_and(is_truthy_raw_prop) {
            out.push_str(" disabled aria-disabled=\"true\"");
        }
        if el.kind == ComponentKind::TextInput
            && el
                .raw_props
                .get("secureTextEntry")
                .or_else(|| el.raw_props.get("secure_text_entry"))
                .is_some_and(is_truthy_raw_prop)
        {
            out.push_str(" type=\"password\"");
        }
        if el
            .raw_props
            .get("readOnly")
            .or_else(|| el.raw_props.get("read_only"))
            .or_else(|| el.raw_props.get("readonly"))
            .is_some_and(is_truthy_raw_prop)
        {
            out.push_str(" readonly");
        }

        out.push_str(" style=\"");
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

    pub(crate) fn apply_text_mutation(&mut self, node_id: NodeId, text: &str) {
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
                if let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>()
                    && inp.value() != text
                {
                    inp.set_value(text);
                }
            } else {
                dom_el.set_text_content(Some(text));
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_dom_style(&mut self, node_id: NodeId) {
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
    pub(crate) fn sync_dom_raw_prop(&mut self, node_id: NodeId, key: &str, value: &RawPropValue) {
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        let kind = self
            .nodes
            .get(&node_id)
            .map(|n| n.kind)
            .unwrap_or(ComponentKind::View);

        if let Some(mapped) = map_a11y_or_input_attr(kind, key, value) {
            match mapped {
                MappedHtmlAttr::Set(attr, val) => {
                    let _ = dom_el.set_attribute(attr, val);
                }
                MappedHtmlAttr::Remove(attr) => {
                    let _ = dom_el.remove_attribute(attr);
                }
                MappedHtmlAttr::Disabled(true) => {
                    let _ = dom_el.set_attribute("disabled", "");
                    let _ = dom_el.set_attribute("aria-disabled", "true");
                }
                MappedHtmlAttr::Disabled(false) => {
                    let _ = dom_el.remove_attribute("disabled");
                    let _ = dom_el.remove_attribute("aria-disabled");
                }
                MappedHtmlAttr::InputPassword(true) => {
                    let _ = dom_el.set_attribute("type", "password");
                }
                MappedHtmlAttr::InputPassword(false) => {
                    let _ = dom_el.set_attribute("type", "text");
                }
                MappedHtmlAttr::ReadOnly(true) => {
                    let _ = dom_el.set_attribute("readonly", "");
                }
                MappedHtmlAttr::ReadOnly(false) => {
                    let _ = dom_el.remove_attribute("readonly");
                }
            }
            return;
        }

        let is_image = kind == ComponentKind::Image;
        match value {
            RawPropValue::Null => {
                let _ = dom_el.remove_attribute(key);
                if is_image && key == "uri" {
                    let _ = dom_el.remove_attribute("src");
                }
            }
            RawPropValue::Bool(true) => {
                let _ = dom_el.set_attribute(key, "");
            }
            RawPropValue::Bool(false) => {
                let _ = dom_el.remove_attribute(key);
            }
            RawPropValue::String(s) => {
                if key == "value"
                    && let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>()
                {
                    if inp.value() != *s {
                        inp.set_value(s);
                    }
                    return;
                }
                if is_image && key == "uri" {
                    let _ = dom_el.set_attribute("src", s);
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
    pub(crate) fn repaint_canvas_node(&self, node_id: NodeId) {
        let Some(mirror) = self.nodes.get(&node_id) else {
            return;
        };
        if mirror.kind != ComponentKind::SkiaCanvas {
            return;
        }
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        let fallback_w = match mirror.layout.width {
            Dimension::Points(v) if v > 0.0 => v as f64,
            _ => 320.0,
        };
        let fallback_h = match mirror.layout.height {
            Dimension::Points(v) if v > 0.0 => v as f64,
            _ => 72.0,
        };
        canvas::paint_canvas_element(dom_el, &mirror.draw_ops, fallback_w, fallback_h);
    }

    /// Repaints every mounted `<canvas>` node (e.g. after a browser window resize updates CSS widths).
    #[cfg(target_arch = "wasm32")]
    pub fn repaint_all_canvases(&self) {
        for (&node_id, mirror) in &self.nodes {
            if mirror.kind == ComponentKind::SkiaCanvas {
                self.repaint_canvas_node(node_id);
            }
        }
    }

    /// Removes nodes no longer reachable from `ROOT_NODE_ID` after a full VDOM rebuild/reset.
    pub(crate) fn prune_unreachable_nodes(&mut self) {
        let mut reachable = HashSet::with_capacity(self.nodes.len());
        let mut stack = vec![ROOT_NODE_ID];
        while let Some(nid) = stack.pop() {
            if reachable.insert(nid)
                && let Some(el) = self.nodes.get(&nid)
            {
                for &cid in &el.children {
                    stack.push(cid);
                }
            }
        }
        let dead_ids: Vec<NodeId> = self
            .nodes
            .keys()
            .copied()
            .filter(|id| !reachable.contains(id))
            .collect();
        for nid in dead_ids {
            self.nodes.remove(&nid);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.dom_nodes.remove(&nid) {
                self.detach_listeners(nid, &el);
                el.remove();
            }
        }
    }
}
