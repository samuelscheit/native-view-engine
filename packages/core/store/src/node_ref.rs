//! Zero-cost validated read handle ([`NodeRef`]) to a live node in [`NodeStore`].

use nve_abi::table::Word;

use crate::id::{NodeId, NodeIndex, ResourceId, TableSet};
use crate::resource::Resource;
use crate::row::RowRef;
use crate::span_pool::StrSpan;
use crate::store::{NodeRow, NodeStore};
use crate::table::{NveTable, TableSlot};

/// Zero-cost validated read handle to a live node (`Copy`, 2 registers: `&NodeStore` + `NodeIndex`).
///
/// Created via [`NodeStore::get`], which validates bounds, `LIVE`, and generation once; every
/// subsequent read on the handle skips repeated liveness/generation checks.
#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    store: &'a NodeStore,
    index: NodeIndex,
}

impl<'a> NodeRef<'a> {
    #[inline(always)]
    pub(crate) const fn new_unchecked(store: &'a NodeStore, index: NodeIndex) -> Self {
        Self { store, index }
    }

    #[inline(always)]
    pub fn raw_row(self) -> &'a NodeRow {
        &self.store.nodes[self.index]
    }

    #[inline(always)]
    pub fn index(self) -> NodeIndex {
        self.index
    }

    #[inline(always)]
    pub fn id(self) -> NodeId {
        NodeId::from_index(self.index, self.raw_row().generation)
    }

    #[inline(always)]
    pub fn tables(self) -> TableSet {
        self.raw_row().tables
    }

    #[inline(always)]
    pub fn primary_slot(self) -> TableSlot {
        self.raw_row().primary_slot
    }

    #[inline(always)]
    pub fn parent(self) -> Option<NodeId> {
        let p = self.raw_row().parent;
        (!p.is_null()).then_some(p)
    }

    #[inline(always)]
    pub fn children(self) -> &'a [NodeId] {
        self.store.children.get(self.raw_row().children)
    }

    #[inline(always)]
    pub fn has<T: NveTable>(self) -> bool {
        self.raw_row().tables.contains(T::slot())
    }

    #[inline(always)]
    pub fn has_slot(self, slot: TableSlot) -> bool {
        self.raw_row().tables.contains(slot)
    }

    #[inline(always)]
    pub fn words(self, slot: TableSlot) -> Option<&'a [Word]> {
        if !self.has_slot(slot) {
            return None;
        }
        let col = self.store.columns[slot.0 as usize].as_ref()?;
        Some(col.words(col.row_of(self.index)))
    }

    #[inline(always)]
    fn words_for<T: NveTable>(self) -> Option<&'a [Word]> {
        let slot = T::slot();
        let w = self.words(slot)?;
        debug_assert_eq!(
            self.store.columns[slot.0 as usize]
                .as_ref()
                .map(|c| c.desc.key()),
            Some(T::DESC.key),
            "table slot {} key mismatch for `{}`",
            slot.0,
            T::DESC.name
        );
        Some(w)
    }

    #[inline(always)]
    pub fn try_row<T: NveTable>(self) -> Option<T::Ref<'a>> {
        self.words_for::<T>()
            .map(|w| T::from_row_ref(RowRef::new(w, &self.store.text, T::DESC.str_words)))
    }

    #[inline(always)]
    pub fn row<T: NveTable>(self) -> T::Ref<'a> {
        let w = self.words_for::<T>().unwrap_or(T::DESC.defaults);
        T::from_row_ref(RowRef::new(w, &self.store.text, T::DESC.str_words))
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`] (`None` if stale or invalid UTF-8).
    #[inline]
    pub fn try_str(self, span: StrSpan) -> Option<&'a str> {
        self.store.text.try_str(span)
    }

    /// Checked UTF-8 lookup for a detached [`StrSpan`] (returns `""` if stale).
    #[inline]
    pub fn str(self, span: StrSpan) -> &'a str {
        self.store.text.str(span)
    }

    #[inline(always)]
    pub fn resource<R: Resource>(self, id: ResourceId) -> Option<&'a R> {
        self.store.resources.get(id)
    }
}
