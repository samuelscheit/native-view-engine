//! Targeted Tier-1 `HotPatchMsg::TemplatePatch` surgical patching directly over `NodeStore`.
//!
//! Ensures live hot-patches update ONLY the exact matching node(s) addressed by
//! `target_test_id`, `node_path`, or exact `text_replacements`, never spraying
//! `attr_updates` or text overwrites onto unrelated sibling nodes.

use nve_tables::tables::{DebugId, TextContent};
use nve_engine::HotPatchMsg;
use nve_store::NodeId;

use crate::writer::StoreMutationTx;

impl StoreMutationTx<'_, '_> {
    fn collect_nodes_by_test_id(&self, tid: &str, out: &mut Vec<NodeId>) {
        out.clear();
        let store = self.tx.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(nid) = stack.pop() {
            if nid != NodeId::ROOT
                && let Some(dbg) = store.try_row::<DebugId>(nid)
                && dbg.test_id() == tid
            {
                out.push(nid);
            }
            for &cid in store.children(nid) {
                stack.push(cid);
            }
        }
        out.sort_unstable();
    }

    fn walk_usize_path(&self, path: &[usize]) -> Option<NodeId> {
        let store = self.tx.store();
        let mut cur = NodeId::ROOT;
        for &idx in path {
            cur = *store.children(cur).get(idx)?;
        }
        (cur != NodeId::ROOT).then_some(cur)
    }

    fn collect_subtree_nodes(&self, roots: &[NodeId], out: &mut Vec<NodeId>) {
        out.clear();
        let store = self.tx.store();
        let mut stack = roots.to_vec();
        while let Some(nid) = stack.pop() {
            out.push(nid);
            for &cid in store.children(nid) {
                stack.push(cid);
            }
        }
        out.sort_unstable();
        out.dedup();
    }

    fn collect_all_live_nodes(&self, out: &mut Vec<NodeId>) {
        out.clear();
        let store = self.tx.store();
        let mut stack = vec![NodeId::ROOT];
        while let Some(nid) = stack.pop() {
            if nid != NodeId::ROOT {
                out.push(nid);
            }
            for &cid in store.children(nid) {
                stack.push(cid);
            }
        }
        out.sort_unstable();
    }

    /// Applies a Tier-1 live template/style/text hot patch surgically to matching `NodeStore` nodes.
    pub(crate) fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        let HotPatchMsg::TemplatePatch {
            attr_updates,
            text_updates,
            target_test_id,
            node_path,
            text_replacements,
            ..
        } = patch
        else {
            return false;
        };

        let mut changed = false;
        let mut matched_text_nodes: Vec<NodeId> = Vec::new();

        // 1. Targeted text patching: prefer exact `text_replacements` over positional `text_updates`.
        if !text_replacements.is_empty() {
            let mut candidates = Vec::new();
            if let Some(tid) = target_test_id.as_deref() {
                let mut tid_roots = Vec::new();
                self.collect_nodes_by_test_id(tid, &mut tid_roots);
                if !tid_roots.is_empty() {
                    self.collect_subtree_nodes(&tid_roots, &mut candidates);
                }
            } else {
                self.collect_all_live_nodes(&mut candidates);
            }

            for (old_text, new_text) in text_replacements {
                for &nid in &candidates {
                    let is_match = {
                        let store = self.tx.store();
                        store
                            .try_row::<TextContent>(nid)
                            .is_some_and(|t| t.text() == old_text)
                    };
                    if is_match {
                        self.tx.edit::<TextContent>(nid).text(new_text);
                        matched_text_nodes.push(nid);
                        changed = true;
                    }
                }
            }
        } else if !text_updates.is_empty() {
            let mut text_nodes = Vec::new();
            if let Some(tid) = target_test_id.as_deref() {
                let mut tid_roots = Vec::new();
                self.collect_nodes_by_test_id(tid, &mut tid_roots);
                if !tid_roots.is_empty() {
                    let mut sub = Vec::new();
                    self.collect_subtree_nodes(&tid_roots, &mut sub);
                    for nid in sub {
                        if self.tx.store().has::<TextContent>(nid) {
                            text_nodes.push(nid);
                        }
                    }
                }
            } else if !node_path.is_empty() {
                if let Some(target) = self.walk_usize_path(node_path) {
                    let mut sub = Vec::new();
                    self.collect_subtree_nodes(&[target], &mut sub);
                    for nid in sub {
                        if self.tx.store().has::<TextContent>(nid) {
                            text_nodes.push(nid);
                        }
                    }
                }
            } else {
                let mut all = Vec::new();
                self.collect_all_live_nodes(&mut all);
                for nid in all {
                    if self.tx.store().has::<TextContent>(nid) {
                        text_nodes.push(nid);
                    }
                }
            }
            for (idx, new_text) in text_updates {
                if let Some(&nid) = text_nodes.get(*idx) {
                    self.tx.edit::<TextContent>(nid).text(new_text);
                    matched_text_nodes.push(nid);
                    changed = true;
                }
            }
        }

        // 2. Targeted attribute patching: resolve exact target nodes, never spray globally.
        if !attr_updates.is_empty() {
            let mut targets = Vec::new();
            if let Some(tid) = target_test_id.as_deref() {
                self.collect_nodes_by_test_id(tid, &mut targets);
            } else if !node_path.is_empty() {
                if let Some(target) = self.walk_usize_path(node_path) {
                    targets.push(target);
                }
            } else if !matched_text_nodes.is_empty() {
                targets.extend_from_slice(&matched_text_nodes);
            }
            targets.sort_unstable();
            targets.dedup();

            for nid in targets {
                for (name, val) in attr_updates {
                    self.apply_str_attr(nid, name, val);
                    changed = true;
                }
            }
        }

        changed
    }
}
