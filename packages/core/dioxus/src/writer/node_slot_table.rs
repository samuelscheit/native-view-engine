//! Generic generation-checked dense per-node side table ([`NodeSlotTable`]).
//!
//! Replaces sparse per-node `HashMap<NodeId, T>` tables in `NveMutationWriter` with a dense
//! `NodeIndex`-indexed sparse-set layout:
//! - **Zero hashing / probing**: lookups index `slots[node.node_index()]` directly and verify the
//!   exact `NodeId` (index + 8-bit generation) against `entries[pos].owner == node`.
//! - **Compact for sparse or large `T`**: `slots` stores only 4 bytes per addressable `NodeIndex`
//!   (0 bytes while empty, grown lazily on demand), while `entries` packs only live `(NodeId, T)`
//!   records so large payloads like `TypedStyle` never waste slots on unstyled nodes.
//! - **Zero steady-state allocations**: `remove` swap-removes in `O(1)` without releasing backing
//!   capacity, and `reset` / `reset_with` drains values and frees backing buffers on full reset.

use nve_store::{IndexVec, NodeId, NodeIndex};

/// Sentinel in [`NodeSlotTable::slots`] meaning "no live entry at this node index".
const EMPTY: u32 = u32::MAX;

// Every live entry owns a distinct non-null `NodeIndex` in `1..=NodeId::MAX_INDEX`, so
// `entries.len()` can never reach `EMPTY` (`u32::MAX`) and `entries.get(EMPTY as usize)` is
// unconditionally `None` (one bounds check handles both empty slots and packed bounds).
const _: () = assert!(
    (NodeId::MAX_INDEX as usize) < (EMPTY as usize),
    "packed entry index never collides with EMPTY sentinel"
);

#[derive(Debug)]
struct Entry<T> {
    owner: NodeId,
    value: T,
}

/// Generation-checked dense per-node side table mapping [`NodeId`] → `T`.
#[derive(Debug)]
pub(crate) struct NodeSlotTable<T> {
    /// Dense `NodeIndex` → index in `entries`, or [`EMPTY`].
    slots: IndexVec<NodeIndex, u32>,
    /// Packed live `(NodeId, T)` records (`owner` is never `NodeId::NULL`).
    entries: Vec<Entry<T>>,
}

impl<T> Default for NodeSlotTable<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> NodeSlotTable<T> {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            slots: IndexVec::new(),
            entries: Vec::new(),
        }
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn contains_key(&self, node: NodeId) -> bool {
        self.get(node).is_some()
    }

    #[inline(always)]
    pub(crate) fn get(&self, node: NodeId) -> Option<&T> {
        let &pos = self.slots.get(node.node_index())?;
        let entry = self.entries.get(pos as usize)?;
        (entry.owner == node).then_some(&entry.value)
    }

    #[inline(always)]
    pub(crate) fn get_mut(&mut self, node: NodeId) -> Option<&mut T> {
        let &pos = self.slots.get(node.node_index())?;
        let entry = self.entries.get_mut(pos as usize)?;
        (entry.owner == node).then_some(&mut entry.value)
    }

    /// Inserts `(node, value)`, returning the previous value if `node` (same index and
    /// generation) was already present.
    ///
    /// Rejects [`NodeId::NULL`] without allocating. If a stale generation still occupied
    /// `node.node_index()`, the stale entry is dropped and replaced in place, returning `None`.
    #[inline]
    pub(crate) fn insert(&mut self, node: NodeId, value: T) -> Option<T> {
        if node.is_null() {
            return None;
        }
        let idx = node.node_index();
        if let Some(&pos) = self.slots.get(idx)
            && let Some(entry) = self.entries.get_mut(pos as usize)
        {
            return if entry.owner == node {
                Some(core::mem::replace(&mut entry.value, value))
            } else {
                let old = core::mem::replace(&mut entry.value, value);
                entry.owner = node;
                drop(old);
                None
            };
        }
        self.insert_vacant(idx, node, value);
        None
    }

    /// Returns a mutable reference to `node`'s entry, lazily constructing one via `make` when
    /// absent or when a stale generation occupied the slot. Returns `None` for [`NodeId::NULL`].
    #[inline]
    pub(crate) fn get_or_insert_with(
        &mut self,
        node: NodeId,
        make: impl FnOnce() -> T,
    ) -> Option<&mut T> {
        if node.is_null() {
            return None;
        }
        let idx = node.node_index();
        if let Some(&pos) = self.slots.get(idx)
            && (pos as usize) < self.entries.len()
        {
            let entry = &mut self.entries[pos as usize];
            if entry.owner != node {
                let fresh = make();
                let old = core::mem::replace(&mut entry.value, fresh);
                entry.owner = node;
                drop(old);
            }
            return Some(&mut entry.value);
        }
        let pos = self.insert_vacant(idx, node, make());
        Some(&mut self.entries[pos].value)
    }

    /// Removes `node`'s entry if its exact generation matches, returning its value.
    ///
    /// Leaving out-of-bounds, unoccupied, null, or newer-generation slots completely untouched.
    #[inline]
    pub(crate) fn remove(&mut self, node: NodeId) -> Option<T> {
        let idx = node.node_index();
        let &pos = self.slots.get(idx)?;
        let pos_usize = pos as usize;
        if self.entries.get(pos_usize)?.owner != node {
            return None;
        }
        self.slots[idx] = EMPTY;
        let removed = self.entries.swap_remove(pos_usize);
        if let Some(moved) = self.entries.get(pos_usize) {
            self.slots[moved.owner.node_index()] = pos;
        }
        Some(removed.value)
    }

    /// Drains all live values into `on_remove` and releases all backing `slots` / `entries` memory.
    ///
    /// Detaches `slots` and `entries` before invoking `on_remove` so an unwind inside `on_remove`
    /// or `T::drop` can never leave stale `slots` aliasing newly inserted entries.
    pub(crate) fn reset_with(&mut self, mut on_remove: impl FnMut(T)) {
        self.slots = IndexVec::new();
        let entries = core::mem::take(&mut self.entries);
        for entry in entries {
            on_remove(entry.value);
        }
    }

    #[cfg(test)]
    pub(crate) fn is_deallocated(&self) -> bool {
        self.slots.capacity() == 0 && self.entries.capacity() == 0
    }

    /// Drops all live entries and releases all backing `slots` / `entries` memory.
    #[inline]
    pub(crate) fn reset(&mut self) {
        self.reset_with(|_| {});
    }

    #[inline]
    fn insert_vacant(&mut self, idx: NodeIndex, node: NodeId, value: T) -> usize {
        if self.slots.len() <= idx.as_usize() {
            self.grow_slots(idx);
        }
        let pos = self.entries.len();
        self.entries.push(Entry { owner: node, value });
        self.slots[idx] = pos as u32;
        pos
    }

    #[cold]
    #[inline(never)]
    fn grow_slots(&mut self, idx: NodeIndex) {
        self.slots.resize(idx.as_usize() + 1, EMPTY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_generation_isolation_across_get_insert_get_or_insert_and_remove() {
        let mut table = NodeSlotTable::new();
        let g1 = NodeId::new(5, 1);
        let g2 = NodeId::new(5, 2);
        let g3 = NodeId::new(5, 3);

        assert_eq!(table.insert(g1, 10u32), None);
        assert_eq!(table.get(g1), Some(&10));
        assert_eq!(table.get(g2), None);
        assert_eq!(table.get_mut(g2), None);
        assert!(!table.contains_key(g2));

        // Removing a mismatched generation must not disturb the live entry.
        assert_eq!(table.remove(g2), None);
        assert_eq!(table.get(g1), Some(&10));

        // Inserting a newer generation over an unremoved stale entry replaces it and returns None.
        assert_eq!(table.insert(g2, 20), None);
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(g1), None);
        assert_eq!(table.get(g2), Some(&20));

        // Removing the older generation leaves g2 intact.
        assert_eq!(table.remove(g1), None);
        assert_eq!(table.get(g2), Some(&20));

        // `get_or_insert_with` for a newer generation never exposes g2's stale payload.
        let slot = table.get_or_insert_with(g3, || 30).expect("non-null");
        assert_eq!(*slot, 30);
        *slot = 31;
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(g2), None);
        assert_eq!(table.get(g3), Some(&31));

        // Exact generation remove clears the slot cleanly.
        assert_eq!(table.remove(g3), Some(31));
        assert_eq!(table.remove(g3), None);
        assert!(table.is_empty());
    }

    #[test]
    fn swap_remove_preserves_remaining_entries_and_null_or_oob_never_allocates() {
        let mut table = NodeSlotTable::<String>::new();
        assert_eq!(table.slots.capacity(), 0);
        assert_eq!(table.entries.capacity(), 0);

        // Null and out-of-bounds operations never allocate or panic.
        assert_eq!(table.insert(NodeId::NULL, "bad".into()), None);
        assert!(table.get_or_insert_with(NodeId::NULL, || "bad".into()).is_none());
        assert_eq!(table.get(NodeId::NULL), None);
        assert_eq!(table.remove(NodeId::NULL), None);
        assert_eq!(table.get(NodeId::new(50_000, 7)), None);
        assert_eq!(table.remove(NodeId::new(50_000, 7)), None);
        assert_eq!(table.slots.capacity(), 0);
        assert_eq!(table.entries.capacity(), 0);

        let nodes = [
            NodeId::new(2, 1),
            NodeId::new(7, 4),
            NodeId::new(3, 2),
            NodeId::new(11, 9),
        ];
        for (i, &n) in nodes.iter().enumerate() {
            assert_eq!(table.insert(n, format!("v{i}")), None);
        }
        assert_eq!(table.len(), 4);

        // Remove first, middle, last, and already-removed entries in turn.
        assert_eq!(table.remove(nodes[0]).as_deref(), Some("v0"));
        assert_eq!(table.get(nodes[0]), None);
        assert_eq!(table.get(nodes[3]).map(String::as_str), Some("v3"));
        assert_eq!(table.get(nodes[1]).map(String::as_str), Some("v1"));
        assert_eq!(table.get(nodes[2]).map(String::as_str), Some("v2"));

        assert_eq!(table.remove(nodes[2]).as_deref(), Some("v2"));
        assert_eq!(table.remove(nodes[3]).as_deref(), Some("v3"));
        assert_eq!(table.remove(nodes[1]).as_deref(), Some("v1"));
        assert!(table.is_empty());
        assert_eq!(table.remove(nodes[1]), None);
    }

    #[test]
    fn sparse_high_index_growth_clear_and_reset_memory_discipline() {
        let mut table = NodeSlotTable::<[u64; 32]>::new();
        let high = NodeId::new(100_000, 3);
        let low = NodeId::new(4, 1);

        assert_eq!(table.insert(high, [7; 32]), None);
        assert_eq!(table.insert(low, [9; 32]), None);
        assert_eq!(table.len(), 2);
        // Packed entries only hold the 2 live large payloads, not 100,001 empty copies.
        assert!(table.entries.capacity() < 16);
        assert_eq!(table.slots.len(), 100_001);
        assert_eq!(table.get(high).map(|a| a[0]), Some(7));
        assert_eq!(table.get(low).map(|a| a[0]), Some(9));
        assert_eq!(table.get(NodeId::new(50_000, 1)), None);

        // `reset_with` drains values and frees all backing memory.
        let mut drained = 0usize;
        table.reset_with(|v| {
            assert!(v[0] == 7 || v[0] == 9);
            drained += 1;
        });
        assert_eq!(drained, 2);
        assert!(table.is_empty());
        assert!(table.is_deallocated());
    }

    #[test]
    fn unwind_in_get_or_insert_with_and_reset_with_never_leaks_or_aliases() {
        let g1 = NodeId::new(2, 1);
        let g2 = NodeId::new(2, 2);
        let other = NodeId::new(9, 1);

        let mut table = NodeSlotTable::new();
        table.insert(g1, 111u32);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = table.get_or_insert_with(g2, || panic!("boom"));
        }));
        assert!(panicked.is_err());
        assert_eq!(table.get(g2), None, "g1 payload must never leak under g2");
        assert_eq!(table.get(g1), Some(&111));

        table.insert(other, 222u32);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            table.reset_with(|_| panic!("boom during reset"));
        }));
        assert!(panicked.is_err());
        assert!(table.is_empty());
        assert!(table.is_deallocated());

        // Post-unwind inserts at both indices must never alias each other's entries.
        table.insert(other, 333u32);
        table.insert(g2, 444u32);
        assert_eq!(table.get(other), Some(&333));
        assert_eq!(table.get(g2), Some(&444));
    }

}
