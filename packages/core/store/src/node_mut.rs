//! Validated mutable handle ([`NodeMut`]) for zero-check chained edits on a single live node.

use crate::id::{NodeId, NodeIndex, ResourceId};
use crate::resource::{FlatResource, ResourceStore};
use crate::row_patch::RowPatchView;
use crate::store::{NodeRef, NodeStore};
use crate::table::{NveTable, Prop, ResProp, ScalarField, StrProp, TableSlot};
use crate::tx::{writer_at_typed, Transaction};

/// Scoped RAII arena for temporary child resources interned while assembling parent resources
/// (e.g. `BackgroundFillRes` / `GridTracks` referenced inside `StateRules` / `ResponsiveRules`).
///
/// Releases every temporary handle automatically on drop so children owned by a newly interned
/// parent drop cleanly to `refs == 1` and abandoned temporaries are reclaimed at `end_frame`.
pub struct TempResources<'a> {
    store: &'a mut NodeStore,
    temps: Vec<ResourceId>,
}

impl<'a> TempResources<'a> {
    #[inline]
    pub(crate) fn new(store: &'a mut NodeStore) -> Self {
        Self {
            store,
            temps: Vec::new(),
        }
    }

    /// Interns `value`, scheduling its temporary handle for automatic release when this scope ends.
    #[inline]
    pub fn intern<R: FlatResource>(&mut self, value: &R) -> ResourceId {
        let id = self.store.resources.intern(value);
        if !id.is_none() {
            self.temps.push(id);
        }
        id
    }

    /// Interns via a zero-alloc scratch closure, scheduling its temporary handle for release.
    #[inline]
    pub fn intern_with<R: FlatResource>(
        &mut self,
        write: impl FnOnce(&mut Vec<u32>) -> bool,
    ) -> ResourceId {
        let id = self
            .store
            .resources
            .try_intern_with::<R>(write)
            .unwrap_or(ResourceId::NONE);
        if !id.is_none() {
            self.temps.push(id);
        }
        id
    }

    /// Reborrows a scoped transaction handle (without `commit`/`commit_into`) so callers can
    /// attach parent resources referencing temporary ids without committing before temporaries drop.
    #[inline]
    pub fn tx(&mut self) -> ScopedTx<'_> {
        ScopedTx { store: self.store }
    }
}

/// Non-committing mutation handle borrowed from [`TempResources::tx`].
///
/// Prevents premature `commit` / `commit_into` while temporary resource handles are still retained
/// by [`TempResources`], guaranteeing producer and replica `end_frame` lifecycles stay in lockstep.
pub struct ScopedTx<'a> {
    store: &'a mut NodeStore,
}

impl<'a> ScopedTx<'a> {
    #[inline(always)]
    fn inner(&mut self) -> Transaction<'_> {
        Transaction { store: self.store }
    }

    #[inline(always)]
    pub fn store(&self) -> &NodeStore {
        self.store
    }

    #[inline(always)]
    pub fn resources_mut(&mut self) -> &mut ResourceStore {
        &mut self.store.resources
    }

    #[inline]
    pub fn create(&mut self) -> NodeId {
        self.inner().create()
    }

    #[inline]
    pub fn spawn(&mut self) -> NodeMut<'_> {
        let id = self.inner().create();
        NodeMut::new(self.store, id.node_index())
    }

    #[inline]
    pub fn node_mut(&mut self, id: NodeId) -> Option<NodeMut<'_>> {
        self.store
            .is_live(id)
            .then(|| NodeMut::new(self.store, id.node_index()))
    }

    #[inline]
    pub fn edit<T: NveTable>(&mut self, id: NodeId) -> T::Mut<'_> {
        assert!(self.store.is_live(id), "NodeStore: write to dead node {id:?}");
        T::from_row_writer(writer_at_typed::<T>(self.store, id.node_index()))
    }

    #[inline]
    pub fn attach<T: NveTable>(&mut self, id: NodeId) {
        self.inner().attach::<T>(id);
    }

    #[inline]
    pub fn set<T: NveTable, V: ScalarField>(&mut self, id: NodeId, prop: Prop<T, V>, value: V) {
        self.inner().set(id, prop, value);
    }

    #[inline]
    pub fn set_str<T: NveTable>(&mut self, id: NodeId, prop: StrProp<T>, text: &str) {
        self.inner().set_str(id, prop, text);
    }

    #[inline]
    pub fn set_resource<T: NveTable, R: FlatResource>(
        &mut self,
        id: NodeId,
        prop: ResProp<T, R>,
        value: Option<R>,
    ) {
        self.inner().set_resource(id, prop, value);
    }

    #[inline]
    pub fn set_resource_id<T: NveTable, R: FlatResource>(
        &mut self,
        id: NodeId,
        prop: ResProp<T, R>,
        rid: ResourceId,
    ) -> bool {
        self.inner().set_resource_id(id, prop, rid)
    }

    #[inline]
    pub fn apply_row_patch(&mut self, id: NodeId, patch: RowPatchView<'_>) {
        self.inner().apply_row_patch(id, patch);
    }

    #[inline]
    pub fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        self.inner().set_children(parent, children);
    }

    #[inline]
    pub fn try_set_children(&mut self, parent: NodeId, children: &[NodeId]) -> bool {
        self.inner().try_set_children(parent, children)
    }

    #[inline]
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        self.inner().append_child(parent, child);
    }

    #[inline]
    pub fn detach<T: NveTable>(&mut self, id: NodeId) {
        self.inner().detach::<T>(id);
    }

    #[inline]
    pub fn detach_slot(&mut self, id: NodeId, slot: TableSlot) {
        self.inner().detach_slot(id, slot);
    }

    #[inline]
    pub fn command<T: NveTable>(&mut self, id: NodeId, op: u32, payload: &[u8]) {
        self.inner().command::<T>(id, op, payload);
    }

    #[inline]
    pub fn remove(&mut self, id: NodeId) {
        self.inner().remove(id);
    }
}

impl Drop for TempResources<'_> {
    fn drop(&mut self) {
        while let Some(id) = self.temps.pop() {
            self.store.resources.release(id);
        }
    }
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
    pub(crate) fn new(store: &'a mut NodeStore, index: NodeIndex) -> Self {
        Self { store, index }
    }

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

    /// Sets one scalar or wide-optional property on this node.
    #[inline(always)]
    pub fn set<T: NveTable, V: ScalarField>(&mut self, prop: Prop<T, V>, value: V) -> &mut Self {
        let mut w = writer_at_typed::<T>(self.store, self.index);
        value.write_row(
            &mut w,
            prop.word as usize,
            prop.shift as u32,
            prop.mask,
            prop.val_word as usize,
        );
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
        self
    }

    /// Sets one string property on this node.
    #[inline]
    pub fn set_str<T: NveTable>(&mut self, prop: StrProp<T>, text: &str) -> &mut Self {
        let mut w = writer_at_typed::<T>(self.store, self.index);
        w.put_str(prop.word as usize, text);
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
        self
    }

    /// Sets (interning) or clears one resource property on this node.
    #[inline]
    pub fn set_resource<T: NveTable, R: FlatResource>(
        &mut self,
        prop: ResProp<T, R>,
        value: Option<R>,
    ) -> &mut Self {
        let mut w = writer_at_typed::<T>(self.store, self.index);
        w.put_resource(prop.word as usize, value.as_ref());
        if prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
        self
    }

    /// Points one resource property on this node at a live [`ResourceId`] of type `R` (or
    /// `ResourceId::NONE`). Ignores stale, dead, or mismatched-kind ids.
    #[inline]
    pub fn set_resource_id<T: NveTable, R: FlatResource>(
        &mut self,
        prop: ResProp<T, R>,
        rid: ResourceId,
    ) -> &mut Self {
        let mut w = writer_at_typed::<T>(self.store, self.index);
        if w.put_resource_id::<R>(prop.word as usize, rid) && prop.explicit_bit != u8::MAX {
            w.set_explicit_bit(prop.explicit_bit as u32, prop.field_effects);
        }
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
