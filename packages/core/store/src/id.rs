//! Strongly typed generational node, dense slot index, table-set, and resource identities.

use core::marker::PhantomData;
use core::ops::{Index, IndexMut};

use crate::table::TableSlot;

/// Dense slot index into [`crate::NodeStore`]'s node table (0 is reserved for null).
///
/// Distinct from [`NodeId`] (which packs `index:24 | generation:8`) so internal slot-index lists
/// (`free`, `pending_free`, `touched`, `reparented`) cannot be accidentally mixed with generational
/// `NodeId` lists (`created`, `removed`) at compile time.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NodeIndex(pub u32);

impl NodeIndex {
    pub const NULL: Self = Self(0);
    pub const ROOT: Self = Self(1);

    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Bitset of attached table slots on a node (up to 128 tables).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TableSet(pub u128);

impl TableSet {
    pub const EMPTY: Self = Self(0);

    #[inline(always)]
    pub const fn raw(self) -> u128 {
        self.0
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn contains(self, slot: TableSlot) -> bool {
        (self.0 & slot.bit()) != 0
    }

    #[inline(always)]
    pub fn insert(&mut self, slot: TableSlot) {
        self.0 |= slot.bit();
    }

    #[inline(always)]
    pub fn remove(&mut self, slot: TableSlot) {
        self.0 &= !slot.bit();
    }

    /// Iterates every set [`TableSlot`] in ascending slot order.
    #[inline]
    pub const fn iter_slots(self) -> TableSetIter {
        TableSetIter(self.0)
    }
}

/// Zero-allocation iterator over set bits in a [`TableSet`].
#[derive(Debug, Clone, Copy)]
pub struct TableSetIter(u128);

impl Iterator for TableSetIter {
    type Item = TableSlot;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0 == 0 {
            return None;
        }
        let tz = self.0.trailing_zeros() as u8;
        self.0 &= self.0 - 1;
        Some(TableSlot(tz))
    }
}

/// Trait for dense `u32`-backed index newtypes usable with [`IndexVec`].
pub trait Idx: Copy {
    fn as_usize(self) -> usize;
    fn from_usize(v: usize) -> Self;
}

impl Idx for NodeIndex {
    #[inline(always)]
    fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    fn from_usize(v: usize) -> Self {
        Self(v as u32)
    }
}

/// Lightweight typed `Vec<T>` indexed exclusively by `I` (zero runtime overhead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexVec<I: Idx, T> {
    raw: Vec<T>,
    _marker: PhantomData<fn() -> I>,
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Idx, T> IndexVec<I, T> {
    #[inline]
    pub const fn new() -> Self {
        Self {
            raw: Vec::new(),
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn from_vec(raw: Vec<T>) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.raw.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    #[inline(always)]
    pub fn get(&self, idx: I) -> Option<&T> {
        self.raw.get(idx.as_usize())
    }

    #[inline(always)]
    pub fn get_mut(&mut self, idx: I) -> Option<&mut T> {
        self.raw.get_mut(idx.as_usize())
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.raw.reserve(additional);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.raw.capacity()
    }

    #[inline]
    pub fn try_reserve(
        &mut self,
        additional: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.raw.try_reserve(additional)
    }

    #[inline]
    pub fn try_reserve_exact(
        &mut self,
        additional: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.raw.try_reserve_exact(additional)
    }

    #[inline]
    pub fn push(&mut self, value: T) -> I {
        let idx = I::from_usize(self.raw.len());
        self.raw.push(value);
        idx
    }

    #[inline]
    pub fn resize(&mut self, new_len: usize, value: T)
    where
        T: Clone,
    {
        self.raw.resize(new_len, value);
    }

    #[inline]
    pub fn truncate(&mut self, len: usize) {
        self.raw.truncate(len);
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.raw
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.raw
    }
}

impl<I: Idx, T> Index<I> for IndexVec<I, T> {
    type Output = T;

    #[inline(always)]
    fn index(&self, idx: I) -> &Self::Output {
        &self.raw[idx.as_usize()]
    }
}

impl<I: Idx, T> IndexMut<I> for IndexVec<I, T> {
    #[inline(always)]
    fn index_mut(&mut self, idx: I) -> &mut Self::Output {
        &mut self.raw[idx.as_usize()]
    }
}

/// Node identity: `index: 24 | generation: 8`. Raw value `0` is the null node.
///
/// Indices are recycled only at frame boundaries, so an id observed during a frame never aliases a
/// different live node in the same frame; the generation catches stale ids across frames.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const NULL: Self = Self(0);
    /// The root node of every surface (index 1, generation 0).
    pub const ROOT: Self = Self(1);
    pub const INDEX_BITS: u32 = 24;
    pub const MAX_INDEX: u32 = (1 << Self::INDEX_BITS) - 1;

    #[inline(always)]
    pub const fn new(index: u32, generation: u8) -> Self {
        Self((generation as u32) << Self::INDEX_BITS | (index & Self::MAX_INDEX))
    }

    /// Rebuilds an id from its packed wire / DOM value ([`Self::raw`]).
    #[inline(always)]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[inline(always)]
    pub const fn from_index(index: NodeIndex, generation: u8) -> Self {
        Self::new(index.0, generation)
    }

    #[inline(always)]
    pub const fn node_index(self) -> NodeIndex {
        NodeIndex(self.0 & Self::MAX_INDEX)
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        (self.0 & Self::MAX_INDEX) as usize
    }

    #[inline(always)]
    pub const fn generation(self) -> u8 {
        (self.0 >> Self::INDEX_BITS) as u8
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Handle of a variable-length blob in the `ResourceStore`. Raw `0` means "none".
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ResourceId(pub u32);

impl ResourceId {
    pub const NONE: Self = Self(0);

    #[inline(always)]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}
