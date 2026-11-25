//! Dense slot table with sparse `HashMap` fallback for O(1) `NodeId` indexing,
//! plus a generation-stamped membership table for zero-allocation cycle/duplicate checks.

use std::collections::HashMap;

use nve_abi::NodeId;

/// Node IDs below this threshold are stored directly in a flat `Vec` for cache-friendly O(1)
/// lookups without hashing. Arbitrary/fuzzed high IDs (up to `u32::MAX`) spill into a sparse
/// `HashMap` so malicious or sparse IDs cannot OOM the process.
pub const DENSE_SLOT_LIMIT: usize = 65_536;

/// Hybrid dense-slot / sparse-fallback map keyed by [`NodeId`].
#[derive(Debug, Clone)]
pub struct NodeSlotMap<V> {
    dense: Vec<Option<(NodeId, V)>>,
    sparse: HashMap<NodeId, V>,
    len: usize,
}

impl<V> Default for NodeSlotMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> NodeSlotMap<V> {
    #[inline]
    pub fn new() -> Self {
        Self {
            dense: Vec::new(),
            sparse: HashMap::new(),
            len: 0,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            dense: Vec::with_capacity(cap.min(DENSE_SLOT_LIMIT)),
            sparse: HashMap::new(),
            len: 0,
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
    pub fn clear(&mut self) {
        self.dense.clear();
        self.sparse.clear();
        self.len = 0;
    }

    #[inline]
    pub fn contains_key(&self, id: NodeId) -> bool {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            self.dense.get(idx).is_some_and(Option::is_some)
        } else {
            self.sparse.contains_key(&id)
        }
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> Option<&V> {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            self.dense.get(idx)?.as_ref().map(|(_, v)| v)
        } else {
            self.sparse.get(&id)
        }
    }

    #[inline]
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut V> {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            self.dense.get_mut(idx)?.as_mut().map(|(_, v)| v)
        } else {
            self.sparse.get_mut(&id)
        }
    }

    #[inline]
    pub fn insert(&mut self, id: NodeId, value: V) -> Option<V> {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            if idx >= self.dense.len() {
                self.dense.resize_with(idx + 1, || None);
            }
            let prev = self.dense[idx].replace((id, value)).map(|(_, v)| v);
            if prev.is_none() {
                self.len += 1;
            }
            prev
        } else {
            let prev = self.sparse.insert(id, value);
            if prev.is_none() {
                self.len += 1;
            }
            prev
        }
    }

    #[inline]
    pub fn remove(&mut self, id: NodeId) -> Option<V> {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            let prev = self.dense.get_mut(idx)?.take().map(|(_, v)| v);
            if prev.is_some() {
                self.len -= 1;
            }
            prev
        } else {
            let prev = self.sparse.remove(&id);
            if prev.is_some() {
                self.len -= 1;
            }
            prev
        }
    }

    /// Iterates over all present keys (`&NodeId`), compatible with `HashMap::keys().copied()`.
    pub fn keys(&self) -> impl Iterator<Item = &NodeId> {
        self.dense
            .iter()
            .filter_map(|slot| slot.as_ref().map(|(id, _)| id))
            .chain(self.sparse.keys())
    }

    /// Iterates over all present values.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.dense
            .iter()
            .filter_map(|slot| slot.as_ref().map(|(_, v)| v))
            .chain(self.sparse.values())
    }

    /// Iterates mutably over all present values.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.dense
            .iter_mut()
            .filter_map(|slot| slot.as_mut().map(|(_, v)| v))
            .chain(self.sparse.values_mut())
    }

    /// Iterates over all `(&NodeId, &V)` entries.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeId, &V)> {
        self.dense
            .iter()
            .filter_map(|slot| slot.as_ref().map(|(id, v)| (id, v)))
            .chain(self.sparse.iter())
    }
}

/// Monotonic generation-stamp table for O(1) zero-allocation membership checks across traversals.
#[derive(Debug, Default, Clone)]
pub(crate) struct VisitStamps {
    dense: Vec<u64>,
    sparse: HashMap<NodeId, u64>,
    epoch: u64,
}

impl VisitStamps {
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Advances to a fresh generation stamp in O(1) (clearing only on `u64` wraparound).
    #[inline]
    pub(crate) fn next_gen(&mut self) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.dense.fill(0);
            self.sparse.clear();
            self.epoch = 1;
        }
        self.epoch
    }

    /// Marks `id` with `stamp`. Returns `true` if `id` was not previously marked with `stamp`.
    #[inline]
    pub(crate) fn mark(&mut self, id: NodeId, stamp: u64) -> bool {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            if idx >= self.dense.len() {
                self.dense.resize(idx + 1, 0);
            }
            let slot = &mut self.dense[idx];
            if *slot == stamp {
                return false;
            }
            *slot = stamp;
            true
        } else {
            let slot = self.sparse.entry(id).or_insert(0);
            if *slot == stamp {
                return false;
            }
            *slot = stamp;
            true
        }
    }

    /// Returns `true` if `id` currently holds `stamp`.
    #[inline]
    pub(crate) fn has(&self, id: NodeId, stamp: u64) -> bool {
        let idx = id as usize;
        if idx < DENSE_SLOT_LIMIT {
            self.dense.get(idx).copied() == Some(stamp)
        } else {
            self.sparse.get(&id).copied() == Some(stamp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_and_sparse_slots_behave_identically() {
        let mut map = NodeSlotMap::new();
        assert!(map.is_empty());
        assert_eq!(map.insert(1, "root"), None);
        assert_eq!(map.insert(65_535, "dense_edge"), None);
        assert_eq!(map.insert(65_536, "sparse_edge"), None);
        assert_eq!(map.insert(u32::MAX, "max"), None);
        assert_eq!(map.len(), 4);

        assert_eq!(map.get(1), Some(&"root"));
        assert_eq!(map.get(65_535), Some(&"dense_edge"));
        assert_eq!(map.get(65_536), Some(&"sparse_edge"));
        assert_eq!(map.get(u32::MAX), Some(&"max"));
        assert_eq!(map.get(42), None);

        assert_eq!(map.insert(1, "root2"), Some("root"));
        assert_eq!(map.len(), 4);

        let mut keys: Vec<NodeId> = map.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 65_535, 65_536, u32::MAX]);

        assert_eq!(map.remove(65_535), Some("dense_edge"));
        assert_eq!(map.remove(65_535), None);
        assert_eq!(map.remove(u32::MAX), Some("max"));
        assert_eq!(map.len(), 2);
        assert!(!map.contains_key(65_535));
        assert!(!map.contains_key(u32::MAX));
    }

    #[test]
    fn visit_stamps_handle_dense_and_sparse_without_collision() {
        let mut stamps = VisitStamps::new();
        let g1 = stamps.next_gen();
        assert!(stamps.mark(1, g1));
        assert!(!stamps.mark(1, g1));
        assert!(stamps.mark(u32::MAX, g1));
        assert!(!stamps.mark(u32::MAX, g1));
        assert!(stamps.has(1, g1));
        assert!(stamps.has(u32::MAX, g1));
        assert!(!stamps.has(2, g1));

        let g2 = stamps.next_gen();
        assert!(!stamps.has(1, g2));
        assert!(!stamps.has(u32::MAX, g2));
        assert!(stamps.mark(1, g2));
        assert!(stamps.mark(u32::MAX, g2));
    }
}
