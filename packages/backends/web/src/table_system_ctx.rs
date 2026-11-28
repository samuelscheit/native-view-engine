//! Per-commit mutable context ([`WebSystemCommitCtx`]) passed into [`super::WebTableSystem`] hooks.

use nve_abi::hash::HashMap;
use nve_store::{EventBatchBuf, NodeId, TableSlot};

#[cfg(any(target_arch = "wasm32", test))]
use crate::table_system::WebStoreRenderer;
use crate::table_system::WebTableRegistry;

/// Click hook returned by [`super::WebTableSystem::click_interceptor`]; `true` consumes the click.
#[cfg(target_arch = "wasm32")]
pub type ClickInterceptor = fn(&web_sys::MouseEvent) -> bool;

/// Registry click hooks mirrored into the root event delegator, refreshed per registry revision.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(crate) struct ClickInterceptors {
    revision: u32,
    hooks: Vec<ClickInterceptor>,
}

#[cfg(target_arch = "wasm32")]
impl ClickInterceptors {
    pub(crate) fn refresh(&mut self, registry: &WebTableRegistry) {
        if self.revision != registry.revision() {
            self.revision = registry.revision();
            self.hooks.clear();
            self.hooks
                .extend(registry.iter().iter().filter_map(|sys| sys.click_interceptor()));
        }
    }

    /// Whether no plugin click hooks are registered.
    pub(crate) fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Runs every hook in priority order without holding a `RefCell` borrow across user code, so
    /// a synchronously re-entrant hook cannot trigger `BorrowMutError` in `refresh()`.
    pub(crate) fn try_consume(
        cell: &std::cell::RefCell<Self>,
        ev: &web_sys::MouseEvent,
    ) -> bool {
        let len = cell.borrow().hooks.len();
        for i in 0..len {
            let Some(&hook) = cell.borrow().hooks.get(i) else {
                break;
            };
            if hook(ev) {
                return true;
            }
        }
        false
    }
}

/// Whether placing `cid` via a non-state-preserving insert into a connected parent needs a
/// connected-sync subtree walk. Childless newly created leaves already committed in Step 4 (where
/// plugins call `request_connected_sync` when needed) and have no pre-existing descendants.
#[cfg(any(target_arch = "wasm32", test))]
#[inline]
pub(crate) fn needs_connected_sync_on_place(
    created: &nve_abi::hash::HashSet<NodeId>,
    store: &nve_store::NodeStore,
    cid: NodeId,
) -> bool {
    !(created.contains(&cid) && store.children(cid).is_empty())
}

#[cfg(any(target_arch = "wasm32", test))]
impl WebStoreRenderer {
    /// Queues [`super::WebTableSystem::sync_connected`] for every plugin table on `node` (its
    /// element was retagged, so imperative state such as the top layer must be re-applied).
    pub(crate) fn queue_connected_sync_node(&mut self, store: &nve_store::NodeStore, node: NodeId) {
        for sys in self.registry.iter() {
            if store.has_slot(node, sys.slot()) {
                self.connected_sync.push((sys.slot(), node));
            }
        }
    }

    /// [`Self::queue_connected_sync_node`] for `root` and every descendant not yet visited during
    /// this commit: a fallback DOM move of `root` ran the removing steps on its whole subtree (a
    /// `Dialog`'s scrim wrapper moving evicts the `<dialog>` inside it from the top layer). The
    /// reusable per-commit `connected_visited` set ensures $O(N)$ worst-case traversal even on
    /// top-down `NVE2` child lists, with zero steady-state allocations. Returns newly visited count.
    pub(crate) fn queue_connected_sync_subtree(
        &mut self,
        store: &nve_store::NodeStore,
        root: NodeId,
    ) -> usize {
        if self.registry.iter().is_empty() || !self.connected_visited.insert(root) {
            return 0;
        }
        let mut stack = std::mem::take(&mut self.connected_stack);
        stack.clear();
        stack.push(root);
        let mut visited = 0usize;
        while let Some(node) = stack.pop() {
            visited += 1;
            self.queue_connected_sync_node(store, node);
            for &child in store.children(node).iter().rev() {
                if self.connected_visited.insert(child) {
                    stack.push(child);
                }
            }
        }
        self.connected_stack = stack;
        visited
    }

    /// Target-neutral connected-sync drainer shared by `wasm32` Step 6 and native unit tests.
    /// Clears the per-commit `connected_visited` guard and invokes `on_sync` once per `(slot, node)`
    /// (first-queued order, so top-layer stacking follows request order) for still-live nodes.
    pub(crate) fn drain_connected_sync<E>(
        &mut self,
        store: &nve_store::NodeStore,
        live: &HashMap<NodeId, E>,
        mut on_sync: impl FnMut(&dyn crate::table_system::WebTableSystem, NodeId, &E),
    ) {
        if !self.connected_visited.is_empty() {
            self.connected_visited.clear();
        }
        if !self.connected_seen.is_empty() {
            self.connected_seen.clear();
        }
        for entry @ (slot, node) in self.connected_sync.drain(..) {
            if self.connected_seen.insert(entry)
                && store.is_live(node)
                && let (Some(sys), Some(el)) = (self.registry.get(slot), live.get(&node))
            {
                on_sync(sys.as_ref(), node, el);
            }
        }
    }

    /// Drains the queued connected syncs once the commit has connected and placed every element,
    /// calling [`super::WebTableSystem::sync_connected`] once per `(slot, node)` (first-queued
    /// order, so top-layer stacking follows request order) for still-live nodes. Pushes never
    /// scan; duplicates coalesce here in O(1) each.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_connected_plugins(
        &mut self,
        store: &nve_store::NodeStore,
        live: &HashMap<NodeId, web_sys::Element>,
    ) {
        self.drain_connected_sync(store, live, |sys, node, el| {
            sys.sync_connected(store, node, el);
        });
    }
}

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
    pub(crate) inner_html_owner: &'a mut HashMap<NodeId, (TableSlot, u64)>,
    pub(crate) slot_inline_css: &'a mut HashMap<(NodeId, TableSlot), OwnedInlineCss>,
    pub(crate) write_seq: &'a mut u64,
    #[cfg(target_arch = "wasm32")]
    pub live_element: Option<&'a web_sys::Element>,
    pub(crate) is_retagged: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) track_connected_sync: bool,
    pub(crate) connected_sync: &'a mut Vec<(TableSlot, NodeId)>,
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
    /// Effective-value mirror write only (retag replay source); the live element is untouched.
    fn write_mirror_attr(&mut self, node: NodeId, name: &str, value: &str) {
        let list = self.attrs.entry(node).or_default();
        if let Some((_, v)) = list.iter_mut().find(|(k, _)| k == name) {
            v.clear();
            v.push_str(value);
        } else {
            list.push((name.to_owned(), value.to_owned()));
        }
    }

    fn write_effective_attr(&mut self, node: NodeId, name: &str, value: &str) {
        self.write_mirror_attr(node, name, value);
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
        // Short-circuit unchanged steady-state writes on non-retagged elements (avoids redundant
        // WASM->JS `set_attribute` FFI on 120 Hz canvas repaints while preserving retag replay).
        if !self.is_retagged
            && self
                .attrs
                .get(&node)
                .is_some_and(|list| list.iter().any(|(k, v)| k == name && v == value))
            && self
                .slot_attrs
                .get(&(node, self.current_slot))
                .is_some_and(|owned| owned.iter().any(|e| e.name == name && e.value == value))
        {
            return;
        }
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
        let seq = self.next_seq();
        #[cfg(target_arch = "wasm32")]
        if let Some(el) = self.live_element {
            el.set_inner_html(&html);
        }
        self.inner_html.insert(node, html);
        self.inner_html_owner.insert(node, (self.current_slot, seq));
    }

    pub fn clear_inner_html(&mut self, node: NodeId) {
        if self
            .inner_html_owner
            .get(&node)
            .is_some_and(|&(slot, _)| slot == self.current_slot)
        {
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

    /// Queues [`super::WebTableSystem::sync_connected`] for `node` (current slot), run in Step 6
    /// once child reconciliation has connected and placed every element (a Step-5 move would
    /// otherwise evict state applied earlier, e.g. `:modal`). Duplicate requests coalesce at
    /// drain time. No-op when no live DOM element is mounted (prevents unbounded queue growth).
    pub fn request_connected_sync(&mut self, node: NodeId) {
        #[cfg(target_arch = "wasm32")]
        if self.live_element.is_none() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !self.track_connected_sync {
            return;
        }
        self.connected_sync.push((self.current_slot, node));
    }

    /// Drops the current slot's mirrored `name` on `node` WITHOUT touching the live element, for
    /// state the plugin now owns imperatively (a `<dialog>`'s `open`), so retag mirror replay
    /// never writes the stale value back. When an older write by another slot remains, the
    /// mirror falls back to it (mirror only: the live element stays plugin-owned).
    pub fn forget_attr(&mut self, node: NodeId, name: &str) {
        let key = (node, self.current_slot);
        let mut removed_seq = None;
        if let Some(owned) = self.slot_attrs.get_mut(&key) {
            if let Some(i) = owned.iter().position(|e| e.name == name) {
                removed_seq = Some(owned.swap_remove(i).seq);
            }
            if owned.is_empty() {
                self.slot_attrs.remove(&key);
            }
        }
        match self.find_latest_other_slot_attr(node, name) {
            None => {
                if let Some(list) = self.attrs.get_mut(&node) {
                    list.retain(|(k, _)| k != name);
                    if list.is_empty() {
                        self.attrs.remove(&node);
                    }
                }
            }
            Some((fallback, max_seq)) if removed_seq.is_some_and(|seq| max_seq < seq) => {
                self.write_mirror_attr(node, name, &fallback);
            }
            Some(_) => {}
        }
    }

    /// Element tag `node` resolves to across every registered plugin and the core tables.
    #[inline]
    pub fn resolved_tag<'s>(&self, store: &'s nve_store::NodeStore, node: NodeId) -> &'s str {
        self.registry.resolve_tag(store, node).0
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

#[cfg(any(target_arch = "wasm32", test))]
impl crate::table_system::WebStoreRenderer {
    /// Replays authoritative resolved plugin mirrors (`attrs`, `inline_css`, `inner_html`) onto
    /// freshly retagged DOM elements without disturbing `write_seq` or per-slot ownership.
    /// Effective values whose winning write has `seq > written_after` were written by this
    /// commit's Steps 1-2 straight onto the (already fresh) live element and are skipped.
    pub(crate) fn replay_retagged_mirrors<E: RetagElementSink>(
        &self,
        retagged: &[NodeId],
        live: &HashMap<NodeId, E>,
        written_after: u64,
    ) {
        for &nid in retagged {
            let Some(el) = live.get(&nid) else {
                continue;
            };
            if let Some(attrs) = self.plugin_attrs.get(&nid) {
                for (k, v) in attrs {
                    if self.latest_attr_seq(nid, k) <= written_after {
                        el.set_attr(k, v);
                    }
                }
            }
            if let Some(css) = self.plugin_inline_css.get(&nid)
                && !css.is_empty()
                && self.latest_inline_css_seq(nid) <= written_after
            {
                el.set_attr("data-nve-mod-style", css);
            }
            if let Some(html) = self.plugin_inner_html.get(&nid)
                && self.inner_html_owner.get(&nid).map_or(0, |&(_, seq)| seq) <= written_after
            {
                el.set_inner_html(html);
            }
        }
    }

    fn latest_attr_seq(&self, node: NodeId, name: &str) -> u64 {
        self.registry
            .iter()
            .iter()
            .filter_map(|sys| self.slot_attrs.get(&(node, sys.slot())))
            .flat_map(|owned| owned.iter().filter(|e| e.name == name))
            .map(|e| e.seq)
            .max()
            .unwrap_or(0)
    }

    fn latest_inline_css_seq(&self, node: NodeId) -> u64 {
        self.registry
            .iter()
            .iter()
            .filter_map(|sys| self.slot_inline_css.get(&(node, sys.slot())))
            .map(|e| e.seq)
            .max()
            .unwrap_or(0)
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
