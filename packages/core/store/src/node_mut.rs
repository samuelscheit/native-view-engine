//! Zero-cost validated mutable handle ([`NodeMut`]) to a single live node in [`NodeStore`].

use crate::id::{NodeId, NodeIndex};
use crate::resource::Resource;
use crate::store::{NodeRef, NodeStore};
use crate::table::{NveTable, Prop, ResProp, StrProp};
use crate::tx::{Transaction, writer_at_typed};
use crate::value::WordValue;

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
