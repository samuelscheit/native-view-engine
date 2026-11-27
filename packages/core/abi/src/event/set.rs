//! [`EventEnum`] trait and typed 32-bit [`EventSet`] bitset stored in table `events_word`s.

use core::fmt;
use core::marker::PhantomData;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

use crate::id::EventId;

/// Table-scoped event enum (`#[repr(u8)]`, contiguous `0..COUNT`, `1 <= COUNT <= 32`).
///
/// Derived automatically via `#[derive(EventEnum)]` and attached to tables via
/// `#[nve_table(..., events = MyEvent)]`.
pub trait EventEnum: Copy + Eq + 'static {
    /// Number of distinct events (`1..=32`).
    const COUNT: usize;
    /// Every variant in discriminant order `0..COUNT`.
    const ALL: &'static [Self];

    /// Table-scoped event discriminant (`0..COUNT`).
    fn event_id(self) -> EventId;

    /// Decodes a table-scoped [`EventId`], returning `None` if out of range.
    fn from_event_id(id: EventId) -> Option<Self>;

    /// Single-bit listener mask (`1u32 << self.event_id().raw()`).
    #[inline(always)]
    fn bit(self) -> u32 {
        1u32 << self.event_id().raw()
    }
}

/// Typed 32-bit listener bitset for one table's [`EventEnum`] stored directly in the table's
/// `events_word` (`word 0`).
#[repr(transparent)]
pub struct EventSet<E: EventEnum>(u32, PhantomData<fn() -> E>);

impl<E: EventEnum> EventSet<E> {
    /// Empty listener set (no events subscribed).
    pub const EMPTY: Self = Self(0, PhantomData);

    /// Bitmask of all valid discriminants `0..E::COUNT`.
    pub const VALID_MASK: u32 = if E::COUNT >= 32 {
        u32::MAX
    } else {
        (1u32 << E::COUNT) - 1
    };

    /// Listener set containing every variant of `E`.
    pub const ALL: Self = Self(Self::VALID_MASK, PhantomData);

    /// Creates an [`EventSet`] from a raw `u32` mask, masking away undefined bits `>= E::COUNT`.
    #[inline(always)]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw & Self::VALID_MASK, PhantomData)
    }

    /// Raw `u32` bitmask.
    #[inline(always)]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Single-event listener set.
    #[inline(always)]
    pub fn one(ev: E) -> Self {
        Self(ev.bit() & Self::VALID_MASK, PhantomData)
    }

    /// Single-event listener set from a raw discriminant (`const`-compatible).
    #[inline(always)]
    pub const fn from_bit_index(idx: u32) -> Self {
        if idx < 32 {
            Self((1u32 << idx) & Self::VALID_MASK, PhantomData)
        } else {
            Self::EMPTY
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    #[inline(always)]
    pub fn contains(self, ev: E) -> bool {
        (self.0 & ev.bit()) != 0
    }

    #[inline(always)]
    pub const fn contains_id(self, id: EventId) -> bool {
        id.raw() < 32 && (self.0 & (1u32 << id.raw())) != 0
    }

    #[inline(always)]
    pub const fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    #[inline(always)]
    pub fn with(self, ev: E) -> Self {
        Self((self.0 | ev.bit()) & Self::VALID_MASK, PhantomData)
    }

    #[inline(always)]
    pub fn without(self, ev: E) -> Self {
        Self(self.0 & !ev.bit(), PhantomData)
    }

    #[inline(always)]
    pub const fn union(self, other: Self) -> Self {
        Self((self.0 | other.0) & Self::VALID_MASK, PhantomData)
    }

    #[inline(always)]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0, PhantomData)
    }

    #[inline(always)]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0, PhantomData)
    }

    #[inline]
    pub const fn iter(self) -> EventSetIter<E> {
        EventSetIter {
            bits: self.0,
            _marker: PhantomData,
        }
    }
}

impl<E: EventEnum> Clone for EventSet<E> {
    #[inline(always)]
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: EventEnum> Copy for EventSet<E> {}

impl<E: EventEnum> Default for EventSet<E> {
    #[inline(always)]
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<E: EventEnum> PartialEq for EventSet<E> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<E: EventEnum> Eq for EventSet<E> {}

impl<E: EventEnum> core::hash::Hash for EventSet<E> {
    #[inline(always)]
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<E: EventEnum + fmt::Debug> fmt::Debug for EventSet<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<E: EventEnum> BitOr for EventSet<E> {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl<E: EventEnum> BitOr<E> for EventSet<E> {
    type Output = Self;
    #[inline(always)]
    fn bitor(self, rhs: E) -> Self {
        self.with(rhs)
    }
}

impl<E: EventEnum> BitOrAssign for EventSet<E> {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 = (self.0 | rhs.0) & Self::VALID_MASK;
    }
}

impl<E: EventEnum> BitOrAssign<E> for EventSet<E> {
    #[inline(always)]
    fn bitor_assign(&mut self, rhs: E) {
        self.0 = (self.0 | rhs.bit()) & Self::VALID_MASK;
    }
}

impl<E: EventEnum> BitAnd for EventSet<E> {
    type Output = Self;
    #[inline(always)]
    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

impl<E: EventEnum> BitAndAssign for EventSet<E> {
    #[inline(always)]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

/// Iterator over set events in an [`EventSet`].
#[derive(Clone, Copy)]
pub struct EventSetIter<E: EventEnum> {
    bits: u32,
    _marker: PhantomData<fn() -> E>,
}

impl<E: EventEnum> Iterator for EventSetIter<E> {
    type Item = E;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.bits != 0 {
            let bit = self.bits.trailing_zeros();
            self.bits &= self.bits - 1;
            if let Some(ev) = E::from_event_id(EventId(bit)) {
                return Some(ev);
            }
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bits.count_ones() as usize;
        (n, Some(n))
    }
}

impl<E: EventEnum> ExactSizeIterator for EventSetIter<E> {}
