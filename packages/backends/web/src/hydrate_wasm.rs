//! `wasm32` Declarative Shadow DOM atomic stream-host promotion, SSR stylesheet hash seeding,
//! `WebBackend::hydrate_into`, and live-DOM positional pairing (`crate::hydrate::pair_ssr_dom`).

use nve_abi::NodeId;
use nve_store::NodeStore;
use wasm_bindgen::JsCast;

use crate::hydrate::{
    pair_ssr_dom, ssr_select_base_value, HydratedControlState, HydrationPass, HydrationReport,
    SsrDomTree, SsrSelectOption,
};
use crate::input_attrs::{read_dom_control_value, write_dom_control_value};
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
  // A chunk `<style>` is always followed by its `<div slot>`, so only a next sibling proves its
  // CSS is whole: a stream truncated mid-`<style>` must never hoist (or seed) partial rules.
  const isChunkStyleComplete = (el) => el.nextSibling !== null;

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
      if (isChunkStyleComplete(chunkStyles[i])) head.appendChild(chunkStyles[i]);
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

/// Whether an SSR `<style>`'s rules are whole. A `data-nve-ssr-chunk` style still inside the
/// `[data-nve-dsd-host]` is whole only once its `<div slot>` follows it (the server always emits
/// the chunk `<style>` strictly before the slot markup); a truncated one is never seeded, so the
/// client re-inserts those rules itself.
fn is_ssr_style_complete(el: &web_sys::Element) -> bool {
    !el.has_attribute("data-nve-ssr-chunk")
        || el.next_sibling().is_some()
        || !el
            .parent_element()
            .is_some_and(|p| p.has_attribute("data-nve-dsd-host"))
}

fn seed_all_ssr_styles(container: &web_sys::Element, stylesheet: &mut StyleSheetCache) -> usize {
    const SEL: &str = "style[data-nve-ssr],style[data-nve-ssr-chunk]";
    let before = stylesheet.rule_count();
    let mut seed_list = |list: web_sys::NodeList| {
        for i in 0..list.length() {
            if let Some(el) = list
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                && is_ssr_style_complete(&el)
                && let Some(hashes) = el.get_attribute("data-nve-hashes")
            {
                stylesheet.seed_ssr_hashes(hashes.split(','));
            }
        }
    };
    if let Some(Ok(list)) = web_sys::window()
        .and_then(|w| w.document())
        .map(|d| d.query_selector_all(SEL))
    {
        seed_list(list);
    }
    if let Ok(list) = container.query_selector_all(SEL) {
        seed_list(list);
    }
    if let Some(Ok(list)) = container
        .query_selector("[data-nve-dsd-host]")
        .ok()
        .flatten()
        .and_then(|host| host.shadow_root())
        .map(|shadow| shadow.query_selector_all(SEL))
    {
        seed_list(list);
    }
    stylesheet.rule_count().saturating_sub(before)
}

#[inline]
fn is_ignored_dom_element(el: &web_sys::Element) -> bool {
    let tag = el.tag_name();
    tag.eq_ignore_ascii_case("style")
        || tag.eq_ignore_ascii_case("script")
        || tag.eq_ignore_ascii_case("template")
        || el.has_attribute("data-nve-dsd-host")
}

fn parse_ssr_id(el: &web_sys::Element) -> Option<NodeId> {
    crate::key_events::parse_dom_node_id(&el.get_attribute("data-nve-id")?)
}

/// The promoted SSR root element: `container` itself or its UI child carrying the root
/// `data-nve-id`, else its first UI child carrying any `data-nve-id`.
fn find_hydrate_root(container: &web_sys::Element) -> Option<web_sys::Element> {
    // Mounts whose container IS the SSR root (`<div id="app" data-nve-id="1">`).
    if parse_ssr_id(container) == Some(NodeId::ROOT) {
        return Some(container.clone());
    }
    let mut first_ui: Option<web_sys::Element> = None;
    let mut cur = container.first_element_child();
    while let Some(el) = cur {
        if !is_ignored_dom_element(&el) && let Some(id) = parse_ssr_id(&el) {
            if id == NodeId::ROOT {
                return Some(el);
            }
            first_ui.get_or_insert(el.clone());
        }
        cur = el.next_element_sibling();
    }
    first_ui
}

/// Server-rendered `<select>` value from its markup alone (shared selectedness rules in
/// [`crate::hydrate::ssr_select_base_value`], identical to `hydrate_native_parse`).
fn select_markup_value(select: &web_sys::Element) -> String {
    let mut options: Vec<SsrSelectOption> = Vec::new();
    let mut push = |opt: &web_sys::Element, group_disabled: bool| {
        options.push(SsrSelectOption {
            selected: opt.has_attribute("selected"),
            disabled: group_disabled || opt.has_attribute("disabled"),
            value: opt.get_attribute("value"),
            text: opt.text_content().unwrap_or_default(),
        });
    };
    let mut cur = select.first_element_child();
    while let Some(child) = cur {
        let tag = child.tag_name();
        if tag.eq_ignore_ascii_case("option") {
            push(&child, false);
        } else if tag.eq_ignore_ascii_case("optgroup") {
            let group_disabled = child.has_attribute("disabled");
            let mut opt = child.first_element_child();
            while let Some(o) = opt {
                if o.tag_name().eq_ignore_ascii_case("option") {
                    push(&o, group_disabled);
                }
                opt = o.next_element_sibling();
            }
        }
        cur = child.next_element_sibling();
    }
    ssr_select_base_value(
        &options,
        select.has_attribute("multiple"),
        select.get_attribute("size").as_deref(),
    )
}

/// Detaches a stale SSR root that is the mount container itself: keeps `#app` connected and its
/// `id`, drops its server content (text and UI children, keeping styles/scripts/templates/DSD
/// hosts), SSR identity, and SSR `class`/`style` so the client root rebuilds inside it.
fn demote_mount_root(mount: &web_sys::Element) {
    let mut cur = mount.first_child();
    while let Some(node) = cur {
        cur = node.next_sibling();
        let keep = node
            .dyn_ref::<web_sys::Element>()
            .is_some_and(is_ignored_dom_element);
        if !keep && let Some(parent) = node.parent_node() {
            let _ = parent.remove_child(&node);
        }
    }
    for attr in ["data-nve-id", "data-nve-ssr-hash", "class", "style"] {
        let _ = mount.remove_attribute(attr);
    }
}

/// Live browser DOM view for positional pairing.
struct LiveDom;

impl SsrDomTree for LiveDom {
    type El = web_sys::Element;

    fn tag(&self, el: &web_sys::Element) -> String {
        let mut tag = el.tag_name();
        tag.make_ascii_lowercase();
        tag
    }

    fn ssr_id(&self, el: &web_sys::Element) -> Option<NodeId> {
        parse_ssr_id(el)
    }

    fn ui_children(&self, el: &web_sys::Element) -> Vec<web_sys::Element> {
        let mut out = Vec::new();
        let mut cur = el.first_element_child();
        while let Some(child) = cur {
            if !is_ignored_dom_element(&child) {
                out.push(child.clone());
            }
            cur = child.next_element_sibling();
        }
        out
    }

    fn control_state(&self, el: &web_sys::Element, tag: &str) -> HydratedControlState {
        let base = match tag {
            "textarea" => el.text_content().unwrap_or_default(),
            "select" => select_markup_value(el),
            _ => el.get_attribute("value").unwrap_or_default(),
        };
        let local = read_dom_control_value(el).unwrap_or_else(|| base.clone());
        HydratedControlState { base, local }
    }
}

impl WebBackend {
    /// Zero-flicker browser DOM hydration: seeds `StyleSheetCache` from every `data-nve-ssr` and
    /// `data-nve-ssr-chunk` stylesheet, atomically promotes any ephemeral `[data-nve-dsd-host]`
    /// shadow root into `container`, stages the SSR root for positional pairing on the first
    /// `commit()`, attaches the root event delegator, and drains `window.__nveReplayDrain()`.
    pub fn hydrate_into(container: web_sys::Element) -> Self {
        let mut backend = Self::attach_into(container.clone(), false);
        let seeded = seed_all_ssr_styles(&container, &mut backend.stylesheet);
        let _ = nve_promote_dsd_stream_host(&container);
        let root = find_hydrate_root(&container);
        let root_hash = root.as_ref().and_then(|r| r.get_attribute("data-nve-ssr-hash"));
        backend.hydration_report = HydrationReport::begin(seeded, root_hash.as_deref());
        backend.hydrate_root = root;
        backend
            .pre_hydration_events
            .extend(crate::event_replay::drain_browser_pre_hydration_events());
        backend
    }

    /// Pairs the staged SSR root against `store` (first hydrated commit only): adopts matching
    /// elements under client `NodeId`s, discards stale SSR elements, and defers adopted control
    /// `.value` writes until 3-way reconciliation.
    pub(crate) fn pair_pending_hydration(&mut self, store: &NodeStore) -> Option<HydrationPass> {
        let root = self.hydrate_root.take()?;
        let pairing = pair_ssr_dom(
            &LiveDom,
            root,
            store,
            self.renderer.registry(),
            &mut self.hydration_report,
        );
        for el in pairing.stale {
            if self.mount_root.as_ref() == Some(&el) {
                demote_mount_root(&el);
            } else {
                el.remove();
            }
        }
        for (node, el) in pairing.adopted {
            if parse_ssr_id(&el) != Some(node) {
                let _ = el.set_attribute("data-nve-id", &node.raw().to_string());
            }
            self.live_elements.insert(node, el);
        }
        self.deferred_controls
            .extend(pairing.pass.controls.iter().map(|(n, _)| *n));
        Some(pairing.pass)
    }

    pub(crate) fn write_hydrated_control(&mut self, node: NodeId, value: &str) {
        self.deferred_controls.remove(&node);
        if let Some(el) = self.live_elements.get(&node) {
            write_dom_control_value(el, value);
        }
    }
}
