//! Targeted Tier-1 binary `RowPatch` & text hot-patching directly over `NodeStore` with
//! collision-free `(template_key, node_path)` resolution and zero steady-state heap allocations.
//!
//! - Compile-time `nve_dioxus::rsx!` stamps static `"__nve_tpl": "<rel_file>#<ordinal>@<path>"`
//!   attributes in debug builds; `NveMutationWriter::instantiate_template_node` strips them before
//!   attribute dispatch and registers `(template_key, prefix_path) -> NodeId` in [`HotPatchIndex`].
//! - Unmounted nodes are evicted in O(1) via the reverse `NodeSlotTable<u64>`.
//! - `RowPatchView::from_words` validates the flat `u32` payload and `Transaction::apply_row_patch`
//!   patches packed words directly without CSS string parsing or heap allocation.

use core::ops::Range;

use nve_abi::hash::HashMap;
use nve_engine::{hot_patch_template_key, HotPatchMsg};
use nve_store::{NodeId, RowPatchView};
use nve_tables::tables::{DebugId, TextContent};

use crate::writer::node_slot_table::NodeSlotTable;
use crate::writer::StoreMutationTx;

/// Debug-only static attribute injected by `nve_dioxus::rsx!` and stripped during template mount.
pub const NVE_TEMPLATE_ATTR: &str = "__nve_tpl";

/// Maximum inline anchor path length stored in [`TemplateRootEntry`].
const MAX_ANCHOR_PATH: usize = 8;

/// Live mounted template anchor mapping a static `(template_key, prefix_path)` to one live `NodeId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TemplateRootEntry {
    pub(crate) node: NodeId,
    pub(crate) path_len: u8,
    pub(crate) path: [u16; MAX_ANCHOR_PATH],
}

impl TemplateRootEntry {
    #[inline]
    pub(crate) fn prefix(&self) -> &[u16] {
        &self.path[..self.path_len as usize]
    }
}

/// Reusable per-writer index of mounted template anchors plus zero-alloc hot-patch scratch buffers.
#[derive(Debug, Default)]
pub(crate) struct HotPatchIndex {
    pub(crate) template_roots: HashMap<u64, Vec<TemplateRootEntry>>,
    pub(crate) node_template_keys: NodeSlotTable<u64>,
    pub(crate) targets: Vec<NodeId>,
    pub(crate) stack: Vec<NodeId>,
    pub(crate) candidates: Vec<NodeId>,
    pub(crate) matched_text: Vec<NodeId>,
}

impl HotPatchIndex {
    pub(crate) fn new() -> Self {
        Self {
            template_roots: HashMap::default(),
            node_template_keys: NodeSlotTable::new(),
            targets: Vec::with_capacity(16),
            stack: Vec::with_capacity(64),
            candidates: Vec::with_capacity(64),
            matched_text: Vec::with_capacity(16),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.template_roots.clear();
        self.node_template_keys.reset();
        self.targets.clear();
        self.stack.clear();
        self.candidates.clear();
        self.matched_text.clear();
    }

    /// Parses `"<rel_file>#<ordinal>@<p0>/<p1>..."` with zero heap allocations and registers `node`.
    pub(crate) fn register_tag(&mut self, node: NodeId, raw: &str) {
        let Some((key, path_len, path)) = parse_template_tag(raw) else {
            return;
        };
        let entry = TemplateRootEntry {
            node,
            path_len,
            path,
        };
        self.template_roots.entry(key).or_default().push(entry);
        self.node_template_keys.insert(node, key);
    }

    /// Evicts `node` from the live template index when its subtree is unmounted.
    #[inline]
    pub(crate) fn unbind_node(&mut self, node: NodeId) {
        let Some(key) = self.node_template_keys.remove(node) else {
            return;
        };
        if let Some(entries) = self.template_roots.get_mut(&key) {
            if let Some(pos) = entries.iter().position(|e| e.node == node) {
                entries.swap_remove(pos);
            }
            if entries.is_empty() {
                self.template_roots.remove(&key);
            }
        }
    }
}

/// Parses `"<rel_file>#<ordinal>@<p0>/<p1>..."` without allocating.
pub(crate) fn parse_template_tag(raw: &str) -> Option<(u64, u8, [u16; MAX_ANCHOR_PATH])> {
    let (left, path_str) = raw.rsplit_once('@')?;
    let (rel_file, ord_str) = left.rsplit_once('#')?;
    let ordinal: u32 = ord_str.parse().ok()?;
    let mut path = [0u16; MAX_ANCHOR_PATH];
    let mut len = 0usize;
    for seg in path_str.split('/') {
        if len >= MAX_ANCHOR_PATH {
            return None;
        }
        path[len] = seg.parse::<u16>().ok()?;
        len += 1;
    }
    if len == 0 {
        return None;
    }
    let key = hot_patch_template_key(rel_file, ordinal);
    Some((key, len as u8, path))
}

#[derive(Debug, Clone)]
struct QueuedPatchMeta {
    template_key: u64,
    node_path: Range<usize>,
    target_test_id: Option<Range<usize>>,
    row_patch_words: Range<usize>,
    text_updates: Range<usize>,
    text_replacements: Range<usize>,
}

/// Flat reusable arena for pending [`HotPatchMsg::TemplatePatch`]es on [`crate::DioxusSurface`].
/// Once warmed, pushing and draining patches performs **zero** heap allocations.
#[derive(Debug, Default)]
pub(crate) struct PendingHotPatchQueue {
    entries: Vec<QueuedPatchMeta>,
    node_paths: Vec<u16>,
    row_patch_words: Vec<u32>,
    test_ids: String,
    text_updates: Vec<(usize, Range<usize>)>,
    text_replacements: Vec<(Range<usize>, Range<usize>)>,
    text_arena: String,
}

impl PendingHotPatchQueue {
    pub(crate) fn push(&mut self, patch: &HotPatchMsg) -> bool {
        let HotPatchMsg::TemplatePatch {
            template_key,
            node_path,
            target_test_id,
            row_patch_words,
            text_updates,
            text_replacements,
            ..
        } = patch
        else {
            return false;
        };

        let np_start = self.node_paths.len();
        self.node_paths.extend_from_slice(node_path);
        let np_range = np_start..self.node_paths.len();

        let tid_range = target_test_id.as_deref().map(|tid| {
            let s = self.test_ids.len();
            self.test_ids.push_str(tid);
            s..self.test_ids.len()
        });

        let rp_start = self.row_patch_words.len();
        self.row_patch_words.extend_from_slice(row_patch_words);
        let rp_range = rp_start..self.row_patch_words.len();

        let tu_start = self.text_updates.len();
        for (idx, text) in text_updates {
            let s = self.text_arena.len();
            self.text_arena.push_str(text);
            self.text_updates.push((*idx, s..self.text_arena.len()));
        }
        let tu_range = tu_start..self.text_updates.len();

        let tr_start = self.text_replacements.len();
        for (old_t, new_t) in text_replacements {
            let os = self.text_arena.len();
            self.text_arena.push_str(old_t);
            let oe = self.text_arena.len();
            self.text_arena.push_str(new_t);
            let ne = self.text_arena.len();
            self.text_replacements.push((os..oe, oe..ne));
        }
        let tr_range = tr_start..self.text_replacements.len();

        self.entries.push(QueuedPatchMeta {
            template_key: *template_key,
            node_path: np_range,
            target_test_id: tid_range,
            row_patch_words: rp_range,
            text_updates: tu_range,
            text_replacements: tr_range,
        });
        true
    }

    pub(crate) fn apply_all(&mut self, mtx: &mut StoreMutationTx<'_, '_>) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let mut changed = false;
        for entry in &self.entries {
            let node_path = &self.node_paths[entry.node_path.clone()];
            let target_test_id = entry
                .target_test_id
                .as_ref()
                .map(|r| &self.test_ids[r.clone()]);
            let row_patch_words = &self.row_patch_words[entry.row_patch_words.clone()];
            let tu_slice = &self.text_updates[entry.text_updates.clone()];
            let tr_slice = &self.text_replacements[entry.text_replacements.clone()];
            let text_arena = &self.text_arena;
            changed |= mtx.apply_hot_patch_inner(
                entry.template_key,
                node_path,
                target_test_id,
                row_patch_words,
                tu_slice.len(),
                |i| {
                    let (idx, ref r) = tu_slice[i];
                    (idx, &text_arena[r.clone()])
                },
                tr_slice.len(),
                |i| {
                    let (ref old_r, ref new_r) = tr_slice[i];
                    (&text_arena[old_r.clone()], &text_arena[new_r.clone()])
                },
            );
        }
        self.clear();
        changed
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.node_paths.clear();
        self.row_patch_words.clear();
        self.test_ids.clear();
        self.text_updates.clear();
        self.text_replacements.clear();
        self.text_arena.clear();
    }
}

impl StoreMutationTx<'_, '_> {
    fn walk_u16_from(&self, start: NodeId, path: &[u16]) -> Option<NodeId> {
        let store = self.tx.store();
        let mut cur = start;
        for &idx in path {
            cur = *store.children(cur).get(idx as usize)?;
        }
        Some(cur)
    }

    fn resolve_template_roots_into_targets(&mut self, template_key: u64, node_path: &[u16]) -> bool {
        self.hot_patch.targets.clear();
        let Some(entries) = self.hot_patch.template_roots.get(&template_key) else {
            return false;
        };
        let store = self.tx.store();
        let mut best_len = 0u8;
        for e in entries {
            if e.path_len > best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                best_len = e.path_len;
            }
        }
        if best_len == 0 {
            return false;
        }
        for e in entries {
            if e.path_len == best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                let suffix = &node_path[best_len as usize..];
                let mut cur = e.node;
                let mut ok = true;
                for &idx in suffix {
                    let Some(&next) = store.children(cur).get(idx as usize) else {
                        ok = false;
                        break;
                    };
                    cur = next;
                }
                if ok {
                    self.hot_patch.targets.push(cur);
                }
            }
        }
        self.hot_patch.targets.sort_unstable();
        self.hot_patch.targets.dedup();
        !self.hot_patch.targets.is_empty()
    }

    /// Collects the longest-prefix template anchor nodes (without walking suffixes) into
    /// `hot_patch.targets`, used when a text node shifted past a dynamic sibling.
    fn collect_template_anchors_into_targets(
        &mut self,
        template_key: u64,
        node_path: &[u16],
    ) -> bool {
        self.hot_patch.targets.clear();
        let Some(entries) = self.hot_patch.template_roots.get(&template_key) else {
            return false;
        };
        let store = self.tx.store();
        let mut best_len = 0u8;
        for e in entries {
            if e.path_len > best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                best_len = e.path_len;
            }
        }
        if best_len == 0 {
            return false;
        }
        for e in entries {
            if e.path_len == best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                self.hot_patch.targets.push(e.node);
            }
        }
        self.hot_patch.targets.sort_unstable();
        self.hot_patch.targets.dedup();
        !self.hot_patch.targets.is_empty()
    }

    fn collect_nodes_by_test_id_into_targets(&mut self, tid: &str) {
        self.hot_patch.targets.clear();
        self.hot_patch.stack.clear();
        let store = self.tx.store();
        self.hot_patch.stack.push(NodeId::ROOT);
        while let Some(nid) = self.hot_patch.stack.pop() {
            if nid != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(nid)
                && dbg.test_id() == tid
            {
                self.hot_patch.targets.push(nid);
            }
            self.hot_patch.stack.extend_from_slice(store.children(nid));
        }
        self.hot_patch.targets.sort_unstable();
        self.hot_patch.targets.dedup();
    }

    fn collect_targets_subtrees_into_candidates(&mut self) {
        self.hot_patch.candidates.clear();
        self.hot_patch.stack.clear();
        let store = self.tx.store();
        // Push targets in reverse so popping yields pre-order DFS document order.
        for &nid in self.hot_patch.targets.iter().rev() {
            self.hot_patch.stack.push(nid);
        }
        while let Some(nid) = self.hot_patch.stack.pop() {
            self.hot_patch.candidates.push(nid);
            for &child in store.children(nid).iter().rev() {
                self.hot_patch.stack.push(child);
            }
        }
    }

    fn collect_all_live_into_candidates(&mut self) {
        self.hot_patch.candidates.clear();
        self.hot_patch.stack.clear();
        let store = self.tx.store();
        self.hot_patch.stack.push(NodeId::ROOT);
        while let Some(nid) = self.hot_patch.stack.pop() {
            if nid != NodeId::ROOT {
                self.hot_patch.candidates.push(nid);
            }
            for &child in store.children(nid).iter().rev() {
                self.hot_patch.stack.push(child);
            }
        }
    }

    fn resolve_targets_into_scratch(
        &mut self,
        template_key: u64,
        node_path: &[u16],
        target_test_id: Option<&str>,
    ) {
        self.hot_patch.targets.clear();
        if template_key != 0
            && self.resolve_template_roots_into_targets(template_key, node_path)
        {
            return;
        }
        if let Some(tid) = target_test_id {
            self.collect_nodes_by_test_id_into_targets(tid);
            if !self.hot_patch.targets.is_empty() {
                return;
            }
        }
        if template_key == 0
            && !node_path.is_empty()
            && let Some(target) = self.walk_u16_from(NodeId::ROOT, node_path)
            && target != NodeId::ROOT
        {
            self.hot_patch.targets.push(target);
        }
    }

    /// Applies a Tier-1 binary `RowPatch` / text hot patch directly onto `NodeStore` with zero
    /// steady-state heap allocations.
    pub(crate) fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        let HotPatchMsg::TemplatePatch {
            template_key,
            node_path,
            target_test_id,
            row_patch_words,
            text_updates,
            text_replacements,
            ..
        } = patch
        else {
            return false;
        };
        self.apply_hot_patch_inner(
            *template_key,
            node_path,
            target_test_id.as_deref(),
            row_patch_words,
            text_updates.len(),
            |i| (text_updates[i].0, text_updates[i].1.as_str()),
            text_replacements.len(),
            |i| {
                (
                    text_replacements[i].0.as_str(),
                    text_replacements[i].1.as_str(),
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_hot_patch_inner<'a>(
        &mut self,
        template_key: u64,
        node_path: &[u16],
        target_test_id: Option<&str>,
        row_patch_words: &[u32],
        text_updates_len: usize,
        text_update_at: impl Fn(usize) -> (usize, &'a str),
        text_replacements_len: usize,
        text_replacement_at: impl Fn(usize) -> (&'a str, &'a str),
    ) -> bool {
        let mut changed = false;
        self.hot_patch.matched_text.clear();

        // 1. Targeted text patching: prefer exact `text_replacements` over positional `text_updates`.
        if text_replacements_len > 0 {
            let mut matched_any = false;
            self.resolve_targets_into_scratch(template_key, node_path, target_test_id);
            if !self.hot_patch.targets.is_empty() {
                self.collect_targets_subtrees_into_candidates();
                matched_any = self.apply_text_replacements_on_candidates(
                    text_replacements_len,
                    &text_replacement_at,
                );
            }
            // If a text leaf shifted after a dynamic sibling, widen to its template anchor subtree.
            if !matched_any
                && template_key != 0
                && self.collect_template_anchors_into_targets(template_key, node_path)
            {
                self.collect_targets_subtrees_into_candidates();
                matched_any = self.apply_text_replacements_on_candidates(
                    text_replacements_len,
                    &text_replacement_at,
                );
            }
            // Unkeyed / untargeted exact text replacement fallback (e.g. text inside a component
            // with no native ancestor `test_id` and no template anchor).
            if !matched_any
                && target_test_id.is_none()
                && (template_key == 0 || !self.hot_patch.template_roots.contains_key(&template_key))
            {
                self.collect_all_live_into_candidates();
                matched_any = self.apply_text_replacements_on_candidates(
                    text_replacements_len,
                    &text_replacement_at,
                );
            }
            changed |= matched_any;
        } else if text_updates_len > 0 {
            self.resolve_targets_into_scratch(template_key, node_path, target_test_id);
            if !self.hot_patch.targets.is_empty() {
                self.collect_targets_subtrees_into_candidates();
            } else if template_key == 0 && target_test_id.is_none() && node_path.is_empty() {
                self.collect_all_live_into_candidates();
            } else {
                self.hot_patch.candidates.clear();
            }
            let store = self.tx.store();
            self.hot_patch
                .candidates
                .retain(|&nid| store.has::<TextContent>(nid) && store.children(nid).is_empty());
            for i in 0..text_updates_len {
                let (idx, new_text) = text_update_at(i);
                if let Some(&nid) = self.hot_patch.candidates.get(idx) {
                    self.tx.edit::<TextContent>(nid).text(new_text);
                    self.hot_patch.matched_text.push(nid);
                    changed = true;
                }
            }
        }

        // 2. Targeted binary `RowPatch` application: never parse CSS strings or spray globally.
        if !row_patch_words.is_empty()
            && let Some(view) = RowPatchView::from_words(row_patch_words)
            && !view.is_empty()
        {
            self.resolve_targets_into_scratch(template_key, node_path, target_test_id);
            if self.hot_patch.targets.is_empty()
                && template_key == 0
                && target_test_id.is_none()
                && node_path.is_empty()
                && !self.hot_patch.matched_text.is_empty()
            {
                self.hot_patch
                    .targets
                    .extend_from_slice(&self.hot_patch.matched_text);
                self.hot_patch.targets.sort_unstable();
                self.hot_patch.targets.dedup();
            }
            for i in 0..self.hot_patch.targets.len() {
                let nid = self.hot_patch.targets[i];
                if self.tx.store().is_live(nid) {
                    self.tx.apply_row_patch(nid, view);
                    changed = true;
                }
            }
        }

        changed
    }

    fn apply_text_replacements_on_candidates<'a>(
        &mut self,
        len: usize,
        at: &impl Fn(usize) -> (&'a str, &'a str),
    ) -> bool {
        let mut changed = false;
        for r_idx in 0..len {
            let (old_text, new_text) = at(r_idx);
            for c_idx in 0..self.hot_patch.candidates.len() {
                let nid = self.hot_patch.candidates[c_idx];
                let is_match = self
                    .tx
                    .store()
                    .try_row::<TextContent>(nid)
                    .is_some_and(|t| t.text() == old_text);
                if is_match {
                    self.tx.edit::<TextContent>(nid).text(new_text);
                    self.hot_patch.matched_text.push(nid);
                    changed = true;
                }
            }
        }
        changed
    }
}
