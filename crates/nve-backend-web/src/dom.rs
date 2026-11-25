//! Web DOM synchronization, keyed O(N) head/tail-trimmed `SetChildren` reconciler,
//! ARIA accessibility & input attribute mapping, legacy inline HTML dump (`render_html`),
//! canvas repainting, and orphan pruning.

use core::fmt::Write as _;
use std::collections::HashSet;

use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};
#[cfg(target_arch = "wasm32")]
use nve_abi::Dimension;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::canvas;
#[cfg(target_arch = "wasm32")]
use crate::css::is_raw_style_prop;
#[cfg(target_arch = "wasm32")]
use crate::reconcile::{plan_keyed_child_ops, ChildDiffOp};
use crate::stylesheet::resolve_node_style;
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
    Set(&'static str, &'a str),
    Remove(&'static str),
    Disabled(bool),
    InputPassword(bool),
    ReadOnly(bool),
}

/// Maps NVE accessibility and `<input>` semantics onto canonical HTML/ARIA attributes.
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

#[cfg(target_arch = "wasm32")]
#[inline]
fn set_attr_if_changed(el: &web_sys::Element, name: &str, value: &str) {
    if el.get_attribute(name).as_deref() != Some(value) {
        let _ = el.set_attribute(name, value);
    }
}

#[cfg(target_arch = "wasm32")]
#[inline]
fn remove_attr_if_present(el: &web_sys::Element, name: &str) {
    if el.has_attribute(name) {
        let _ = el.remove_attribute(name);
    }
}

impl WebBackend {
    /// Serializes the DOM mirror subtree rooted at `id` directly into `out` (inline-style
    /// diagnostic dump used by mirror tests and benchmarks; use [`Self::render_ssr`] for
    /// production Semantic HTML5 + Option 5 Hybrid CSS SSR).
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
        let (is_input, wrap_code) = self
            .nodes
            .get(&node_id)
            .map(|n| (n.kind == ComponentKind::TextInput, n.wrap_code))
            .unwrap_or((false, false));

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
                let target = self.text_or_child_mount_target(dom_el, wrap_code);
                if target.text_content().as_deref() != Some(text) {
                    target.set_text_content(Some(text));
                }
            }
        }
    }

    /// Resolves Option 5 Hybrid styles for every surviving dirty node in one batched pass,
    /// flushing the stylesheet once before updating DOM `className` and inline `style`.
    pub(crate) fn flush_dirty_styles(&mut self, dirty_ids: &[NodeId]) {
        if dirty_ids.is_empty() {
            return;
        }

        // 1. Resolve class names, structural rules, and inline lane overrides for surviving nodes.
        let mut updates: Vec<(NodeId, String, String)> = Vec::with_capacity(dirty_ids.len());
        for &node_id in dirty_ids {
            let Some(node) = self.nodes.get(&node_id) else {
                continue;
            };
            let parent = self
                .parent_of
                .get(&node_id)
                .and_then(|pid| self.nodes.get(pid));
            let allow_new = self.stylesheet.can_insert_new();
            let resolved = resolve_node_style(node, parent, allow_new, |cls| {
                self.stylesheet.contains_rule(cls)
            });
            if let Some((cls, decls)) = resolved.structural_rule {
                let _ = self.stylesheet.ensure_rule(cls, decls);
            }
            updates.push((node_id, resolved.class_name, resolved.inline_style));
        }

        // 2. Flush any newly queued CSS rules to the browser stylesheet ONCE.
        self.stylesheet.flush_to_dom();

        // 3. Commit class & inline-style state to mirror and browser DOM.
        for (node_id, class_name, inline_style) in updates {
            let had_inline = self
                .nodes
                .get(&node_id)
                .is_some_and(|n| n.has_inline_style);
            let now_has_inline = !inline_style.is_empty();

            #[cfg(target_arch = "wasm32")]
            if let Some(dom_el) = self.dom_nodes.get(&node_id) {
                if dom_el.class_name() != class_name {
                    dom_el.set_class_name(&class_name);
                }
                if let Some(html_el) = dom_el.dyn_ref::<web_sys::HtmlElement>() {
                    if now_has_inline {
                        if html_el.style().css_text() != inline_style {
                            html_el.style().set_css_text(&inline_style);
                        }
                    } else if had_inline || !html_el.style().css_text().is_empty() {
                        html_el.style().set_css_text("");
                    }
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            let _ = had_inline;

            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.class_name = class_name;
                node.inline_style = inline_style;
                node.has_inline_style = now_has_inline;
            }
        }
    }

    /// Reconciles `parent_id`'s live browser DOM children using O(N) head/tail trimming + LIS,
    /// calling `remove_child` and `insert_before` ONLY for removed, inserted, or moved children.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn reconcile_dom_children(&self, parent_id: NodeId, new_children: &[NodeId]) {
        let Some(parent_dom) = self.dom_nodes.get(&parent_id) else {
            return;
        };
        let wrap_code = self.nodes.get(&parent_id).is_some_and(|n| n.wrap_code);
        let container = self.text_or_child_mount_target(parent_dom, wrap_code);
        let desired_set: HashSet<NodeId> = new_children.iter().copied().collect();

        // Read existing DOM children, removing stray/obsolete nodes in-place without touching survivors.
        let child_nodes = container.child_nodes();
        let len = child_nodes.length();
        let mut to_remove: Vec<web_sys::Node> = Vec::new();
        let mut old_ids: Vec<NodeId> = Vec::with_capacity(len as usize);
        let mut seen_old: HashSet<NodeId> = HashSet::with_capacity(len as usize);
        let keep_text_node = new_children.is_empty()
            && self
                .nodes
                .get(&parent_id)
                .is_some_and(|n| n.text.is_some());

        for i in 0..len {
            let Some(child) = child_nodes.item(i) else {
                continue;
            };
            if let Some(el) = child.dyn_ref::<web_sys::Element>()
                && let Some(raw_id) = el.get_attribute("data-nve-id")
                && let Ok(nid) = raw_id.parse::<NodeId>()
                && desired_set.contains(&nid)
                && self
                    .dom_nodes
                    .get(&nid)
                    .is_some_and(|live| live.is_same_node(Some(&child)))
                && seen_old.insert(nid)
            {
                old_ids.push(nid);
            } else if keep_text_node && child.node_type() == web_sys::Node::TEXT_NODE {
                continue;
            } else {
                to_remove.push(child);
            }
        }

        for dead in to_remove {
            let _ = container.remove_child(&dead);
        }

        // Plan minimal moves/insertions among surviving & new children.
        let ops = plan_keyed_child_ops(&old_ids, new_children);
        for op in ops {
            match op {
                ChildDiffOp::Remove(nid) => {
                    if let Some(child_el) = self.dom_nodes.get(&nid)
                        && child_el.parent_node().is_some_and(|p| p.is_same_node(Some(&container)))
                    {
                        let _ = container.remove_child(child_el);
                    }
                }
                ChildDiffOp::InsertBefore { node_id, before } => {
                    let Some(child_el) = self.dom_nodes.get(&node_id) else {
                        continue;
                    };
                    let anchor: Option<&web_sys::Node> = before
                        .and_then(|bid| self.dom_nodes.get(&bid))
                        .map(|el| el.unchecked_ref::<web_sys::Node>())
                        .filter(|node| {
                            node.parent_node()
                                .is_some_and(|p| p.is_same_node(Some(&container)))
                        });
                    let _ = container.insert_before(child_el, anchor);
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn text_or_child_mount_target(
        &self,
        dom_el: &web_sys::Element,
        wrap_code: bool,
    ) -> web_sys::Element {
        if wrap_code
            && let Some(first) = dom_el.first_element_child()
            && first.tag_name().eq_ignore_ascii_case("code")
        {
            return first;
        }
        dom_el.clone()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_dom_raw_prop(&mut self, node_id: NodeId, key: &str, value: &RawPropValue) {
        // CSS style props belong exclusively in `RawStyleProps` / stylesheet, never HTML attrs.
        if is_raw_style_prop(key) {
            return;
        }
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
                MappedHtmlAttr::Set(attr, val) => set_attr_if_changed(dom_el, attr, val),
                MappedHtmlAttr::Remove(attr) => remove_attr_if_present(dom_el, attr),
                MappedHtmlAttr::Disabled(true) => {
                    set_attr_if_changed(dom_el, "disabled", "");
                    set_attr_if_changed(dom_el, "aria-disabled", "true");
                }
                MappedHtmlAttr::Disabled(false) => {
                    remove_attr_if_present(dom_el, "disabled");
                    remove_attr_if_present(dom_el, "aria-disabled");
                }
                MappedHtmlAttr::InputPassword(true) => {
                    set_attr_if_changed(dom_el, "type", "password");
                }
                MappedHtmlAttr::InputPassword(false) => {
                    set_attr_if_changed(dom_el, "type", "text");
                }
                MappedHtmlAttr::ReadOnly(true) => set_attr_if_changed(dom_el, "readonly", ""),
                MappedHtmlAttr::ReadOnly(false) => remove_attr_if_present(dom_el, "readonly"),
            }
            return;
        }

        let is_image = kind == ComponentKind::Image;
        match value {
            RawPropValue::Null => {
                remove_attr_if_present(dom_el, key);
                if is_image && key == "uri" {
                    remove_attr_if_present(dom_el, "src");
                }
            }
            RawPropValue::Bool(true) => set_attr_if_changed(dom_el, key, ""),
            RawPropValue::Bool(false) => remove_attr_if_present(dom_el, key),
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
                    set_attr_if_changed(dom_el, "src", s);
                }
                set_attr_if_changed(dom_el, key, s);
            }
            RawPropValue::I64(n) => {
                self.attr_scratch.clear();
                let _ = write!(&mut self.attr_scratch, "{n}");
                set_attr_if_changed(dom_el, key, &self.attr_scratch);
            }
            RawPropValue::F64(n) => {
                self.attr_scratch.clear();
                let _ = write!(&mut self.attr_scratch, "{n}");
                set_attr_if_changed(dom_el, key, &self.attr_scratch);
            }
            RawPropValue::Bytes(_) => {}
        }
    }

    /// Replaces a live DOM element in-place when semantic raw props upgrade its HTML5 tag
    /// (e.g. `<span>` -> `<h1>`, `<div>` -> `<nav>`, `<button>` -> `<a href="...">`).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn upgrade_dom_element_tag(&mut self, node_id: NodeId) {
        let Some((new_tag, wrap_code)) = self.nodes.get(&node_id).map(|n| (n.tag, n.wrap_code))
        else {
            return;
        };
        let Some(old_el) = self.dom_nodes.get(&node_id).cloned() else {
            return;
        };
        if old_el.tag_name().eq_ignore_ascii_case(new_tag) {
            return;
        }
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(new_el) = doc.create_element(new_tag) else {
            return;
        };
        self.attr_scratch.clear();
        let _ = write!(&mut self.attr_scratch, "{node_id}");
        let _ = new_el.set_attribute("data-nve-id", &self.attr_scratch);

        let mount_target = if wrap_code {
            if let Ok(code_el) = doc.create_element("code") {
                let _ = new_el.append_child(&code_el);
                code_el
            } else {
                new_el.clone()
            }
        } else {
            new_el.clone()
        };

        while let Some(first) = old_el.first_child() {
            let _ = mount_target.append_child(&first);
        }

        if let Some(parent) = old_el.parent_node() {
            let _ = parent.replace_child(&new_el, &old_el);
        }
        self.dom_nodes.insert(node_id, new_el);

        // Re-sync attributes & text onto the upgraded element.
        if let Some(props) = self.nodes.get(&node_id).map(|n| n.raw_props.clone()) {
            for (k, v) in &props {
                self.sync_dom_raw_prop(node_id, k, v);
            }
        }
        if let Some(text) = self.nodes.get(&node_id).and_then(|n| n.text.clone()) {
            self.apply_text_mutation(node_id, &text);
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
            self.parent_of.remove(&nid);
            self.remove_node_event_meta(nid);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.dom_nodes.remove(&nid) {
                el.remove();
            }
        }
    }
}
