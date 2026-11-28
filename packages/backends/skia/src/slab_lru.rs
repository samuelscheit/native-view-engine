//! Pre-allocated intrusive doubly-linked LRU cache (`SlabLru`) with O(1) touch/eviction,
//! strict `max_entries` + `max_bytes` caps, pluggable `BuildHasher` (`IdentityU64BuildHasher` /
//! `FastBuildHasher`), and zero heap allocations on hit.

use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

use nve_abi::Xxh64Hasher;

const NONE: u32 = u32::MAX;
const MAX_VALID_ENTRIES: usize = (NONE - 1) as usize;

/// Zero-overhead identity hasher for keys that are already uniformly distributed `u64` digests
/// (such as pre-computed `xxh64` hashes).
///
/// Must receive **at most one** `write_u64` / `write_u32` call per hashed key; debug builds
/// assert against accidental multi-field overwrites (use [`FastBuildHasher`] for composite keys).
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityU64Hasher {
    hash: u64,
    #[cfg(debug_assertions)]
    written: bool,
}

impl Hasher for IdentityU64Hasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.hash = nve_abi::xxh64(bytes, self.hash);
        #[cfg(debug_assertions)]
        {
            self.written = true;
        }
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !self.written,
                "IdentityU64Hasher accepts only one pre-hashed u64 write; use FastBuildHasher for multi-field keys"
            );
            self.written = true;
        }
        self.hash = i;
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !self.written,
                "IdentityU64Hasher accepts only one pre-hashed u32 write; use FastBuildHasher for multi-field keys"
            );
            self.written = true;
        }
        self.hash = u64::from(i);
    }
}

/// [`BuildHasher`] producing [`IdentityU64Hasher`] (zero SipHash overhead on pre-hashed keys).
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityU64BuildHasher;

impl BuildHasher for IdentityU64BuildHasher {
    type Hasher = IdentityU64Hasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        IdentityU64Hasher::default()
    }
}

/// Fast non-cryptographic `Hasher` backed by `nve_abi::Xxh64Hasher` (zero SipHash/FNV overhead
/// for small struct cache keys).
#[derive(Debug, Clone)]
pub struct FastKeyHasher(Xxh64Hasher);

impl Default for FastKeyHasher {
    #[inline]
    fn default() -> Self {
        Self(Xxh64Hasher::new(0))
    }
}

impl Hasher for FastKeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0.digest()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0.update_u32(i);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0.update_u64(i);
    }
}

/// [`BuildHasher`] producing [`FastKeyHasher`] (`XXH64`).
#[derive(Debug, Default, Clone, Copy)]
pub struct FastBuildHasher;

impl BuildHasher for FastBuildHasher {
    type Hasher = FastKeyHasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        FastKeyHasher::default()
    }
}

#[derive(Debug)]
struct OccupiedSlot<K, V> {
    key: K,
    value: V,
    bytes: usize,
    prev: u32,
    next: u32,
}

/// Bounded `O(1)` intrusive doubly-linked LRU cache backed by a dense slot slab.
///
/// Lookup (`get_mut` / `peek`) updates recency purely via `u32` slot indices with zero heap
/// allocations. Used across text shaping caches, decoded image LRU, and Skia GPU resource caches.
#[derive(Debug)]
pub struct SlabLru<K, V, S = RandomState> {
    slots: Vec<Option<OccupiedSlot<K, V>>>,
    free: Vec<u32>,
    index: HashMap<K, u32, S>,
    head: u32,
    tail: u32,
    len: usize,
    max_entries: usize,
    max_bytes: usize,
    live_bytes: usize,
    evictions: u64,
}

impl<K, V> SlabLru<K, V, RandomState>
where
    K: Eq + Hash + Clone,
{
    /// Creates an empty [`SlabLru`] bounded by `max_entries` and `max_bytes`.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self::with_initial_capacity(max_entries, max_bytes, 0)
    }

    /// Creates an empty [`SlabLru`] with pre-allocated slot and index storage.
    pub fn with_initial_capacity(max_entries: usize, max_bytes: usize, initial_cap: usize) -> Self {
        Self::with_initial_capacity_and_hasher(
            max_entries,
            max_bytes,
            initial_cap,
            RandomState::new(),
        )
    }
}

impl<K, V, S> SlabLru<K, V, S>
where
    K: Eq + Hash + Clone,
    S: BuildHasher,
{
    /// Creates an empty [`SlabLru`] with pre-allocated storage and an explicit `BuildHasher`.
    pub fn with_initial_capacity_and_hasher(
        max_entries: usize,
        max_bytes: usize,
        initial_cap: usize,
        hash_builder: S,
    ) -> Self {
        let max_entries = max_entries.clamp(1, MAX_VALID_ENTRIES);
        let max_bytes = max_bytes.max(1);
        let cap = initial_cap.min(max_entries);
        Self {
            slots: Vec::with_capacity(cap),
            free: Vec::with_capacity(cap),
            index: HashMap::with_capacity_and_hasher(cap, hash_builder),
            head: NONE,
            tail: NONE,
            len: 0,
            max_entries,
            max_bytes,
            live_bytes: 0,
            evictions: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    #[inline]
    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    #[inline]
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    #[inline]
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    #[inline]
    pub fn slot_len(&self) -> usize {
        self.slots.len()
    }

    /// Iterates over all live values in slot order without mutating recency.
    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.slots.iter().filter_map(|s| s.as_ref().map(|o| &o.value))
    }

    /// Looks up `key`, promoting it to MRU head in O(1) with zero heap allocations.
    #[inline]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.get_slot_and_value_mut(key).map(|(_, v)| v)
    }

    /// Looks up `key`, returning its stable dense slot index and mutable value while promoting it
    /// to MRU head in O(1) with zero heap allocations.
    #[inline]
    pub fn get_slot_and_value_mut<Q>(&mut self, key: &Q) -> Option<(u32, &mut V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = *self.index.get(key)?;
        self.move_to_front(idx);
        self.slots[idx as usize]
            .as_mut()
            .map(|s| (idx, &mut s.value))
    }

    /// Looks up `key` without modifying recency order.
    #[inline]
    pub fn peek<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.peek_slot_and_value(key).map(|(_, v)| v)
    }

    /// Looks up `key`, returning its stable dense slot index and value without modifying recency.
    #[inline]
    pub fn peek_slot_and_value<Q>(&self, key: &Q) -> Option<(u32, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = *self.index.get(key)?;
        self.slots[idx as usize].as_ref().map(|s| (idx, &s.value))
    }

    /// Inserts `(key, value)` accounting for `bytes` under both `max_entries` and `max_bytes`.
    ///
    /// If `bytes > max_bytes`, any previous entry for `key` is removed and the oversized entry is
    /// skipped so existing smaller entries are not wiped. Returns the number of evicted entries.
    pub fn insert(&mut self, key: K, value: V, bytes: usize) -> usize {
        self.insert_with_slot(key, value, bytes).1
    }

    /// Inserts `(key, value)` and returns `(Option<slot>, evicted_count)`. When `key` already
    /// exists and `bytes <= max_bytes`, its existing dense slot index is preserved in-place.
    pub fn insert_with_slot(&mut self, key: K, value: V, bytes: usize) -> (Option<u32>, usize) {
        self.insert_with_slot_and_evict(key, value, bytes, |_| {})
    }

    /// Inserts `(key, value)`, invoking `on_evict(slot_idx)` for every slot cleared by eviction
    /// or oversized replacement. Preserves `key`'s existing dense slot index when overwritten
    /// in-place with `bytes <= max_bytes`.
    pub fn insert_with_slot_and_evict(
        &mut self,
        key: K,
        value: V,
        bytes: usize,
        on_evict: impl FnMut(u32),
    ) -> (Option<u32>, usize) {
        self.insert_with_slot_and_second_chance(key, value, bytes, |_| false, on_evict)
    }

    /// Inserts `(key, value)` with a CLOCK second-chance predicate: before evicting `tail`,
    /// `give_second_chance(tail_idx)` is consulted (up to `self.len` steps); if it returns
    /// `true`, the tail slot is promoted to MRU head instead of evicted, restoring LRU recency
    /// for lock-free fast-path hits.
    pub fn insert_with_slot_and_second_chance(
        &mut self,
        key: K,
        value: V,
        bytes: usize,
        mut give_second_chance: impl FnMut(u32) -> bool,
        mut on_evict: impl FnMut(u32),
    ) -> (Option<u32>, usize) {
        let mut reuse_idx: Option<u32> = None;
        if let Some(existing_idx) = self.index.remove(&key) {
            self.unlink(existing_idx);
            if let Some(old) = self.slots[existing_idx as usize].take() {
                self.live_bytes = self.live_bytes.saturating_sub(old.bytes);
                self.len = self.len.saturating_sub(1);
            }
            if bytes <= self.max_bytes {
                reuse_idx = Some(existing_idx);
            } else {
                self.free.push(existing_idx);
                on_evict(existing_idx);
            }
        }

        if bytes > self.max_bytes {
            return (None, 0);
        }

        // `bytes <= self.max_bytes` guarantees `self.max_bytes - bytes` cannot underflow and
        // stays exact even when `self.max_bytes == usize::MAX`.
        let max_live_before_insert = self.max_bytes - bytes;
        let mut evicted = 0usize;
        while self.len > 0
            && (self.len >= self.max_entries || self.live_bytes > max_live_before_insert)
        {
            // CLOCK second-chance scan: promote recently fast-hit tails up to once per eviction.
            let mut promoted = 0usize;
            while promoted < self.len && self.tail != NONE {
                let candidate = self.tail;
                if give_second_chance(candidate) {
                    self.move_to_front(candidate);
                    promoted += 1;
                } else {
                    break;
                }
            }
            let Some(evicted_idx) = self.evict_tail() else {
                break;
            };
            on_evict(evicted_idx);
            evicted += 1;
        }

        let idx = if let Some(existing) = reuse_idx {
            existing
        } else if let Some(free_idx) = self.free.pop() {
            free_idx
        } else {
            let next_idx = self.slots.len() as u32;
            self.slots.push(None);
            next_idx
        };

        self.slots[idx as usize] = Some(OccupiedSlot {
            key: key.clone(),
            value,
            bytes,
            prev: NONE,
            next: NONE,
        });
        self.index.insert(key, idx);
        self.push_front(idx);
        self.len += 1;
        self.live_bytes += bytes;
        (Some(idx), evicted)
    }

    /// Removes `key` if present, returning its value.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.index.remove(key)?;
        self.unlink(idx);
        let old = self.slots[idx as usize].take()?;
        self.live_bytes = self.live_bytes.saturating_sub(old.bytes);
        self.len = self.len.saturating_sub(1);
        self.free.push(idx);
        Some(old.value)
    }

    /// Drops every entry for which `keep(k, v)` returns `false`, updating `live_bytes` and
    /// `evictions` in place without allocating. Returns the number of dropped entries.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) -> usize {
        let mut dropped = 0usize;
        let mut cursor = self.head;
        while cursor != NONE {
            let (next, should_keep) = match self.slots[cursor as usize].as_ref() {
                Some(slot) => (slot.next, keep(&slot.key, &slot.value)),
                None => break,
            };
            if !should_keep {
                self.unlink(cursor);
                if let Some(old) = self.slots[cursor as usize].take() {
                    self.index.remove(&old.key);
                    self.live_bytes = self.live_bytes.saturating_sub(old.bytes);
                    self.len = self.len.saturating_sub(1);
                    self.free.push(cursor);
                    dropped += 1;
                }
            }
            cursor = next;
        }
        self.evictions = self.evictions.saturating_add(dropped as u64);
        dropped
    }

    /// Drops all entries, returning how many were cleared.
    pub fn clear(&mut self) -> usize {
        let prev_len = self.len;
        let mut cursor = self.head;
        while cursor != NONE {
            let next = self.slots[cursor as usize]
                .as_ref()
                .map_or(NONE, |s| s.next);
            self.slots[cursor as usize] = None;
            self.free.push(cursor);
            cursor = next;
        }
        self.index.clear();
        self.head = NONE;
        self.tail = NONE;
        self.len = 0;
        self.live_bytes = 0;
        prev_len
    }

    fn evict_tail(&mut self) -> Option<u32> {
        let tail_idx = self.tail;
        if tail_idx == NONE {
            return None;
        }
        self.unlink(tail_idx);
        if let Some(old) = self.slots[tail_idx as usize].take() {
            self.index.remove(&old.key);
            self.live_bytes = self.live_bytes.saturating_sub(old.bytes);
            self.len = self.len.saturating_sub(1);
            self.free.push(tail_idx);
            self.evictions = self.evictions.saturating_add(1);
            Some(tail_idx)
        } else {
            None
        }
    }

    #[inline]
    fn move_to_front(&mut self, idx: u32) {
        if self.head == idx {
            return;
        }
        self.unlink(idx);
        self.push_front(idx);
    }

    #[inline]
    fn push_front(&mut self, idx: u32) {
        let old_head = self.head;
        if let Some(slot) = self.slots[idx as usize].as_mut() {
            slot.prev = NONE;
            slot.next = old_head;
        }
        if old_head != NONE
            && let Some(head_slot) = self.slots[old_head as usize].as_mut()
        {
            head_slot.prev = idx;
        }
        self.head = idx;
        if self.tail == NONE {
            self.tail = idx;
        }
    }

    #[inline]
    fn unlink(&mut self, idx: u32) {
        let (prev, next) = match self.slots[idx as usize].as_mut() {
            Some(slot) => {
                let pair = (slot.prev, slot.next);
                slot.prev = NONE;
                slot.next = NONE;
                pair
            }
            None => return,
        };
        if prev != NONE {
            if let Some(p) = self.slots[prev as usize].as_mut() {
                p.next = next;
            }
        } else if self.head == idx {
            self.head = next;
        }
        if next != NONE {
            if let Some(n) = self.slots[next as usize].as_mut() {
                n.prev = prev;
            }
        } else if self.tail == idx {
            self.tail = prev;
        }
    }
}

#[cfg(test)]
#[path = "slab_lru_tests.rs"]
mod tests;
