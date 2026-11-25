//! Observed-Remove Set (`OrSet<K>`) with add-wins semantics, delta extraction,
//! and idempotent join-semilattice merges.

use std::collections::{BTreeMap, BTreeSet};

use super::hlc::{OpId, VersionVector};

/// Delta payload extracted from an [`OrSet`] relative to a remote [`VersionVector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrSetDelta<K> {
    /// Active `(key, add_op_id)` entries not yet observed by the remote peer.
    pub adds: Vec<(K, OpId)>,
    /// `(removed_tag, remove_op_id)` tombstones not yet observed by the remote peer.
    pub removes: Vec<(OpId, OpId)>,
}

impl<K> Default for OrSetDelta<K> {
    fn default() -> Self {
        Self {
            adds: Vec::new(),
            removes: Vec::new(),
        }
    }
}

impl<K> OrSetDelta<K> {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.adds.is_empty() && self.removes.is_empty()
    }
}

/// Observed-Remove Set with add-wins conflict resolution.
///
/// Each `add(key, op_id)` associates a globally unique [`OpId`] tag with `key`.
/// `remove(key)` tombstones only the tags currently observed for `key`, so a concurrent
/// `add` with a fresh tag wins cleanly upon merge.
#[derive(Debug, Clone)]
pub struct OrSet<K: Ord> {
    /// Active (non-tombstoned) tags per key. Keys with empty tag sets are pruned.
    entries: BTreeMap<K, BTreeSet<OpId>>,
    /// All tombstoned add-tags.
    tombstones: BTreeSet<OpId>,
    /// Earliest known `remove_op` per tombstoned tag for minimal causal delta extraction.
    /// Not part of logical equality: `tombstones` fully determines logical membership.
    tombstone_ops: BTreeMap<OpId, OpId>,
}

impl<K: Ord> Default for OrSet<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + PartialEq> PartialEq for OrSet<K> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.tombstones == other.tombstones
    }
}

impl<K: Ord + Eq> Eq for OrSet<K> {}

impl<K: Ord> OrSet<K> {
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            tombstones: BTreeSet::new(),
            tombstone_ops: BTreeMap::new(),
        }
    }

    /// Adds `key` tagged with unique `op_id`.
    /// Ignored if `op_id` was already tombstoned (out-of-order remove-before-add).
    /// Returns `true` if `op_id` was newly inserted into active tags.
    pub fn add(&mut self, key: K, op_id: OpId) -> bool {
        if self.tombstones.contains(&op_id) {
            return false;
        }
        self.entries.entry(key).or_default().insert(op_id)
    }

    /// Removes `key` by tombstoning every tag currently observed for it.
    /// Returns the tombstoned [`OpId`] tags.
    pub fn remove(&mut self, key: &K) -> Vec<OpId> {
        self.remove_with_op(key, OpId::ZERO)
    }

    /// Removes `key` by tombstoning every tag currently observed for it, associating
    /// `remove_op` so `delta_since` can prune already-seen tombstones.
    pub fn remove_with_op(&mut self, key: &K, remove_op: OpId) -> Vec<OpId> {
        let Some(tags) = self.entries.remove(key) else {
            return Vec::new();
        };
        let mut removed = Vec::with_capacity(tags.len());
        for tag in tags {
            self.record_tombstone(tag, remove_op);
            removed.push(tag);
        }
        removed
    }

    /// Tombstones a single specific `tag` (e.g. from a remote delta).
    /// Returns `true` if `tag` was not previously tombstoned.
    pub fn remove_tag(&mut self, tag: OpId, remove_op: OpId) -> bool {
        let newly_tombstoned = self.record_tombstone(tag, remove_op);
        if newly_tombstoned {
            self.prune_tag(tag);
        }
        newly_tombstoned
    }

    #[inline]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.entries.keys()
    }

    #[inline]
    pub fn active_tags(&self, key: &K) -> Option<&BTreeSet<OpId>> {
        self.entries.get(key)
    }

    #[inline]
    pub fn tombstones(&self) -> &BTreeSet<OpId> {
        &self.tombstones
    }

    fn record_tombstone(&mut self, tag: OpId, remove_op: OpId) -> bool {
        let newly = self.tombstones.insert(tag);
        if remove_op.seq > 0 {
            self.tombstone_ops
                .entry(tag)
                .and_modify(|existing| {
                    if existing.seq == 0 || remove_op < *existing {
                        *existing = remove_op;
                    }
                })
                .or_insert(remove_op);
        }
        newly
    }

    fn prune_tag(&mut self, tag: OpId) {
        let mut any_empty = false;
        for tags in self.entries.values_mut() {
            if tags.remove(&tag) && tags.is_empty() {
                any_empty = true;
            }
        }
        if any_empty {
            self.entries.retain(|_, tags| !tags.is_empty());
        }
    }
}

impl<K: Ord + Clone> OrSet<K> {
    /// Extracts a causal delta containing active adds and tombstones not yet covered by `since`.
    pub fn delta_since(&self, since: &VersionVector) -> OrSetDelta<K> {
        let mut delta = OrSetDelta::default();
        for (key, tags) in &self.entries {
            for &tag in tags {
                if !since.contains(tag) {
                    delta.adds.push((key.clone(), tag));
                }
            }
        }
        for &tag in &self.tombstones {
            let remove_op = self.tombstone_ops.get(&tag).copied().unwrap_or(OpId::ZERO);
            if !since.contains(remove_op) {
                delta.removes.push((tag, remove_op));
            }
        }
        delta
    }

    /// Applies an [`OrSetDelta`] idempotently. Returns `true` if logical state changed.
    pub fn merge_delta(&mut self, delta: &OrSetDelta<K>) -> bool {
        let mut changed = false;
        for &(tag, remove_op) in &delta.removes {
            if self.remove_tag(tag, remove_op) {
                changed = true;
            }
        }
        for (key, tag) in &delta.adds {
            if self.add(key.clone(), *tag) {
                changed = true;
            }
        }
        changed
    }

    /// Full join-semilattice merge with another [`OrSet`].
    /// Commutative, associative, and idempotent.
    pub fn merge(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for &tag in &other.tombstones {
            let remove_op = other.tombstone_ops.get(&tag).copied().unwrap_or(OpId::ZERO);
            if self.remove_tag(tag, remove_op) {
                changed = true;
            }
        }
        for (key, tags) in &other.entries {
            for &tag in tags {
                if self.add(key.clone(), tag) {
                    changed = true;
                }
            }
        }
        changed
    }
}
