//! `wasm32` Declarative Shadow DOM atomic stream-host promotion, stylesheet hash seeding,
//! `WebBackend::hydrate_into`, and top-down structural DOM hydration walk.

use core::fmt::Write as _;

use nve_abi::hash::HashMap;
use nve_abi::{ComponentKind, NodeId, RawPropValue, ROOT_NODE_ID};
use wasm_bindgen::JsCast;

use crate::dom::create_dom_element_for_tag;
use crate::hydrate::{is_form_control_tag, HydratedControlState, HydrationMismatch};
use crate::{StyleSheetCache, WebBackend};

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_promote_dsd_stream_host(container) {
  const host = container && container.querySelector('[data-nve-dsd-host]');
  if (!host) return false;
  const doc = container.ownerDocument || document;
  // Browsers without Declarative Shadow DOM keep `<template shadowrootmode>` as an inert template.
  const tmpl = host.shadowRoot ? null : host.querySelector(':scope > template[shadowrootmode]');
  const shadow = host.shadowRoot || (tmpl && tmpl.content);
  // An element is parser-complete once the parser moved past it (or the document finished).
  const isComplete = (el) => doc.readyState !== 'loading' || el.nextSibling !== null;

  let active = doc.activeElement;
  while (active && active.shadowRoot && active.shadowRoot.activeElement) {
    active = active.shadowRoot.activeElement;
  }
  let selStart = null, selEnd = null, selDir = null;
  try {
    if (active && typeof active.selectionStart === 'number') {
      selStart = active.selectionStart;
      selEnd = active.selectionEnd;
      selDir = active.selectionDirection || 'none';
    }
  } catch (_) {}

  const scrolls = [];
  const collectScrolls = (root) => {
    if (!root || !root.querySelectorAll) return;
    const list = root.querySelectorAll('.nve-s');
    for (let i = 0; i < list.length; i++) {
      const el = list[i];
      if (el.scrollTop || el.scrollLeft) scrolls.push([el, el.scrollTop, el.scrollLeft]);
    }
  };
  collectScrolls(shadow);
  collectScrolls(host);

  const head = doc.head || doc.documentElement;
  if (head) {
    const chunkStyles = host.querySelectorAll(':scope > style[data-nve-ssr-chunk]');
    for (let i = 0; i < chunkStyles.length; i++) {
      if (isComplete(chunkStyles[i])) head.appendChild(chunkStyles[i]);
    }
  }

  const moveNode = (parent, node, refNode) => {
    if (typeof parent.moveBefore === 'function') {
      try { parent.moveBefore(node, refNode || null); return; } catch (_) {}
    }
    if (refNode) parent.insertBefore(node, refNode);
    else parent.appendChild(node);
  };

  if (shadow) {
    const slots = Array.from(shadow.querySelectorAll('slot[name^=\"nve-slot-\"]'));
    for (let i = 0; i < slots.length; i++) {
      const slot = slots[i];
      const parent = slot.parentNode;
      if (!parent) continue;
      const slotId = (slot.getAttribute('name') || '').slice('nve-slot-'.length);
      // Only a chunk directly followed by its `data-nve-slot-ack` may win (never a closed-but-
      // unacked or truncated `<div>`); an unacked chunk stays in the hidden host for the drain.
      let winner = null;
      for (let c = host.firstElementChild; c; c = c.nextElementSibling) {
        const ack = c.nextElementSibling;
        if (c.getAttribute('data-nve-slot') === slotId && ack
            && ack.getAttribute('data-nve-slot-ack') === slotId) winner = c;
      }
      if (winner) {
        while (winner.firstChild) moveNode(parent, winner.firstChild, slot);
        const done = [];
        for (let c = host.firstElementChild; c; c = c.nextElementSibling) {
          if ((c.getAttribute('data-nve-slot-ack') === slotId
              || c.getAttribute('data-nve-slot') === slotId) && isComplete(c)) done.push(c);
          // The adopted markup's slot-bound query payload is now safe to ingest.
          if (c.getAttribute('data-nve-slot-for') === slotId) c.removeAttribute('data-nve-slot-for');
        }
        for (let j = 0; j < done.length; j++) done[j].remove();
      } else {
        while (slot.firstChild) moveNode(parent, slot.firstChild, slot);
      }
      slot.remove();
    }

    const rootEl = shadow.querySelector('[data-nve-id=\"1\"]');
    if (rootEl && rootEl.parentNode === shadow) {
      moveNode(container, rootEl, host);
    } else {
      const kids = Array.from(shadow.children);
      for (let i = 0; i < kids.length; i++) {
        const k = kids[i];
        const t = k.tagName;
        if (t === 'STYLE' || t === 'SCRIPT' || t === 'TEMPLATE') continue;
        moveNode(container, k, host);
      }
    }
    if (tmpl) tmpl.remove();
  }

  // Keep the (hidden) host while the stream is open or it still holds unconsumed payloads,
  // unhoisted chunk styles, or unfinished elements for the mid-stream drain.
  let pending = !host.querySelector(':scope > template[data-nve-stream-end]');
  for (let c = host.firstElementChild; c && !pending; c = c.nextElementSibling) {
    pending = !isComplete(c)
      || c.matches('script[type=\"application/x-nve-query\"],style[data-nve-ssr-chunk],[data-nve-slot]');
  }
  if (pending) host.setAttribute('hidden', '');
  else host.remove();

  for (let i = 0; i < scrolls.length; i++) {
    const [el, top, left] = scrolls[i];
    if (el.isConnected) { el.scrollTop = top; el.scrollLeft = left; }
  }
  if (active && active.isConnected && doc.activeElement !== active && typeof active.focus === 'function') {
    try { active.focus({ preventScroll: true }); } catch (_) { try { active.focus(); } catch (_) {} }
    if (selStart !== null && typeof active.setSelectionRange === 'function') {
      try { active.setSelectionRange(selStart, selEnd, selDir); } catch (_) {}
    }
  }
  return true;
}
")]
extern "C" {
    fn nve_promote_dsd_stream_host(container: &web_sys::Element) -> bool;
}

fn seed_all_ssr_styles(container: &web_sys::Element, stylesheet: &mut StyleSheetCache) -> usize {
    let mut seeded = 0usize;
    let seed_list = |list: web_sys::NodeList, sheet: &mut StyleSheetCache, count: &mut usize| {
        let len = list.length();
        for i in 0..len {
            if let Some(node) = list.item(i)
                && let Ok(el) = node.dyn_into::<web_sys::Element>()
                && let Some(hashes) = el.get_attribute("data-nve-hashes")
            {
                *count += sheet.seed_ssr_hashes(&hashes);
            }
        }
    };
    const SEL: &str = "style[data-nve-ssr],style[data-nve-ssr-chunk]";
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if doc.query_selector("style[data-nve-base]").ok().flatten().is_some() {
            stylesheet.mark_base_mounted();
        }
        if let Ok(list) = doc.query_selector_all(SEL) {
            seed_list(list, stylesheet, &mut seeded);
        }
    }
    if container
        .query_selector("style[data-nve-base]")
        .ok()
        .flatten()
        .is_some()
    {
        stylesheet.mark_base_mounted();
    }
    if let Ok(list) = container.query_selector_all(SEL) {
        seed_list(list, stylesheet, &mut seeded);
    }
    if let Ok(Some(host)) = container.query_selector("[data-nve-dsd-host]")
        && let Some(shadow) = host.shadow_root()
    {
        if shadow
            .query_selector("style[data-nve-base]")
            .ok()
            .flatten()
            .is_some()
        {
            stylesheet.mark_base_mounted();
        }
        if let Ok(list) = shadow.query_selector_all(SEL) {
            seed_list(list, stylesheet, &mut seeded);
        }
    }
    seeded
}

#[inline]
fn is_ignored_dom_element(el: &web_sys::Element) -> bool {
    let tag = el.tag_name();
    tag.eq_ignore_ascii_case("style")
        || tag.eq_ignore_ascii_case("script")
        || tag.eq_ignore_ascii_case("template")
        || el.has_attribute("data-nve-dsd-host")
}

fn find_hydrate_root(container: &web_sys::Element) -> web_sys::Element {
    if container.has_attribute("data-nve-id") {
        return container.clone();
    }
    let mut first_ui: Option<web_sys::Element> = None;
    let mut cur = container.first_element_child();
    while let Some(el) = cur {
        if !is_ignored_dom_element(&el) {
            if el.get_attribute("data-nve-id").as_deref() == Some("1") {
                return el;
            }
            if first_ui.is_none() {
                first_ui = Some(el.clone());
            }
        }
        cur = el.next_element_sibling();
    }
    if let Some(el) = first_ui {
        return el;
    }
    let _ = container.set_attribute("data-nve-id", "1");
    container.clone()
}

fn collect_ui_dom_children(
    dom_el: &web_sys::Element,
    wrap_code: bool,
    mirror_tag: &str,
    mirror_has_children: bool,
) -> (web_sys::Element, Vec<web_sys::Element>) {
    let mount_target = if wrap_code
        && let Some(first) = dom_el.first_element_child()
        && first.tag_name().eq_ignore_ascii_case("code")
        && !first.has_attribute("data-nve-id")
    {
        first
    } else {
        dom_el.clone()
    };
    if mirror_tag.eq_ignore_ascii_case("svg") && !mirror_has_children {
        return (mount_target, Vec::new());
    }
    let mut children = Vec::new();
    let mut cur = mount_target.first_element_child();
    while let Some(el) = cur {
        if !is_ignored_dom_element(&el)
            && (mirror_has_children
                || !(el.tag_name().eq_ignore_ascii_case("svg") && !el.has_attribute("data-nve-id")))
        {
            children.push(el.clone());
        }
        cur = el.next_element_sibling();
    }
    (mount_target, children)
}

fn read_dom_control_base(el: &web_sys::Element, tag: &str) -> String {
    match tag {
        "input" => el.get_attribute("value").unwrap_or_default(),
        "textarea" => el.text_content().unwrap_or_default(),
        "select" => el
            .query_selector("option[selected]")
            .ok()
            .flatten()
            .and_then(|o| o.get_attribute("value").or_else(|| o.text_content()))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn read_dom_control_value(el: &web_sys::Element) -> Option<String> {
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        return Some(inp.value());
    }
    if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        return Some(ta.value());
    }
    if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>() {
        return Some(sel.value());
    }
    None
}

pub(crate) fn write_dom_control_value(el: &web_sys::Element, val: &str) {
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        if inp.value() != val {
            inp.set_value(val);
        }
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        if ta.value() != val {
            ta.set_value(val);
        }
    } else if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>()
        && sel.value() != val
    {
        sel.set_value(val);
    }
}

impl WebBackend {
    /// Zero-flicker browser DOM hydration: seeds `StyleSheetCache` from `data-nve-base`,
    /// `data-nve-ssr`, and all `data-nve-ssr-chunk` stylesheets, atomically promotes any
    /// ephemeral `[data-nve-dsd-host]` shadow root into `#app`, stores `hydrate_root` for
    /// top-down structural matching in `finalize_hydration`, attaches root delegated listeners,
    /// and drains `window.__nveReplayDrain()`.
    pub fn hydrate_into(container: web_sys::Element) -> Self {
        let mut backend = Self::new();
        let seeded_rule_count = seed_all_ssr_styles(&container, &mut backend.stylesheet);
        let _ = nve_promote_dsd_stream_host(&container);
        let root_el = find_hydrate_root(&container);
        if let Some(hash) = root_el.get_attribute("data-nve-ssr-hash") {
            backend.hydration_report.ssr_hash = Some(hash);
        }
        backend.hydrate_root = Some(root_el);
        backend.hydrating = true;
        backend.hydration_report.hydrated = true;
        backend.hydration_report.seeded_rule_count = seeded_rule_count;

        backend.attach_root_event_delegator(container);
        backend.attach_window_keydown_listener();
        backend.attach_visual_viewport_listener();
        backend.pending_pre_hydration_events =
            crate::event_replay::drain_browser_pre_hydration_events();
        backend
    }

    pub(crate) fn hydrate_walk_wasm(
        &mut self,
        ssr_id_remap: &mut HashMap<NodeId, NodeId>,
        captured_controls: &mut Vec<(NodeId, HydratedControlState)>,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let Some(root_el) = self.hydrate_root.take() else {
            return;
        };
        let mut stack: Vec<(NodeId, web_sys::Element, Option<web_sys::Element>)> =
            vec![(ROOT_NODE_ID, root_el, None)];

        while let Some((mirror_id, dom_el, parent_mount)) = stack.pop() {
            let Some(el) = self.nodes.get(&mirror_id) else {
                continue;
            };
            let expected_eff_tag = self.effective_html_tag(mirror_id, el).to_owned();
            let expected_static_tag = el.tag;
            let wrap_code = el.wrap_code;
            let mirror_children = el.children.clone();
            let actual_tag = dom_el.tag_name().to_ascii_lowercase();

            if actual_tag.eq_ignore_ascii_case(&expected_eff_tag) {
                self.hydration_report.adopted_nodes += 1;
                let old_ssr_id = dom_el
                    .get_attribute("data-nve-id")
                    .and_then(|raw| raw.parse::<NodeId>().ok());
                if let Some(old_id) = old_ssr_id {
                    ssr_id_remap.insert(old_id, mirror_id);
                }
                if old_ssr_id != Some(mirror_id) {
                    self.attr_scratch.clear();
                    let _ = write!(&mut self.attr_scratch, "{mirror_id}");
                    let _ = dom_el.set_attribute("data-nve-id", &self.attr_scratch);
                }

                let existing_class = dom_el.class_name();
                let existing_inline = dom_el.get_attribute("style").unwrap_or_default();
                let has_inline = !existing_inline.trim().is_empty();
                if let Some(m) = self.nodes.get_mut(&mirror_id) {
                    m.class_name = existing_class;
                    m.has_inline_style = has_inline;
                    m.inline_style = if has_inline { existing_inline } else { String::new() };
                }

                self.dom_nodes.insert(mirror_id, dom_el.clone());

                let is_control = is_form_control_tag(&expected_eff_tag);
                if is_control {
                    let base = read_dom_control_base(&dom_el, &expected_eff_tag);
                    let local = read_dom_control_value(&dom_el).unwrap_or_else(|| base.clone());
                    captured_controls.push((mirror_id, HydratedControlState { base, local }));
                }
                self.sync_adopted_wasm_element(mirror_id, is_control);

                let (mount_target, dom_children) = collect_ui_dom_children(
                    &dom_el,
                    wrap_code,
                    &expected_eff_tag,
                    !mirror_children.is_empty(),
                );
                let paired = mirror_children.len().min(dom_children.len());
                for i in (0..paired).rev() {
                    stack.push((
                        mirror_children[i],
                        dom_children[i].clone(),
                        Some(mount_target.clone()),
                    ));
                }
                for &missing_id in &mirror_children[paired..] {
                    let missing_tag = self.nodes.get(&missing_id).map_or("div", |n| n.tag);
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id: missing_id,
                        expected_tag: missing_tag,
                        actual_tag: "<missing>".into(),
                    });
                    self.hydration_report.replaced_nodes += 1;
                    self.mount_wasm_subtree_from_mirror(missing_id, &mount_target, None, dirty_styles);
                }
                for extra in &dom_children[paired..] {
                    self.hydration_report.mismatches.push(HydrationMismatch {
                        node_id: mirror_id,
                        expected_tag: "",
                        actual_tag: extra.tag_name().to_ascii_lowercase(),
                    });
                    extra.remove();
                }
            } else {
                self.hydration_report.mismatches.push(HydrationMismatch {
                    node_id: mirror_id,
                    expected_tag: expected_static_tag,
                    actual_tag,
                });
                self.hydration_report.replaced_nodes += 1;
                if let Some(ref p_mount) = parent_mount {
                    self.mount_wasm_subtree_from_mirror(
                        mirror_id,
                        p_mount,
                        Some(&dom_el),
                        dirty_styles,
                    );
                } else {
                    self.dom_nodes.insert(ROOT_NODE_ID, dom_el.clone());
                    dom_el.set_text_content(None);
                    for &cid in &mirror_children {
                        self.mount_wasm_subtree_from_mirror(cid, &dom_el, None, dirty_styles);
                    }
                }
            }
        }
    }

    fn sync_adopted_wasm_element(&mut self, node_id: NodeId, defer_control_value: bool) {
        let Some((props, text, wrap_code, has_children, has_svg, is_input)) =
            self.nodes.get(&node_id).map(|n| {
                (
                    n.raw_props.clone(),
                    n.text.clone(),
                    n.wrap_code,
                    !n.children.is_empty(),
                    n.svg.is_some(),
                    n.kind == ComponentKind::TextInput,
                )
            })
        else {
            return;
        };
        for (k, v) in &props {
            if defer_control_value && k == "value" {
                continue;
            }
            self.sync_dom_raw_prop(node_id, k, v);
        }
        #[cfg(any(
            feature = "nv-blur",
            feature = "nv-webview",
            feature = "nv-video",
            feature = "nv-web-element",
            feature = "camera",
        ))]
        if let (Some(dom_el), Some(nv)) =
            (self.dom_nodes.get(&node_id), self.native_views.get_mut(&node_id))
        {
            crate::native_view::sync_native_view_dom_attrs(dom_el, nv);
        }
        if !has_children
            && !has_svg
            && !is_input
            && let Some(t) = text.as_deref()
            && let Some(dom_el) = self.dom_nodes.get(&node_id)
        {
            let target = self.text_or_child_mount_target(dom_el, wrap_code);
            if target.text_content().as_deref() != Some(t) {
                target.set_text_content(Some(t));
            }
        }
        self.sync_connected_overlay_state(node_id);
    }

    fn mount_wasm_subtree_from_mirror(
        &mut self,
        root_id: NodeId,
        parent_mount: &web_sys::Element,
        replace_el: Option<&web_sys::Element>,
        dirty_styles: &mut Vec<NodeId>,
    ) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let mut stack: Vec<(NodeId, web_sys::Element, Option<web_sys::Element>)> =
            vec![(root_id, parent_mount.clone(), replace_el.cloned())];

        while let Some((nid, p_mount, old_el)) = stack.pop() {
            let Some((eff_tag, wrap_code, children, kind, has_svg, has_dir)) =
                self.nodes.get(&nid).map(|n| {
                    (
                        self.effective_html_tag(nid, n).to_owned(),
                        n.wrap_code,
                        n.children.clone(),
                        n.kind,
                        n.svg.is_some(),
                        crate::layout_ext_css::node_dir_attr(n.layout_ext.as_deref()).is_some(),
                    )
                })
            else {
                continue;
            };
            let Some(new_el) = create_dom_element_for_tag(&doc, &eff_tag) else {
                continue;
            };
            self.attr_scratch.clear();
            let _ = write!(&mut self.attr_scratch, "{nid}");
            let _ = new_el.set_attribute("data-nve-id", &self.attr_scratch);

            let child_mount = if wrap_code {
                if let Ok(code_el) = doc.create_element("code") {
                    let _ = new_el.append_child(&code_el);
                    code_el
                } else {
                    new_el.clone()
                }
            } else {
                new_el.clone()
            };

            if let Some(old) = old_el {
                if let Some(p) = old.parent_node() {
                    let _ = p.replace_child(&new_el, &old);
                } else {
                    let _ = p_mount.append_child(&new_el);
                }
            } else {
                let _ = p_mount.append_child(&new_el);
            }

            self.dom_nodes.insert(nid, new_el);
            if let Some(m) = self.nodes.get_mut(&nid) {
                m.class_name.clear();
                m.inline_style.clear();
                m.has_inline_style = false;
            }
            self.sync_adopted_wasm_element(nid, false);
            if kind == ComponentKind::TextInput
                && let Some(val) = self.nodes.get(&nid).and_then(|n| {
                    match n.raw_props.get("value") {
                        Some(RawPropValue::String(v)) => Some(v.clone()),
                        _ => n.text.clone(),
                    }
                })
                && let Some(dom_el) = self.dom_nodes.get(&nid)
            {
                write_dom_control_value(dom_el, &val);
            }
            dirty_styles.push(nid);
            if has_dir {
                self.frame_dirty_dirs.push(nid);
            }
            if has_svg {
                self.frame_dirty_svgs.push(nid);
            }
            if kind == ComponentKind::SkiaCanvas {
                self.frame_dirty_canvases.push(nid);
            }
            for cid in children.into_iter().rev() {
                stack.push((cid, child_mount.clone(), None));
            }
        }
    }
}
