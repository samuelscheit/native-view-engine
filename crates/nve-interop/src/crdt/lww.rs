//! Last-Write-Wins Register (`LwwRegister`) and convergent counters (`GCounter`, `PnCounter`).
//!
//! Every merge operation forms a join-semilattice: commutative, associative, and idempotent.

use std::collections::BTreeMap;

use super::hlc::{HlcTimestamp, PeerId};

/// Last-Write-Wins register ordered by [`HlcTimestamp`].
///
/// Ties on identical timestamps leave the existing value untouched, preserving strict
/// idempotence; distinct peers/writes always produce strictly ordered [`HlcTimestamp`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwwRegister<T> {
    value: T,
    timestamp: HlcTimestamp,
}

impl<T> LwwRegister<T> {
    #[inline]
    pub const fn new(value: T, timestamp: HlcTimestamp) -> Self {
        Self { value, timestamp }
    }

    #[inline]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[inline]
    pub const fn timestamp(&self) -> HlcTimestamp {
        self.timestamp
    }

    #[inline]
    pub fn into_value(self) -> T {
        self.value
    }

    /// Applies a write if `timestamp` is strictly greater than the current timestamp.
    /// Returns `true` when the register updated.
    #[inline]
    pub fn set(&mut self, value: T, timestamp: HlcTimestamp) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }

    /// Merges an owned remote register by strictly higher timestamp.
    #[inline]
    pub fn merge_owned(&mut self, other: Self) -> bool {
        self.set(other.value, other.timestamp)
    }
}

impl<T: Clone> LwwRegister<T> {
    /// Merges a borrowed remote register by strictly higher timestamp.
    #[inline]
    pub fn merge(&mut self, other: &Self) -> bool {
        if other.timestamp > self.timestamp {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
            true
        } else {
            false
        }
    }
}

impl<T: Default> Default for LwwRegister<T> {
    fn default() -> Self {
        Self::new(T::default(), HlcTimestamp::ZERO)
    }
}

/// Grow-only state-based counter tracking a monotonic total per [`PeerId`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GCounter {
    counts: BTreeMap<PeerId, u64>,
}

impl GCounter {
    #[inline]
    pub fn new() -> Self {
        Self {
            counts: BTreeMap::new(),
        }
    }

    /// Increments `peer`'s slot by `delta` (saturating on `u64::MAX`).
    #[inline]
    pub fn increment(&mut self, peer: PeerId, delta: u64) -> u64 {
        if delta == 0 {
            return self.peer_count(peer);
        }
        let slot = self.counts.entry(peer).or_insert(0);
        *slot = slot.saturating_add(delta);
        *slot
    }

    /// Monotonically raises `peer`'s slot to `max(current, count)`. Returns `true` if raised.
    #[inline]
    pub fn set_peer_count(&mut self, peer: PeerId, count: u64) -> bool {
        if count == 0 {
            return false;
        }
        let slot = self.counts.entry(peer).or_insert(0);
        if count > *slot {
            *slot = count;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn peer_count(&self, peer: PeerId) -> u64 {
        self.counts.get(&peer).copied().unwrap_or(0)
    }

    /// Total counter value summed across all peers (saturating on `u64::MAX`).
    #[inline]
    pub fn value(&self) -> u64 {
        self.counts
            .values()
            .fold(0u64, |acc, &v| acc.saturating_add(v))
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (PeerId, u64)> + '_ {
        self.counts.iter().map(|(&p, &v)| (p, v))
    }

    /// Pointwise maximum merge across all peers (commutative, associative, idempotent).
    pub fn merge(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (&peer, &count) in &other.counts {
            if self.set_peer_count(peer, count) {
                changed = true;
            }
        }
        changed
    }
}

/// Positive-Negative counter combining two monotonic [`GCounter`]s so concurrent
/// increments and decrements (such as reactions or votes) sum without clobbering.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PnCounter {
    pos: GCounter,
    neg: GCounter,
}

impl PnCounter {
    #[inline]
    pub fn new() -> Self {
        Self {
            pos: GCounter::new(),
            neg: GCounter::new(),
        }
    }

    #[inline]
    pub fn increment(&mut self, peer: PeerId, delta: u64) {
        self.pos.increment(peer, delta);
    }

    #[inline]
    pub fn decrement(&mut self, peer: PeerId, delta: u64) {
        self.neg.increment(peer, delta);
    }

    /// Applies an increment when `delta > 0` or a decrement when `delta < 0`.
    #[inline]
    pub fn apply_delta(&mut self, peer: PeerId, delta: i64) {
        if delta > 0 {
            self.increment(peer, delta as u64);
        } else if delta < 0 {
            self.decrement(peer, delta.unsigned_abs());
        }
    }

    /// Monotonically merges a single peer's `(pos, neg)` slots. Returns `true` if either raised.
    #[inline]
    pub fn merge_peer(&mut self, peer: PeerId, pos: u64, neg: u64) -> bool {
        let p = self.pos.set_peer_count(peer, pos);
        let n = self.neg.set_peer_count(peer, neg);
        p || n
    }

    #[inline]
    pub fn peer_counts(&self, peer: PeerId) -> (u64, u64) {
        (self.pos.peer_count(peer), self.neg.peer_count(peer))
    }

    /// Signed counter value `pos - neg`, clamped to `i64::MIN..=i64::MAX`.
    #[inline]
    pub fn value(&self) -> i64 {
        let diff = i128::from(self.pos.value()) - i128::from(self.neg.value());
        diff.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Clamped non-negative `u32` projection (e.g., reaction badge counts).
    #[inline]
    pub fn clamped_u32(&self) -> u32 {
        self.value().clamp(0, i64::from(u32::MAX)) as u32
    }

    #[inline]
    pub const fn pos(&self) -> &GCounter {
        &self.pos
    }

    #[inline]
    pub const fn neg(&self) -> &GCounter {
        &self.neg
    }

    /// Iterates over every peer with a non-zero positive or negative slot in deterministic order.
    pub fn peer_entries(&self) -> Vec<(PeerId, u64, u64)> {
        let mut peers = BTreeMap::<PeerId, (u64, u64)>::new();
        for (p, v) in self.pos.iter() {
            peers.entry(p).or_default().0 = v;
        }
        for (p, v) in self.neg.iter() {
            peers.entry(p).or_default().1 = v;
        }
        peers.into_iter().map(|(p, (pos, neg))| (p, pos, neg)).collect()
    }

    /// Pointwise merge of both positive and negative counters.
    pub fn merge(&mut self, other: &Self) -> bool {
        let p = self.pos.merge(&other.pos);
        let n = self.neg.merge(&other.neg);
        p || n
    }
}
