//! `Transaction` and validated [`NodeMut`]: the only way to mutate a [`NodeStore`];
//! `commit_into` closes the frame.

use nve_abi::table::effect;

use crate::changes::Command;
use crate::id::{NodeId, NodeIndex, ResourceId};
use crate::resource::Resource;
use crate::row::{DirtyList, RowWriter};
use crate::span_pool::StrSpan;
use crate::store::{
    NodeRef, NodeRow, NodeStore, ANCESTOR_MARK, CHILDREN_DIRTY, CHILD_SEEN, CREATED, IN_FREE,
    LIVE, PENDING_FREE,
};
use crate::table::{NveTable, Prop, ResProp, StrProp, TableSlot};
use crate::value::WordValue;

/// Mutable frame scope over a [`NodeStore`].
pub struct Transaction<'s> {
    pub(crate) store: &'s mut NodeStore,
}

impl NodeStore {
    /// Opens a mutation scope. Changes accumulate across transactions until `commit_into`.
    #[inline]
    pub fn tx(&mut self) -> Transaction<'_> {
        Transaction { store: self }
    }
}

#[inline(always)]
fn touch(node: &mut NodeRow, touched: &mut DirtyList, index: NodeIndex, fx: u8) {
    let had = node.effects;
    node.effects = had | fx;
    touched.push_if(index, (had == 0) & (fx != 0));
}

/// Zero-cost validated mutable handle to a single live node (2 registers: `&mut NodeStore` +
/// `NodeIndex`).
///
/// Created via [`Transaction::spawn`] or [`Transaction::node_mut`], validating liveness once so
/// multiple table edits / property writes on the same node skip repeated checks.
pub struct NodeMut<'a> {
    store: &'a mut NodeStore,
    index: NodeIndex,
}

impl<'a> NodeMut<'a> {
    #[inline(always)]
    pub fn index(&self) -> NodeIndex {
        self.index
    }

    #[inline(always)]
    pub fn id(&self) -> NodeId {
        NodeId::from_index(self.index, self.store.nodes[self.index].generation)
    }

    #[inline(always)]
    pub fn as_ref(&self) -> NodeRef<'_> {
        NodeRef::new_unchecked(self.store, self.index)
    }

    /// Fluent typed writer for `T` on this node (skips repeated liveness/generation checks).
    #[inline(always)]
    pub fn edit<T: NveTable>(&mut self) -> T::Mut<'_> {
        T::from_row_writer(writer_at_typed::<T>(self.store, self.index))
    }

    /// Attaches `T` with its defaults (no-op if attached).
    #[inline]
    pub fn attach<T: NveTable>(&mut self) -> &mut Self {
        writer_at_typed::<T>(self.store, self.index);
        self
    }

    /// Sets one scalar property on this node.
    #[inline(always)]
    pub fn set<T: NveTable, V: WordValue>(&mut self, prop: Prop<T, V>, value: V) -> &mut Self {
        writer_at_typed::<T>(self.store, self.index).put(
            prop.word as usize,
            prop.shift as u32,
            prop.mask,
            value.to_bits(),
        );
        self
    }

    /// Sets one string property on this node.
    #[inline]
    pub fn set_str<T: NveTable>(&mut self, prop: StrProp<T>, text: &str) -> &mut Self {
        writer_at_typed::<T>(self.store, self.index).put_str(prop.word as usize, text);
        self
    }

    /// Sets (interning) or clears one resource property on this node.
    #[inline]
    pub fn set_resource<T: NveTable, R: Resource>(
        &mut self,
        prop: ResProp<T, R>,
        value: Option<R>,
    ) -> &mut Self {
        writer_at_typed::<T>(self.store, self.index).put_resource(prop.word as usize, value);
        self
    }

    /// Replaces this node's ordered child list (zero heap allocations).
    #[inline]
    pub fn set_children(&mut self, children: &[NodeId]) -> &mut Self {
        let id = self.id();
        Transaction { store: self.store }.set_children(id, children);
        self
    }

    /// Detaches table `T` from this node.
    #[inline]
    pub fn detach<T: NveTable>(&mut self) -> &mut Self {
        let id = self.id();
        Transaction { store: self.store }.detach_slot(id, T::slot());
        self
    }

    /// Enqueues a typed imperative command on this node.
    #[inline]
    pub fn command<T: NveTable>(&mut self, op: u32, payload: &[u8]) -> &mut Self {
        let id = self.id();
        Transaction { store: self.store }.command_slot(id, T::slot(), op, payload);
        self
    }

    /// Removes this node and its subtree.
    #[inline]
    pub fn remove(self) {
        let id = self.id();
        Transaction { store: self.store }.remove(id);
    }
}

/// Typed writer resolving `T::slot()` and ensuring its column once before mutating words.
#[inline]
fn writer_at_typed<T: NveTable>(store: &mut NodeStore, index: NodeIndex) -> RowWriter<'_> {
    let slot = T::slot();
    store.column_mut_typed::<T>(slot);
    writer_at_existing(store, index, slot)
}

/// Internal writer taking an already-validated `NodeIndex` (zero redundant liveness checks).
#[inline]
fn writer_at(store: &mut NodeStore, index: NodeIndex, slot: TableSlot) -> RowWriter<'_> {
    store.column_mut(slot);
    writer_at_existing(store, index, slot)
}

#[inline]
fn writer_at_existing(store: &mut NodeStore, index: NodeIndex, slot: TableSlot) -> RowWriter<'_> {
    let NodeStore {
        nodes,
        columns,
        text,
        resources,
        touched,
        realization_mask,
        ..
    } = store;
    let node = &mut nodes[index];
    let col = columns[slot.0 as usize].as_mut().expect("column created above");
    if !node.tables.contains(slot) {
        let id = NodeId::from_index(index, node.generation);
        col.attach(id, index);
        node.tables.insert(slot);
        let realize = if realization_mask.contains(slot) {
            effect::REALIZE
        } else {
            0
        };
        touch(node, touched, index, realize | col.desc.effects());
    }
    let row = col.row_of(index);
    let w = col.w;
    RowWriter {
        row: &mut col.rows[row * w..(row + 1) * w],
        mask: &mut col.masks[row],
        dirty: &mut col.dirty,
        node_index: index,
        node,
        touched,
        desc: &col.desc,
        text,
        resources,
    }
}

impl Transaction<'_> {
    /// Read access to the store mid-transaction.
    #[inline]
    pub fn store(&self) -> &NodeStore {
        self.store
    }

    /// Allocates a node (detached, no tables).
    pub fn create(&mut self) -> NodeId {
        let index = self.alloc_slot();
        NodeId::from_index(index, self.store.nodes[index].generation)
    }

    /// Allocates a node and returns a validated [`NodeMut`] handle for chained zero-check edits.
    #[inline]
    pub fn spawn(&mut self) -> NodeMut<'_> {
        let index = self.alloc_slot();
        NodeMut {
            store: self.store,
            index,
        }
    }

    /// Validates `id` once and returns a [`NodeMut`] handle (`None` if dead/stale).
    #[inline(always)]
    pub fn node_mut(&mut self, id: NodeId) -> Option<NodeMut<'_>> {
        let index = id.node_index();
        let r = self.store.nodes.get(index)?;
        if r.flags & LIVE != 0 && r.generation == id.generation() {
            Some(NodeMut {
                store: self.store,
                index,
            })
        } else {
            None
        }
    }

    fn alloc_slot(&mut self) -> NodeIndex {
        let s = &mut *self.store;
        let mut reused = None;
        while let Some(i) = s.free.pop_front() {
            if s.nodes[i].flags & IN_FREE != 0 {
                reused = Some(i);
                break;
            }
        }
        let index = match reused {
            Some(i) => i,
            None => {
                let raw = s.nodes.len() as u32;
                assert!(raw <= NodeId::MAX_INDEX, "NodeStore: node index space exhausted");
                s.nodes.push(NodeRow::default())
            }
        };
        let row = &mut s.nodes[index];
        row.flags = LIVE | CREATED;
        row.primary_slot = TableSlot::NONE;
        let id = NodeId::from_index(index, row.generation);
        s.created.push(id);
        index
    }

    /// Creates `id` exactly (replica stores applying a wire frame). Returns `false` if the slot
    /// is occupied by a live node. O(1) — clears `IN_FREE`/`PENDING_FREE` flags without scanning.
    pub fn create_at(&mut self, id: NodeId) -> bool {
        let s = &mut *self.store;
        let index = id.node_index();
        if index == NodeIndex::NULL {
            return false;
        }
        if s.nodes.len() <= index.as_usize() {
            s.nodes.resize(index.as_usize() + 1, NodeRow::default());
        }
        if s.nodes[index].flags & LIVE != 0 {
            return false;
        }
        let row = &mut s.nodes[index];
        *row = NodeRow {
            generation: id.generation(),
            primary_slot: TableSlot::NONE,
            flags: LIVE | CREATED,
            ..NodeRow::default()
        };
        s.created.push(id);
        true
    }

    /// Removes `id` and its whole subtree, detaching it from its parent (zero heap allocations
    /// once `scratch_stack` is warm). Records `removed` in postorder (descendants before ancestors).
    pub fn remove(&mut self, id: NodeId) {
        if !self.store.is_live(id) {
            return;
        }
        self.detach_child(id);
        let removed_start = self.store.removed.len();
        self.store.scratch_stack.clear();
        self.store.scratch_stack.push(id);
        while let Some(n) = self.store.scratch_stack.pop() {
            let row = self.store.nodes[n.node_index()];
            if row.flags & LIVE == 0 || row.generation != n.generation() {
                continue;
            }
            let children_span = row.children;
            let NodeStore {
                children,
                scratch_stack,
                ..
            } = &mut *self.store;
            scratch_stack.extend_from_slice(children.get(children_span));
            self.free_node(n);
        }
        // Reverse this subtree's recorded removals so descendants precede ancestors.
        self.store.removed[removed_start..].reverse();
    }

    fn free_node(&mut self, id: NodeId) {
        let index = id.node_index();
        let row = self.store.nodes[index];
        debug_assert!(
            row.flags & LIVE != 0 && row.generation == id.generation(),
            "free_node called on non-live or stale node {id:?}"
        );
        if row.flags & LIVE == 0 || row.generation != id.generation() {
            return;
        }
        let tables = row.tables;
        for slot in tables.iter_slots() {
            self.release_row(index, slot);
        }
        let s = &mut *self.store;
        let row = s.nodes[index];
        s.children.free(row.children);
        if row.flags & CREATED == 0 {
            s.removed.push(id);
        }
        s.nodes[index] = NodeRow {
            generation: row.generation,
            flags: PENDING_FREE,
            ..NodeRow::default()
        };
        s.pending_free.push(index);
    }

    /// Releases strings/resources referenced by `slot`'s row and drops the row.
    fn release_row(&mut self, index: NodeIndex, slot: TableSlot) {
        let s = &mut *self.store;
        let Some(col) = s.columns[slot.0 as usize].as_mut() else {
            return;
        };
        let Some(row) = col.find_row(index) else {
            return;
        };
        let (str_words, res_words) = (col.desc.str_words(), col.desc.resource_words());
        let words = &mut col.rows[row * col.w..(row + 1) * col.w];
        let mut m = str_words;
        while m != 0 {
            let w = m.trailing_zeros() as usize;
            m &= m - 1;
            s.text.free(StrSpan::new(words[w], words[w + 1]));
            words[w] = 0;
            words[w + 1] = 0;
        }
        let mut m = res_words;
        while m != 0 {
            let w = m.trailing_zeros() as usize;
            m &= m - 1;
            s.resources.release(ResourceId(words[w]));
            words[w] = 0;
        }
        if col.is_dense() {
            col.masks[row] = 0;
        }
        col.remove_row(index);
    }

    // -----------------------------------------------------------------------------------------
    // Children
    // -----------------------------------------------------------------------------------------

    #[inline]
    fn mark_children(&mut self, parent: NodeIndex) {
        let s = &mut *self.store;
        let row = &mut s.nodes[parent];
        let first = row.flags & CHILDREN_DIRTY == 0;
        row.flags |= CHILDREN_DIRTY;
        s.reparented.push_if(parent, first);
        touch(row, &mut s.touched, parent, effect::LAYOUT);
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
        // Clear previous parent pointers without allocating a temporary Vec.
        {
            let NodeStore {
                nodes, children, ..
            } = &mut *self.store;
            let old_span = nodes[parent_idx].children;
            for &c in children.get(old_span) {
                if let Some(r) = nodes.get_mut(c.node_index())
                    && r.parent == parent
                {
                    r.parent = NodeId::NULL;
                }
            }
        }
        for &c in new_children {
            if let Some(prev) = self.store.parent(c)
                && prev != parent
            {
                self.detach_child(c);
            }
            self.store.nodes[c.node_index()].parent = parent;
        }
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
        self.detach_child(child);
        let parent_idx = parent.node_index();
        let s = &mut *self.store;
        let old_span = s.nodes[parent_idx].children;
        let new_span = s.children.insert_at(old_span, index, child);
        s.nodes[parent_idx].children = new_span;
        s.nodes[child.node_index()].parent = parent;
        self.mark_children(parent_idx);
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
        s.nodes[child.node_index()].parent = NodeId::NULL;
        self.mark_children(parent_idx);
    }

    // -----------------------------------------------------------------------------------------
    // Tables
    // -----------------------------------------------------------------------------------------

    /// Crate-private row writer for `slot` on `id`, attaching the table (defaults) when absent.
    /// Sealed so external callers cannot overwrite string/resource words via raw scalar puts.
    pub(crate) fn writer(&mut self, id: NodeId, slot: TableSlot) -> RowWriter<'_> {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        writer_at(self.store, id.node_index(), slot)
    }

    /// Fluent typed writer for `T` on `id` (attaches `T` with defaults when absent).
    #[inline]
    pub fn edit<T: NveTable>(&mut self, id: NodeId) -> T::Mut<'_> {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        T::from_row_writer(writer_at_typed::<T>(self.store, id.node_index()))
    }

    /// Attaches `T` with its defaults (no-op if attached).
    #[inline]
    pub fn attach<T: NveTable>(&mut self, id: NodeId) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        writer_at_typed::<T>(self.store, id.node_index());
    }

    /// Sets one scalar property.
    #[inline(always)]
    pub fn set<T: NveTable, V: WordValue>(&mut self, id: NodeId, prop: Prop<T, V>, value: V) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        writer_at_typed::<T>(self.store, id.node_index())
            .put(prop.word as usize, prop.shift as u32, prop.mask, value.to_bits());
    }

    /// Sets one string property.
    #[inline]
    pub fn set_str<T: NveTable>(&mut self, id: NodeId, prop: StrProp<T>, text: &str) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        writer_at_typed::<T>(self.store, id.node_index()).put_str(prop.word as usize, text);
    }

    /// Sets (interned) or clears one resource property.
    #[inline]
    pub fn set_resource<T: NveTable, R: Resource>(
        &mut self,
        id: NodeId,
        prop: ResProp<T, R>,
        value: Option<R>,
    ) {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        writer_at_typed::<T>(self.store, id.node_index())
            .put_resource(prop.word as usize, value);
    }

    /// Detaches table `T` from `id`, releasing its strings and resources.
    #[inline]
    pub fn detach<T: NveTable>(&mut self, id: NodeId) {
        self.detach_slot(id, T::slot());
    }

    pub fn detach_slot(&mut self, id: NodeId, slot: TableSlot) {
        if !self.store.has_slot(id, slot) {
            return;
        }
        let index = id.node_index();
        self.release_row(index, slot);
        let s = &mut *self.store;
        let fx = s.columns[slot.0 as usize]
            .as_ref()
            .map_or(0, |c| c.desc.effects());
        let node = &mut s.nodes[index];
        node.tables.remove(slot);
        let created = node.flags & CREATED != 0;
        let realize = if s.realization_mask.contains(slot) {
            effect::REALIZE
        } else {
            0
        };
        touch(node, &mut s.touched, index, realize | fx);
        if !created && let Some(col) = s.columns[slot.0 as usize].as_mut() {
            col.record_detached(id);
        }
    }

    /// Enqueues an imperative command targeting `T` on `id`.
    #[inline]
    pub fn command<T: NveTable>(&mut self, id: NodeId, op: u32, payload: &[u8]) {
        self.command_slot(id, T::slot(), op, payload);
    }

    pub fn command_slot(&mut self, id: NodeId, table: TableSlot, op: u32, payload: &[u8]) {
        let s = &mut *self.store;
        let off = s.command_bytes.len() as u32;
        s.command_bytes.extend_from_slice(payload);
        s.commands.push(Command {
            node: id,
            table,
            op,
            off,
            len: payload.len() as u32,
        });
    }

    pub(crate) fn store_mut(&mut self) -> &mut NodeStore {
        self.store
    }
}
