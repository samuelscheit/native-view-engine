//! Validated mutable handle ([`NodeMut`]) for zero-check chained edits on a single live node.

use crate::id::{NodeId, NodeIndex, ResourceId};
use crate::resource::FlatResource;
use crate::store::{NodeRef, NodeStore};
use crate::table::{NveTable, Prop, ResProp, ScalarField, StrProp};
use crate::tx::{writer_at_typed, TxCore};

/// Temporary handles kept inline before spilling to the heap (typical nested resources hold a few).
const INLINE_TEMPS: usize = 8;

/// Scoped RAII arena for temporary child resources interned while assembling parent resources
/// (e.g. `BackgroundFillRes` / `GridTracks` referenced inside `StateRules` / `ResponsiveRules`).
///
/// Releases every temporary handle automatically on drop so children owned by a newly interned
/// parent drop cleanly to `refs == 1` and abandoned temporaries are reclaimed at `end_frame`.
pub struct TempResources<'a> {
    store: &'a mut NodeStore,
    /// Inline temporaries in insertion order; free slots hold [`ResourceId::NONE`].
    inline: [ResourceId; INLINE_TEMPS],
    spill: Vec<ResourceId>,
}

impl<'a> TempResources<'a> {
    #[inline]
    pub(crate) fn new(store: &'a mut NodeStore) -> Self {
        Self {
            store,
            inline: [ResourceId::NONE; INLINE_TEMPS],
            spill: Vec::new(),
        }
    }

    #[inline]
    fn track(&mut self, id: ResourceId) {
        if id.is_none() {
            return;
        }
        match self.inline.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = id,
            None => self.spill.push(id),
        }
    }

    /// Interns `value`, scheduling its temporary handle for automatic release when this scope ends.
    #[inline]
    pub fn intern<R: FlatResource>(&mut self, value: &R) -> ResourceId {
        let id = self.store.resources.intern(value);
        self.track(id);
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
        self.track(id);
        id
    }

    /// Reborrows a non-committing [`TxCore`] so callers can attach parent resources referencing
    /// temporary ids; the frame cannot be closed while temporaries are still retained:
    ///
    /// ```compile_fail
    /// let mut store = nve_store::NodeStore::new();
    /// let mut tx = store.tx();
    /// tx.with_temp_resources(|scope| {
    ///     scope.tx().commit_into(&mut nve_store::ChangeSet::default());
    /// });
    /// ```
    #[inline]
    pub fn tx(&mut self) -> TxCore<'_> {
        TxCore { store: self.store }
    }
}

impl Drop for TempResources<'_> {
    fn drop(&mut self) {
        while let Some(id) = self.spill.pop() {
            self.store.resources.release(id);
        }
        for &id in self.inline.iter().rev().filter(|id| !id.is_none()) {
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
    pub(crate) store: &'a mut NodeStore,
    pub(crate) index: NodeIndex,
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
        // Reject before attaching `T` so an invalid id never realizes the row.
        if !rid.is_none() && !self.store.resources.is_live_of::<R>(rid) {
            return self;
        }
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
        TxCore { store: self.store }.set_children(id, children);
        self
    }

    /// Detaches table `T` from this node.
    #[inline]
    pub fn detach<T: NveTable>(&mut self) -> &mut Self {
        let id = self.id();
        TxCore { store: self.store }.detach_slot(id, T::slot());
        self
    }

    /// Enqueues a typed imperative command on this node.
    #[inline]
    pub fn command<T: NveTable>(&mut self, op: u32, payload: &[u8]) -> &mut Self {
        let id = self.id();
        TxCore { store: self.store }.command_slot(id, T::slot(), op, payload);
        self
    }

    /// Removes this node and its subtree.
    #[inline]
    pub fn remove(self) {
        let id = self.id();
        TxCore { store: self.store }.remove(id);
    }
}
