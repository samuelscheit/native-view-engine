//! Web DOM synchronization, keyed O(N) head/tail-trimmed `SetChildren` reconciler,
//! ARIA accessibility & input attribute mapping, legacy inline HTML dump (`render_html`),
//! canvas repainting, and orphan pruning.

#[cfg(target_arch = "wasm32")]
use core::fmt::Write as _;
use nve_abi::hash::HashSet;

use nve_abi::{NodeId, ROOT_NODE_ID};
#[cfg(target_arch = "wasm32")]
use nve_abi::{ComponentKind, Dimension, RawPropValue};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(target_arch = "wasm32")]
use crate::canvas;
use crate::raw_style::is_raw_style_prop;
#[cfg(target_arch = "wasm32")]
use crate::reconcile::{plan_keyed_child_ops, ChildDiffOp};
use crate::stylesheet::resolve_node_style;
use crate::WebBackend;

pub use crate::input_attrs::{is_truthy_raw_prop, map_a11y_or_input_attr, MappedHtmlAttr};

/// Where a `SetRawProp` lands. One decision for the mirror, SSR and the live DOM path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPropRoute {
    /// `drawList`: decoded into canvas ops, shown as `data-nve-draw-ops`.
    DrawList,
    /// Native view props (NVNP payloads, navigation, camera state).
    NativeView,
    /// A CSS declaration slot of [`crate::RawStyleProps`]; never an HTML attribute.
    Style,
    /// Attributes the backend owns on every element: ignored, so an app prop cannot fight the
    /// stylesheet flush (`style`, `class`) or break the child lookup and hydration
    /// (`data-nve-id`, `data-nve-ssr-hash`).
    Reserved,
    /// Everything else: an HTML attribute (ARIA and input keys mapped by
    /// [`map_a11y_or_input_attr`]).
    Attribute,
}

#[inline]
fn is_native_view_prop_key(key: &str) -> bool {
    #[cfg(any(
        feature = "nv-blur",
        feature = "nv-webview",
        feature = "nv-video",
        feature = "nv-web-element",
        feature = "camera",
    ))]
    {
        crate::native_view::is_native_view_prop(key)
    }
    #[cfg(not(any(
        feature = "nv-blur",
        feature = "nv-webview",
        feature = "nv-video",
        feature = "nv-web-element",
        feature = "camera",
    )))]
    {
        matches!(
            key,
            "nve.native_kind" | "nve.native_props" | "nve.native.kind" | "nve.native.props"
        )
    }
}

/// Routes a raw prop key; see [`RawPropRoute`].
pub fn raw_prop_route(key: &str) -> RawPropRoute {
    if crate::canvas::is_draw_list_prop(key) {
        RawPropRoute::DrawList
    } else if is_native_view_prop_key(key) {
        RawPropRoute::NativeView
    } else if is_raw_style_prop(key) {
        RawPropRoute::Style
    } else if matches!(key, "style" | "class" | "data-nve-id" | "data-nve-ssr-hash") {
        RawPropRoute::Reserved
    } else {
        RawPropRoute::Attribute
    }
}



#[cfg(target_arch = "wasm32")]
#[inline]
pub(crate) fn create_dom_element_for_tag(
    doc: &web_sys::Document,
    tag: &str,
) -> Option<web_sys::Element> {
    if tag == "svg" {
        doc.create_element_ns(Some("http://www.w3.org/2000/svg"), "svg")
            .ok()
    } else {
        doc.create_element(tag).ok()
    }
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
    pub(crate) fn apply_text_mutation(&mut self, node_id: NodeId, text: &str) {
        #[cfg(target_arch = "wasm32")]
        let (is_input, wrap_code, has_children) = self
            .nodes
            .get(&node_id)
            .map(|n| (n.kind == ComponentKind::TextInput, n.wrap_code, !n.children.is_empty()))
            .unwrap_or((false, false, false));

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
                #[cfg(feature = "hydrate")]
                if self.hydrating && self.hydrated_controls.contains_key(&node_id) {
                    // Defer live DOM `.value` write to 3-way `StateReconciler` in `finalize_hydration`.
                    return;
                }
                self.input_sync.borrow_mut().on_backend_value_sync(node_id, text);
                if let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>()
                    && inp.value() != text
                {
                    inp.set_value(text);
                } else if let Some(ta) = dom_el.dyn_ref::<web_sys::HtmlTextAreaElement>()
                    && ta.value() != text
                {
                    ta.set_value(text);
                }
            } else if !has_children {
                // Text shows only while the node has no element children (it returns when
                // `SetChildren` empties the node, see `reconcile_dom_children`).
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

        // 1. Resolve class names, structural rules, responsive `@media` rules, and inline lane.
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
            #[cfg(any(
                feature = "nv-blur",
                feature = "nv-webview",
                feature = "nv-video",
                feature = "nv-web-element",
                feature = "camera",
            ))]
            let mut resolved = resolve_node_style(node, parent, allow_new, |cls| {
                self.stylesheet.contains_rule(cls)
            });
            #[cfg(not(any(
                feature = "nv-blur",
                feature = "nv-webview",
                feature = "nv-video",
                feature = "nv-web-element",
                feature = "camera",
            )))]
            let resolved = resolve_node_style(node, parent, allow_new, |cls| {
                self.stylesheet.contains_rule(cls)
            });
            if let Some((cls, decls)) = resolved.structural_rule {
                let _ = self.stylesheet.ensure_rule(cls, decls);
            }
            if let Some((cls, decls)) = resolved.cursor_rule {
                let _ = self.stylesheet.ensure_rule(cls.to_owned(), decls.to_owned());
            }
            if let Some((cls, decls)) = resolved.transition_rule {
                let _ = self.stylesheet.ensure_rule(cls, decls);
            }
            for media_rule in resolved.media_rules {
                let _ = self.stylesheet.ensure_media_rule(media_rule);
            }
            for state_rule in resolved.state_rules {
                let _ = self.stylesheet.ensure_state_rule(state_rule);
            }
            #[cfg(any(
                feature = "nv-blur",
                feature = "nv-webview",
                feature = "nv-video",
                feature = "nv-web-element",
                feature = "camera",
            ))]
            if let Some(nv) = self.native_views.get(&node_id) && !nv.extra_css.is_empty() {
                resolved.inline_style.push_str(&nv.extra_css);
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
        let mut seen_old: HashSet<NodeId> = HashSet::with_capacity_and_hasher(len as usize, Default::default());
        let restore_text = self
            .nodes
            .get(&parent_id)
            .filter(|n| new_children.is_empty() && n.kind != ComponentKind::TextInput)
            .and_then(|n| n.text.as_deref());
        let keep_text_node = restore_text.is_some();

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
        if new_children.is_empty() && self.nodes.get(&parent_id).is_some_and(|n| n.svg.is_some()) {
            self.sync_dom_svg_node(parent_id);
        } else if let Some(text) = restore_text
            && container.text_content().as_deref() != Some(text)
        {
            container.set_text_content(Some(text));
        }
        // Sync native top-layer `<dialog>` / `[popover]` state once children are connected.
        for &cid in new_children {
            self.sync_connected_overlay_state(cid);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_connected_overlay_state(&self, node_id: NodeId) {
        let (Some(mirror), Some(dom_el)) = (self.nodes.get(&node_id), self.dom_nodes.get(&node_id)) else {
            return;
        };
        if mirror.tag == "dialog" {
            let is_open = mirror.raw_props.get("open").is_some_and(is_truthy_raw_prop);
            let is_modal = mirror
                .raw_props
                .get("ariaModal")
                .or_else(|| mirror.raw_props.get("aria-modal"))
                .is_some_and(is_truthy_raw_prop);
            crate::semantic_attrs::sync_dialog_open_state(dom_el, is_open, is_modal);
        } else if mirror.raw_props.contains_key("popover")
            && let Some(open_val) = mirror.raw_props.get("data-nve-open")
        {
            crate::semantic_attrs::nve_sync_popover_state(dom_el, is_truthy_raw_prop(open_val));
        }
    }

    /// Writes or removes `data-nve-draw-ops` from [`crate::MirrorElement::draw_ops_attr`].
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_dom_draw_ops_attr(&mut self, node_id: NodeId) {
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        match self.nodes.get(&node_id).and_then(crate::MirrorElement::draw_ops_attr) {
            Some(ops) => {
                self.attr_scratch.clear();
                let _ = write!(&mut self.attr_scratch, "{ops}");
                set_attr_if_changed(dom_el, "data-nve-draw-ops", &self.attr_scratch);
            }
            None => remove_attr_if_present(dom_el, "data-nve-draw-ops"),
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
        // Styles, draw lists, native view props, tag-only metadata, and reserved keys are never HTML attributes.
        if raw_prop_route(key) != RawPropRoute::Attribute
            || crate::semantic_attrs::is_tag_only_raw_prop(key)
        {
            return;
        }
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        let (kind, tag, is_modal, has_popover) = self
            .nodes
            .get(&node_id)
            .map(|n| {
                (
                    n.kind,
                    n.tag,
                    n.raw_props
                        .get("ariaModal")
                        .or_else(|| n.raw_props.get("aria-modal"))
                        .is_some_and(is_truthy_raw_prop),
                    n.raw_props.contains_key("popover"),
                )
            })
            .unwrap_or((ComponentKind::View, "div", false, false));

        if tag == "dialog" && key == "open" {
            crate::semantic_attrs::sync_dialog_open_state(dom_el, is_truthy_raw_prop(value), is_modal);
            return;
        }
        if has_popover && key == "data-nve-open" {
            crate::semantic_attrs::nve_sync_popover_state(dom_el, is_truthy_raw_prop(value));
        }

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
                MappedHtmlAttr::ReadOnly(true) => set_attr_if_changed(dom_el, "readonly", ""),
                MappedHtmlAttr::ReadOnly(false) => remove_attr_if_present(dom_el, "readonly"),
            }
            return;
        }

        let attr_key = crate::semantic_attrs::canonical_html_attr_name(key).unwrap_or(key);
        let is_image = kind == ComponentKind::Image;
        match value {
            RawPropValue::Null => {
                remove_attr_if_present(dom_el, attr_key);
                if is_image && key == "uri" {
                    remove_attr_if_present(dom_el, "src");
                }
            }
            RawPropValue::Bool(true) => {
                let val = if attr_key == "popover" { "auto" } else { "" };
                set_attr_if_changed(dom_el, attr_key, val);
            }
            RawPropValue::Bool(false) => remove_attr_if_present(dom_el, attr_key),
            RawPropValue::String(s) => {
                if key == "value" {
                    #[cfg(feature = "hydrate")]
                    if self.hydrating && self.hydrated_controls.contains_key(&node_id) {
                        // Defer live DOM `.value` write to 3-way `StateReconciler` in `finalize_hydration`.
                        return;
                    }
                    self.input_sync.borrow_mut().on_backend_value_sync(node_id, s);
                    if let Some(inp) = dom_el.dyn_ref::<web_sys::HtmlInputElement>() {
                        if inp.value() != *s {
                            inp.set_value(s);
                        }
                        return;
                    }
                    if let Some(ta) = dom_el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                        if ta.value() != *s {
                            ta.set_value(s);
                        }
                        return;
                    }
                }
                if is_image && key == "uri" {
                    set_attr_if_changed(dom_el, "src", s);
                }
                set_attr_if_changed(dom_el, attr_key, s);
            }
            RawPropValue::I64(n) => {
                self.attr_scratch.clear();
                nve_abi::num::write_i64(&mut self.attr_scratch, *n);
                set_attr_if_changed(dom_el, attr_key, &self.attr_scratch);
            }
            RawPropValue::F64(n) => {
                self.attr_scratch.clear();
                nve_abi::num::write_f64(&mut self.attr_scratch, *n);
                set_attr_if_changed(dom_el, attr_key, &self.attr_scratch);
            }
            RawPropValue::Bytes(_) => {}
        }
    }

    /// Replaces a live DOM element in-place when semantic raw props upgrade its HTML5 tag
    /// (e.g. `<span>` -> `<h1>`, `<div>` -> `<nav>`, `<button>` -> `<a href="...">`).
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn upgrade_dom_element_tag(&mut self, node_id: NodeId) {
        let Some((new_tag, wrap_code)) = self.nodes.get(&node_id)
            .map(|n| (self.effective_html_tag(node_id, n).to_owned(), n.wrap_code)) else { return; };
        let Some(old_el) = self.dom_nodes.get(&node_id).cloned() else { return; };
        if old_el.tag_name().eq_ignore_ascii_case(&new_tag) { return; }
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return; };
        let Some(new_el) = create_dom_element_for_tag(&doc, &new_tag) else { return; };
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
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if let (Some(dom_el), Some(nv)) = (self.dom_nodes.get(&node_id), self.native_views.get_mut(&node_id)) {
            crate::native_view::sync_native_view_dom_attrs(dom_el, nv);
        }
        if let Some(text) = self.nodes.get(&node_id).and_then(|n| n.text.clone()) {
            self.apply_text_mutation(node_id, &text);
        }
        self.sync_dom_dir_attr(node_id);
        if self.nodes.get(&node_id).is_some_and(|n| n.svg.is_some()) {
            self.sync_dom_svg_node(node_id);
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
        let mut reachable = HashSet::with_capacity_and_hasher(self.nodes.len(), Default::default());
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
            #[cfg(any(
                feature = "nv-blur",
                feature = "nv-webview",
                feature = "nv-video",
                feature = "nv-web-element",
                feature = "camera",
            ))]
            self.native_views.remove(&nid);
            self.parent_of.remove(&nid);
            self.remove_node_event_meta(nid);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.dom_nodes.remove(&nid) {
                el.remove();
            }
        }
    }
}
