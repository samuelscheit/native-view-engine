//! Per-commit mutable context ([`WebSystemCommitCtx`]) passed into [`super::WebTableSystem`] hooks.

use nve_abi::hash::HashMap;
use nve_store::{EventBatchBuf, NodeId, TableSlot};

use crate::table_system::WebTableRegistry;

/// Attribute value owned by a single `(NodeId, TableSlot)` tagged with its global write sequence.
pub(crate) struct OwnedAttr {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) seq: u64,
}

/// Inline CSS string owned by a single `(NodeId, TableSlot)` tagged with its global write sequence.
pub(crate) struct OwnedInlineCss {
    pub(crate) css: String,
    pub(crate) seq: u64,
}

/// Context passed into [`super::WebTableSystem`] hooks during `commit` and command dispatch.
pub struct WebSystemCommitCtx<'a> {
    pub attrs: &'a mut HashMap<NodeId, Vec<(String, String)>>,
    pub inner_html: &'a mut HashMap<NodeId, String>,
    pub inline_css: &'a mut HashMap<NodeId, String>,
    pub events: &'a mut EventBatchBuf,
    pub(crate) current_slot: TableSlot,
    pub(crate) registry: &'a WebTableRegistry,
    pub(crate) slot_attrs: &'a mut HashMap<(NodeId, TableSlot), Vec<OwnedAttr>>,
    pub(crate) inner_html_owner: &'a mut HashMap<NodeId, TableSlot>,
    pub(crate) slot_inline_css: &'a mut HashMap<(NodeId, TableSlot), OwnedInlineCss>,
    pub(crate) write_seq: &'a mut u64,
    #[cfg(target_arch = "wasm32")]
    pub live_element: Option<&'a web_sys::Element>,
}

impl WebSystemCommitCtx<'_> {
    #[inline]
    fn next_seq(&mut self) -> u64 {
        *self.write_seq += 1;
        *self.write_seq
    }

    #[inline]
    fn find_latest_other_slot_attr(&self, node: NodeId, name: &str) -> Option<(String, u64)> {
        let mut best: Option<(&str, u64)> = None;
        for sys in self.registry.iter() {
            let slot = sys.slot();
            if slot == self.current_slot {
                continue;
            }
            if let Some(list) = self.slot_attrs.get(&(node, slot))
                && let Some(entry) = list.iter().find(|e| e.name == name)
                && best.is_none_or(|(_, best_seq)| entry.seq > best_seq)
            {
                best = Some((entry.value.as_str(), entry.seq));
            }
        }
        best.map(|(v, seq)| (v.to_owned(), seq))
    }

    #[inline]
    fn find_latest_other_slot_inline_css(&self, node: NodeId) -> Option<(String, u64)> {
        let mut best: Option<(&str, u64)> = None;
        for sys in self.registry.iter() {
            let slot = sys.slot();
            if slot == self.current_slot {
                continue;
            }
            if let Some(entry) = self.slot_inline_css.get(&(node, slot))
                && !entry.css.is_empty()
                && best.is_none_or(|(_, best_seq)| entry.seq > best_seq)
            {
                best = Some((entry.css.as_str(), entry.seq));
            }
        }
        best.map(|(css, seq)| (css.to_owned(), seq))
    }

    #[inline]
    fn write_effective_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let list = self.attrs.entry(node).or_default();
        if let Some((_, v)) = list.iter_mut().find(|(k, _)| k == name) {
            v.clear();
            v.push_str(value);
        } else {
            list.push((name.to_owned(), value.to_owned()));
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(el) = self.live_element {
            let _ = el.set_attribute(name, value);
        }
    }

    #[inline]
    fn restore_or_remove_attr(&mut self, node: NodeId, name: &str, removed_seq: u64) {
        match self.find_latest_other_slot_attr(node, name) {
            Some((_, max_seq)) if max_seq > removed_seq => {
                // Another plugin wrote this attribute more recently; leave live DOM & mirror intact.
            }
            Some((fallback, _)) => {
                self.write_effective_attr(node, name, &fallback);
            }
            None => {
                if let Some(list) = self.attrs.get_mut(&node) {
                    list.retain(|(k, _)| k != name);
                    if list.is_empty() {
                        self.attrs.remove(&node);
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if let Some(el) = self.live_element {
                    let _ = el.remove_attribute(name);
                }
            }
        }
    }

    #[inline]
    fn restore_or_remove_inline_css(&mut self, node: NodeId, removed_seq: u64) {
        match self.find_latest_other_slot_inline_css(node) {
            Some((_, max_seq)) if max_seq > removed_seq => {
                // Another plugin wrote inline CSS more recently; leave live DOM & mirror intact.
            }
            Some((fallback, _)) => {
                #[cfg(target_arch = "wasm32")]
                if let Some(el) = self.live_element {
                    let _ = el.set_attribute("data-nve-mod-style", &fallback);
                }
                self.inline_css.insert(node, fallback);
            }
            None => {
                self.inline_css.remove(&node);
                #[cfg(target_arch = "wasm32")]
                if let Some(el) = self.live_element {
                    let _ = el.remove_attribute("data-nve-mod-style");
                }
            }
        }
    }

    pub fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let seq = self.next_seq();
        self.write_effective_attr(node, name, value);
        let owned = self.slot_attrs.entry((node, self.current_slot)).or_default();
        if let Some(entry) = owned.iter_mut().find(|e| e.name == name) {
            entry.value.clear();
            entry.value.push_str(value);
            entry.seq = seq;
        } else {
            owned.push(OwnedAttr {
                name: name.to_owned(),
                value: value.to_owned(),
                seq,
            });
        }
    }

    pub fn remove_attr(&mut self, node: NodeId, name: &str) {
        let removed_seq = {
            let Some(owned) = self.slot_attrs.get_mut(&(node, self.current_slot)) else {
                return;
            };
            let Some(pos) = owned.iter().position(|e| e.name == name) else {
                return;
            };
            let removed = owned.swap_remove(pos);
            if owned.is_empty() {
                self.slot_attrs.remove(&(node, self.current_slot));
            }
            removed.seq
        };
        self.restore_or_remove_attr(node, name, removed_seq);
    }

    /// Removes every attribute previously written by the current plugin slot on `node` for which
    /// `keep(attr_name)` returns `false`, restoring any value still owned by another plugin slot.
    pub fn retain_slot_attrs(&mut self, node: NodeId, mut keep: impl FnMut(&str) -> bool) {
        let mut to_remove: Vec<(String, u64)> = Vec::new();
        let mut empty = false;
        if let Some(owned) = self.slot_attrs.get_mut(&(node, self.current_slot)) {
            let mut i = 0;
            while i < owned.len() {
                if keep(&owned[i].name) {
                    i += 1;
                } else {
                    let removed = owned.swap_remove(i);
                    to_remove.push((removed.name, removed.seq));
                }
            }
            empty = owned.is_empty();
        }
        if empty {
            self.slot_attrs.remove(&(node, self.current_slot));
        }
        for (removed, seq) in to_remove {
            self.restore_or_remove_attr(node, &removed, seq);
        }
    }

    pub fn set_inner_html(&mut self, node: NodeId, html: String) {
        #[cfg(target_arch = "wasm32")]
        if let Some(el) = self.live_element {
            el.set_inner_html(&html);
        }
        self.inner_html.insert(node, html);
        self.inner_html_owner.insert(node, self.current_slot);
    }

    pub fn clear_inner_html(&mut self, node: NodeId) {
        if self.inner_html_owner.get(&node) == Some(&self.current_slot) {
            self.inner_html_owner.remove(&node);
            self.inner_html.remove(&node);
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.live_element {
                el.set_inner_html("");
            }
        }
    }

    pub fn set_inline_css(&mut self, node: NodeId, css: String) {
        if css.is_empty() {
            let Some(removed) = self.slot_inline_css.remove(&(node, self.current_slot)) else {
                return;
            };
            self.restore_or_remove_inline_css(node, removed.seq);
        } else {
            let seq = self.next_seq();
            #[cfg(target_arch = "wasm32")]
            if let Some(el) = self.live_element {
                let _ = el.set_attribute("data-nve-mod-style", &css);
            }
            self.slot_inline_css
                .insert((node, self.current_slot), OwnedInlineCss { css: css.clone(), seq });
            self.inline_css.insert(node, css);
        }
    }

    /// Automatic cleanup when `current_slot` detaches from `node`: clears all attrs, `inner_html`,
    /// and `inline_css` owned by `current_slot` and restores any values still owned by other slots.
    pub(crate) fn cleanup_current_slot(&mut self, node: NodeId) {
        if let Some(owned) = self.slot_attrs.remove(&(node, self.current_slot)) {
            for attr in owned {
                self.restore_or_remove_attr(node, &attr.name, attr.seq);
            }
        }
        self.clear_inner_html(node);
        if let Some(removed) = self.slot_inline_css.remove(&(node, self.current_slot)) {
            self.restore_or_remove_inline_css(node, removed.seq);
        }
    }
}

/// Target-neutral DOM sink for replaying authoritative plugin mirrors onto retagged elements.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) trait RetagElementSink {
    fn set_attr(&self, name: &str, value: &str);
    fn set_inner_html(&self, html: &str);
}

#[cfg(target_arch = "wasm32")]
impl RetagElementSink for web_sys::Element {
    #[inline]
    fn set_attr(&self, name: &str, value: &str) {
        let _ = self.set_attribute(name, value);
    }

    #[inline]
    fn set_inner_html(&self, html: &str) {
        web_sys::Element::set_inner_html(self, html);
    }
}

/// Replays authoritative resolved plugin mirrors (`attrs`, `inline_css`, `inner_html`) directly
/// onto freshly retagged DOM elements without disturbing `write_seq` or per-slot ownership.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn replay_retagged_mirrors<E: RetagElementSink>(
    retagged: &[NodeId],
    live: &HashMap<NodeId, E>,
    plugin_attrs: &HashMap<NodeId, Vec<(String, String)>>,
    plugin_inline_css: &HashMap<NodeId, String>,
    plugin_inner_html: &HashMap<NodeId, String>,
) {
    for &nid in retagged {
        let Some(el) = live.get(&nid) else {
            continue;
        };
        if let Some(attrs) = plugin_attrs.get(&nid) {
            for (k, v) in attrs {
                el.set_attr(k, v);
            }
        }
        if let Some(css) = plugin_inline_css.get(&nid)
            && !css.is_empty()
        {
            el.set_attr("data-nve-mod-style", css);
        }
        if let Some(html) = plugin_inner_html.get(&nid) {
            el.set_inner_html(html);
        }
    }
}

/// Step 2b: O(1)-gated retag sync for attached tables not already committed in Step 2.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn dispatch_retagged_sync<E>(
    registry: &WebTableRegistry,
    store: &nve_store::NodeStore,
    retagged: &[NodeId],
    live: &HashMap<NodeId, E>,
    committed: &nve_abi::hash::HashSet<(TableSlot, NodeId)>,
    mut on_sync: impl FnMut(&dyn crate::table_system::WebTableSystem, NodeId, &E),
) {
    for &nid in retagged {
        let Some(el) = live.get(&nid) else { continue };
        for sys in registry.iter() {
            let slot = sys.slot();
            if store.has_slot(nid, slot) && !committed.contains(&(slot, nid)) {
                on_sync(sys.as_ref(), nid, el);
            }
        }
    }
}
