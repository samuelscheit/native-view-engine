//! `wasm32` Declarative Shadow DOM atomic stream-host promotion, SSR stylesheet hash seeding,
//! `WebBackend::hydrate_into`, and live-DOM positional pairing (`crate::hydrate::pair_ssr_dom`).

use nve_abi::NodeId;
use nve_store::{NodeId as StoreNodeId, NodeStore};
use wasm_bindgen::JsCast;

use crate::hydrate::{pair_ssr_dom, HydratedControlState, HydrationPass, HydrationReport, SsrDomTree};
use crate::input_attrs::{read_dom_control_value, write_dom_control_value};
use crate::{StyleSheetCache, WebBackend};

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function nve_promote_dsd_stream_host(container) {
  const host = container && container.querySelector('[data-nve-dsd-host]');
  if (!host) return false;
  const doc = container.ownerDocument || document;
  const shadow = host.shadowRoot;

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
    const chunkStyles = host.querySelectorAll('style[data-nve-ssr-chunk]');
    for (let i = 0; i < chunkStyles.length; i++) head.appendChild(chunkStyles[i]);
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
      let winner = null;
      const matches = [];
      for (let c = host.firstElementChild; c; c = c.nextElementSibling) {
        if (c.getAttribute('data-nve-slot') === slotId) { matches.push(c); winner = c; }
      }
      if (winner) {
        while (winner.firstChild) moveNode(parent, winner.firstChild, slot);
        for (let j = 0; j < matches.length; j++) matches[j].remove();
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
  }

  if (host.querySelector('template[data-nve-stream-end]') && !host.querySelector('script[type=\"application/x-nve-query\"]')) host.remove();
  else host.setAttribute('hidden', '');

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
    const SEL: &str = "style[data-nve-ssr],style[data-nve-ssr-chunk]";
    let before = stylesheet.rule_count();
    let mut seed_list = |list: web_sys::NodeList| {
        for i in 0..list.length() {
            if let Some(hashes) = list
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                .and_then(|el| el.get_attribute("data-nve-hashes"))
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
    el.get_attribute("data-nve-id")?.parse().ok()
}

/// The promoted SSR root element: `container` itself or its UI child carrying the root
/// `data-nve-id`, else its first UI child carrying any `data-nve-id`.
fn find_hydrate_root(container: &web_sys::Element) -> Option<web_sys::Element> {
    // Mounts whose container IS the SSR root (`<div id="app" data-nve-id="1">`).
    if parse_ssr_id(container) == Some(StoreNodeId::ROOT.raw()) {
        return Some(container.clone());
    }
    let mut first_ui: Option<web_sys::Element> = None;
    let mut cur = container.first_element_child();
    while let Some(el) = cur {
        if !is_ignored_dom_element(&el) && let Some(id) = parse_ssr_id(&el) {
            if id == StoreNodeId::ROOT.raw() {
                return Some(el);
            }
            first_ui.get_or_insert(el.clone());
        }
        cur = el.next_element_sibling();
    }
    first_ui
}

/// Server-rendered `<select>` value: the last `<option selected>`'s `value` (else its text),
/// mirroring `hydrate_native_parse` so native and browser hydration reconcile the same base.
fn ssr_select_base_value(select: &web_sys::Element) -> String {
    let Ok(list) = select.query_selector_all("option[selected]") else {
        return String::new();
    };
    list.length()
        .checked_sub(1)
        .and_then(|last| list.item(last))
        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        .map(|opt| opt.get_attribute("value").or_else(|| opt.text_content()).unwrap_or_default())
        .unwrap_or_default()
}

/// Detaches a stale SSR root that is the mount container itself: keeps `#app` connected and its
/// `id`, drops its server UI children and SSR identity so the client root rebuilds inside it.
fn demote_mount_root(mount: &web_sys::Element) {
    let mut cur = mount.first_element_child();
    while let Some(el) = cur {
        cur = el.next_element_sibling();
        if !is_ignored_dom_element(&el) {
            el.remove();
        }
    }
    let _ = mount.remove_attribute("data-nve-id");
    let _ = mount.remove_attribute("data-nve-ssr-hash");
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
            "select" => ssr_select_base_value(el),
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
            let raw = node.raw();
            if parse_ssr_id(&el) != Some(raw) {
                let _ = el.set_attribute("data-nve-id", &raw.to_string());
            }
            self.live_elements.insert(raw, el);
        }
        self.deferred_controls
            .extend(pairing.pass.controls.iter().map(|(n, _)| n.raw()));
        Some(pairing.pass)
    }

    pub(crate) fn write_hydrated_control(&mut self, node: StoreNodeId, value: &str) {
        self.deferred_controls.remove(&node.raw());
        if let Some(el) = self.live_elements.get(&node.raw()) {
            write_dom_control_value(el, value);
        }
    }
}
