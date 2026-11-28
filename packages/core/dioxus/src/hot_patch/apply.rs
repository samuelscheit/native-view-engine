//! Target resolution and zero-alloc Tier-1 binary `RowPatch` / text patch application on
//! [`StoreMutationTx`].

use nve_engine::HotPatchMsg;
use nve_store::{NodeId, RowPatchView};
use nve_tables::tables::{DebugId, TextContent};

use crate::writer::StoreMutationTx;

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
        let mut best_len = 0usize;
        for e in entries {
            let plen = e.path_len();
            if plen > best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                best_len = plen;
            }
        }
        if best_len == 0 {
            return false;
        }
        for e in entries {
            if e.path_len() == best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                let suffix = &node_path[best_len..];
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
        let mut best_len = 0usize;
        for e in entries {
            let plen = e.path_len();
            if plen > best_len
                && store.is_live(e.node)
                && node_path.starts_with(e.prefix())
            {
                best_len = plen;
            }
        }
        if best_len == 0 {
            return false;
        }
        for e in entries {
            if e.path_len() == best_len
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
    pub(super) fn apply_hot_patch_inner<'a>(
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
