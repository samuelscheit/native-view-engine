//! Parent/child tree operations of [`Transaction`].

use crate::id::{NodeId, NodeIndex};
use crate::store::{ANCESTOR_MARK, CHILD_SEEN, CHILDREN_DIRTY, NodeStore};
use crate::tx::{touch, TxCore};
use nve_abi::table::effect;

impl TxCore<'_> {
    #[inline]
    fn mark_children(&mut self, parent: NodeIndex) {
        let s = &mut *self.store;
        let row = &mut s.nodes[parent];
        let first = row.flags & CHILDREN_DIRTY == 0;
        row.flags |= CHILDREN_DIRTY;
        s.reparented.push_if(parent, first);
        touch(row, &mut s.touched, parent, effect::LAYOUT);
    }

    /// A child that joined another parent: its row (`parent`) changed and it inherits a new
    /// context (typography, layout), so it is dirty in its own right — not merely moved.
    #[inline]
    fn mark_reparented(&mut self, child: NodeIndex) {
        let s = &mut *self.store;
        touch(
            &mut s.nodes[child],
            &mut s.touched,
            child,
            effect::LAYOUT | effect::MEASURE | effect::PAINT,
        );
    }

    /// A still-live child that left its parent without joining another: its row (`parent`)
    /// changed and it no longer renders, so hosts and layout must see it.
    const DETACHED_EFFECTS: u8 = effect::LAYOUT | effect::PAINT;

    /// `set`: clears the parent pointer of every child of `parent` and flags it `CHILD_SEEN`;
    /// `!set`: drops those flags again, touching every flagged child that stayed detached (zero
    /// allocations).
    fn flag_children(&mut self, parent: NodeId, set: bool) {
        let NodeStore {
            nodes, children, touched, ..
        } = &mut *self.store;
        let owner = nodes[parent.node_index()].children;
        for &c in children.get(owner) {
            let idx = c.node_index();
            let Some(r) = nodes.get_mut(idx) else {
                continue;
            };
            if set {
                if r.parent == parent {
                    r.parent = NodeId::NULL;
                    r.flags |= CHILD_SEEN;
                }
            } else {
                if r.flags & CHILD_SEEN != 0 && r.parent.is_null() {
                    touch(r, touched, idx, Self::DETACHED_EFFECTS);
                }
                r.flags &= !CHILD_SEEN;
            }
        }
    }

    /// Returns `true` if `candidate` equals `node` or is an ancestor of `node`.
    fn is_ancestor_or_self(&self, candidate: NodeId, mut node: NodeId) -> bool {
        let mut hops = 0usize;
        let limit = self.store.nodes.len();
        while !node.is_null() {
            if node == candidate {
                return true;
            }
            hops += 1;
            if hops > limit {
                return true;
            }
            let Some(p) = self.store.parent(node) else {
                break;
            };
            node = p;
        }
        false
    }

    /// Validated child-list replacement returning `false` without mutating the tree if `parent` is
    /// dead, any child is non-live/`NULL`, any child is duplicated, or any child would form a cycle.
    /// Fallible version of [`Self::set_children`] returning `false` on dead/NULL/duplicate/cyclic input.
    pub fn try_set_children(&mut self, parent: NodeId, new_children: &[NodeId]) -> bool {
        if !self.store.is_live(parent) {
            return false;
        }
        // O(K) unchanged fast path: existing children are already live, unique, and acyclic.
        if self.store.children(parent) == new_children {
            return true;
        }
        // Mark `parent` and its ancestor chain once so cycle detection across K children is
        // O(K + depth) instead of O(K * depth), with zero heap allocations.
        let limit = self.store.nodes.len();
        let mut cur = parent;
        let mut anc_hops = 0usize;
        while !cur.is_null() && anc_hops <= limit {
            let cidx = cur.node_index();
            self.store.nodes[cidx].flags |= ANCESTOR_MARK;
            anc_hops += 1;
            let p = self.store.nodes[cidx].parent;
            if p.is_null() || !self.store.is_live(p) {
                break;
            }
            cur = p;
        }

        let mut valid = true;
        let mut marked = 0usize;
        for &c in new_children {
            if !self.store.is_live(c) {
                valid = false;
                break;
            }
            let idx = c.node_index();
            if self.store.nodes[idx].flags & (ANCESTOR_MARK | CHILD_SEEN) != 0 {
                valid = false;
                break;
            }
            self.store.nodes[idx].flags |= CHILD_SEEN;
            marked += 1;
        }
        for &c in &new_children[..marked] {
            self.store.nodes[c.node_index()].flags &= !CHILD_SEEN;
        }
        // Clear ancestor scratch marks.
        let mut cur = parent;
        let mut clr_hops = 0usize;
        while !cur.is_null() && clr_hops <= limit {
            let cidx = cur.node_index();
            self.store.nodes[cidx].flags &= !ANCESTOR_MARK;
            clr_hops += 1;
            let p = self.store.nodes[cidx].parent;
            if p.is_null() || !self.store.is_live(p) {
                break;
            }
            cur = p;
        }
        if !valid {
            return false;
        }
        let parent_idx = parent.node_index();
        // Clear previous parent pointers, flagging the old children (`CHILD_SEEN` scratch) so a
        // child that stays is told apart from one that arrives — without a temporary Vec.
        self.flag_children(parent, true);
        for &c in new_children {
            let idx = c.node_index();
            if self.store.nodes[idx].flags & CHILD_SEEN == 0 {
                if self.store.parent(c).is_some() {
                    self.detach_child(c);
                }
                self.mark_reparented(idx);
            }
            self.store.nodes[idx].parent = parent;
        }
        self.flag_children(parent, false);
        let s = &mut *self.store;
        let old_span = s.nodes[parent_idx].children;
        let span = s.children.replace(old_span, new_children);
        s.nodes[parent_idx].children = span;
        self.mark_children(parent_idx);
        true
    }

    /// Replaces the ordered child list of `parent` (no-op when unchanged or invalid, zero heap
    /// allocations). Rejects dead/null nodes, duplicates, and cycles; detaches moved children from
    /// any previous parent.
    pub fn set_children(&mut self, parent: NodeId, new_children: &[NodeId]) {
        let _ = self.try_set_children(parent, new_children);
    }

    /// Inserts `child` at `index` (clamped) in `parent`'s child list, moving it from any previous
    /// parent (zero heap allocations via `SpanPool::insert_at`). Ignores dead nodes or cycles.
    pub fn insert_child(&mut self, parent: NodeId, index: usize, child: NodeId) {
        if !self.store.is_live(parent)
            || !self.store.is_live(child)
            || self.is_ancestor_or_self(child, parent)
        {
            return;
        }
        let moved_in = self.store.parent(child) != Some(parent);
        self.detach_child(child);
        let parent_idx = parent.node_index();
        let s = &mut *self.store;
        let old_span = s.nodes[parent_idx].children;
        let new_span = s.children.insert_at(old_span, index, child);
        s.nodes[parent_idx].children = new_span;
        s.nodes[child.node_index()].parent = parent;
        self.mark_children(parent_idx);
        if moved_in {
            self.mark_reparented(child.node_index());
        }
    }

    /// Appends `child` to `parent` (zero heap allocations).
    #[inline]
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let len = self.store.children(parent).len();
        self.insert_child(parent, len, child);
    }

    /// Detaches `child` from its parent (the child stays alive).
    pub fn detach_child(&mut self, child: NodeId) {
        let Some(parent) = self.store.parent(child) else {
            return;
        };
        let parent_idx = parent.node_index();
        let s = &mut *self.store;
        let span = s.nodes[parent_idx].children;
        if let Some(at) = s.children.get(span).iter().position(|&c| c == child) {
            let new_span = s.children.remove_at(span, at);
            s.nodes[parent_idx].children = new_span;
        }
        let child_idx = child.node_index();
        s.nodes[child_idx].parent = NodeId::NULL;
        touch(&mut s.nodes[child_idx], &mut s.touched, child_idx, Self::DETACHED_EFFECTS);
        self.mark_children(parent_idx);
    }
}
