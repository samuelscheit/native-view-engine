//! `display: contents` for the Taffy adapter. Taffy 0.16 has no `Display::Contents`, so a parent
//! whose children include contents nodes exposes Taffy a flattened box-children list: every
//! contents child is replaced, recursively and in document order, by its own children. Contents
//! nodes are never laid out; the rounding pass passes their parent's origin straight through.

use nve_components::tables::is_display_contents;
use nve_store::{NodeId, NodeStore};

use super::node_lists::NodeLists;

/// Box children of every parent with at least one `display: contents` child.
#[derive(Debug, Default)]
pub(super) struct BoxChildren {
    lists: NodeLists,
    /// `true` when the parent's box children differ from `NodeStore::children` (read `lists`).
    flattened: Vec<bool>,
    /// Reused DFS stack of the flattening walk.
    stack: Vec<NodeId>,
    /// Reused flattened output, written to `lists` in one exact-length `extend`.
    out: Vec<NodeId>,
}

impl BoxChildren {
    pub(super) fn ensure_len(&mut self, len: usize) {
        self.lists.ensure_len(len);
        if self.flattened.len() < len {
            self.flattened.resize(len, false);
        }
    }

    pub(super) fn reset(&mut self) {
        self.lists.reset();
        self.flattened.clear();
        self.stack.clear();
        self.out.clear();
    }

    #[cfg(test)]
    pub(super) fn list_capacity(&self, parent: NodeId) -> usize {
        self.lists.capacity_of(parent.index())
    }

    #[cfg(test)]
    pub(super) fn dead_slots(&self) -> usize {
        self.lists.dead_slots()
    }

    /// Pool accounting invariant of the underlying lists; unflattened parents own no capacity.
    #[cfg(test)]
    pub(super) fn assert_consistent(&self) {
        self.lists.assert_consistent();
        for (idx, &flat) in self.flattened.iter().enumerate() {
            assert!(
                flat || self.lists.capacity_of(idx) == 0,
                "unflattened parent {idx} pins list capacity"
            );
        }
    }

    /// Forgets `idx`'s list (node slot created or removed).
    pub(super) fn release(&mut self, idx: usize) {
        self.lists.release(idx);
        if let Some(f) = self.flattened.get_mut(idx) {
            *f = false;
        }
    }

    /// Recomputes `parent`'s box children from the live tree.
    pub(super) fn rebuild(&mut self, store: &NodeStore, parent: NodeId) {
        let idx = parent.index();
        let children = store.children(parent);
        let flatten = children
            .iter()
            .any(|&c| is_display_contents(store, c));
        if !flatten {
            // Return the capacity: a parent that stops flattening must not pin its peak list.
            if self.flattened.get(idx).copied().unwrap_or(false) {
                self.release(idx);
            }
            return;
        }
        let Some(flag) = self.flattened.get_mut(idx) else {
            return;
        };
        *flag = true;
        self.out.clear();
        self.stack.clear();
        self.stack.extend(children.iter().rev());
        while let Some(child) = self.stack.pop() {
            if is_display_contents(store, child) {
                self.stack.extend(store.children(child).iter().rev());
            } else {
                self.out.push(child);
            }
        }
        self.lists.clear(idx);
        self.lists.extend(idx, self.out.iter().copied());
    }

    /// Whether `parent` exposes a flattened list (some child is `display: contents`).
    #[inline]
    pub(super) fn is_flattened(&self, parent: NodeId) -> bool {
        self.flattened.get(parent.index()).copied().unwrap_or(false)
    }

    /// Repacks the list pool once rebuilds have fragmented it.
    pub(super) fn compact_if_fragmented(&mut self) {
        self.lists.compact_if_fragmented();
    }

    /// The children Taffy lays out under `parent`.
    #[inline]
    pub(super) fn get<'a>(&'a self, store: &'a NodeStore, parent: NodeId) -> &'a [NodeId] {
        if self.is_flattened(parent) {
            self.lists.get(parent.index())
        } else {
            store.children(parent)
        }
    }
}
