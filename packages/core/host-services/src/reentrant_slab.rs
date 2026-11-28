//! Re-entrant-safe keyed slot slab (`ReentrantSlotSlab<K, V>`) preventing use-after-free when a
//! callback (e.g., a `wasm-bindgen` `Closure`) unmounts or cancels its own slot while executing.
//!
//! Each inserted entry carries a unique monotonic generation so same-key re-insertions while an
//! older entry is active on the call stack never reap the live replacement, and all eviction
//! methods return evicted `V` values so callers drop them OUTSIDE any `RefCell::borrow_mut()`
//! guard (preventing re-entrant `BorrowMutError` panics).

struct SlabEntry<K, V> {
    generation: u64,
    key: K,
    value: V,
    retired: bool,
}

/// Re-entrant-safe keyed container deferring `Drop` of any slot whose generation is currently
/// executing on the call stack until after the active callback unwinds.
pub struct ReentrantSlotSlab<K, V> {
    next_gen: u64,
    slots: Vec<SlabEntry<K, V>>,
    active_stack: Vec<(K, u64)>,
    has_deferred: bool,
}

impl<K, V> Default for ReentrantSlotSlab<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> ReentrantSlotSlab<K, V> {
    pub const fn new() -> Self {
        Self {
            next_gen: 1,
            slots: Vec::new(),
            active_stack: Vec::new(),
            has_deferred: false,
        }
    }

    /// Total number of slots currently retained (including retired-but-unreaped active slots).
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Iterates over all live (non-retired) `(key, value)` entries.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.slots
            .iter()
            .filter(|e| !e.retired)
            .map(|e| (&e.key, &e.value))
    }
}

impl<K: Copy + Eq, V> ReentrantSlotSlab<K, V> {
    #[inline]
    fn is_gen_active(&self, generation: u64) -> bool {
        self.active_stack.iter().any(|&(_, g)| g == generation)
    }

    /// Returns `true` if any entry for `key` is currently active anywhere on the re-entrancy stack.
    #[inline]
    pub fn is_active(&self, key: K) -> bool {
        self.active_stack.iter().any(|&(k, _)| k == key)
    }

    /// Marks the active entry for `key` as executing on the call stack (supports nested re-entrancy).
    #[inline]
    pub fn enter(&mut self, key: K) {
        let generation = self
            .slots
            .iter()
            .rev()
            .find(|e| e.key == key && !e.retired)
            .or_else(|| self.slots.iter().rev().find(|e| e.key == key))
            .map_or(0, |e| e.generation);
        self.active_stack.push((key, generation));
    }

    /// Pops one execution frame for `key` from the active stack. When `mark_settled` is `true`,
    /// ONLY the popped generation is marked retired so same-key re-insertions stay alive.
    ///
    /// Returns `true` when the active stack is now empty AND deferred drops are waiting to be
    /// reaped (signaling the caller to schedule an asynchronous deferred reap).
    pub fn leave(&mut self, key: K, mark_settled: bool) -> bool {
        if let Some(pos) = self.active_stack.iter().rposition(|&(k, _)| k == key) {
            let (_, generation) = self.active_stack.remove(pos);
            if mark_settled
                && let Some(entry) = self.slots.iter_mut().find(|e| e.generation == generation)
            {
                entry.retired = true;
                self.has_deferred = true;
            }
        }
        self.active_stack.is_empty() && self.has_deferred
    }

    fn collect_reapable(&mut self, out: &mut Vec<V>) {
        let mut i = 0;
        let mut remaining_retired = false;
        while i < self.slots.len() {
            if self.slots[i].retired {
                if self.is_gen_active(self.slots[i].generation) {
                    remaining_retired = true;
                    i += 1;
                } else {
                    out.push(self.slots.swap_remove(i).value);
                }
            } else {
                i += 1;
            }
        }
        self.has_deferred = remaining_retired;
    }

    /// Inserts `(key, value)` with a fresh generation, retiring any prior entry with the same key
    /// and returning evicted inactive values to drop outside any `RefCell` borrow.
    #[must_use]
    pub fn insert(&mut self, key: K, value: V) -> Vec<V> {
        let mut evicted = Vec::new();
        for entry in &mut self.slots {
            if entry.key == key && !entry.retired {
                entry.retired = true;
                self.has_deferred = true;
            }
        }
        if self.has_deferred {
            self.collect_reapable(&mut evicted);
        }
        let generation = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1).max(1);
        self.slots.push(SlabEntry {
            generation,
            key,
            value,
            retired: false,
        });
        evicted
    }

    /// Retires all entries for `key` and returns every inactive retired value so the caller drops
    /// them outside any `RefCell` borrow; active generations remain until unwind + reap.
    #[must_use]
    pub fn remove_or_defer(&mut self, key: K) -> Vec<V> {
        let mut evicted = Vec::new();
        for entry in &mut self.slots {
            if entry.key == key {
                entry.retired = true;
                self.has_deferred = true;
            }
        }
        if self.has_deferred {
            self.collect_reapable(&mut evicted);
        }
        evicted
    }

    /// Retires every live entry matching `pred`, invokes `on_remove` for each matched entry, and
    /// returns `(matched_keys, evicted_values)` so evicted values drop outside any `RefCell` borrow.
    #[must_use]
    pub fn remove_where(
        &mut self,
        mut pred: impl FnMut(K, &V) -> bool,
        mut on_remove: impl FnMut(K, &V),
    ) -> (Vec<K>, Vec<V>) {
        let mut evicted = Vec::new();
        if self.has_deferred {
            self.collect_reapable(&mut evicted);
        }
        let mut matched = Vec::new();
        for entry in &mut self.slots {
            if !entry.retired && pred(entry.key, &entry.value) {
                entry.retired = true;
                self.has_deferred = true;
                on_remove(entry.key, &entry.value);
                matched.push(entry.key);
            }
        }
        if self.has_deferred {
            self.collect_reapable(&mut evicted);
        }
        (matched, evicted)
    }

    /// Collects and returns every retired entry whose generation is no longer active on the call
    /// stack, leaving live (including same-key re-inserted) entries untouched.
    #[must_use]
    pub fn reap_deferred(&mut self) -> Vec<V> {
        let mut evicted = Vec::new();
        if self.has_deferred {
            self.collect_reapable(&mut evicted);
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::ReentrantSlotSlab;

    struct DropCounter(Arc<AtomicUsize>);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn active_slot_drop_is_deferred_across_nested_reentrancy_until_unwind_and_reap() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut slab: ReentrantSlotSlab<u64, DropCounter> = ReentrantSlotSlab::new();
        drop(slab.insert(1, DropCounter(Arc::clone(&drops))));
        drop(slab.insert(2, DropCounter(Arc::clone(&drops))));

        slab.enter(1);
        slab.enter(1);
        let (removed, evicted) = slab.remove_where(|_, _| true, |_, _| {});
        drop(evicted);
        assert_eq!(removed, vec![1, 2]);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(slab.len(), 1);

        assert!(!slab.leave(1, true));
        drop(slab.reap_deferred());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(slab.len(), 1);

        assert!(slab.leave(1, true));
        drop(slab.reap_deferred());
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(slab.is_empty());
    }

    #[test]
    fn same_key_reinsert_during_active_callback_reaps_only_retired_generation() {
        let old_drops = Arc::new(AtomicUsize::new(0));
        let new_drops = Arc::new(AtomicUsize::new(0));
        let mut slab: ReentrantSlotSlab<u64, DropCounter> = ReentrantSlotSlab::new();
        drop(slab.insert(7, DropCounter(Arc::clone(&old_drops))));
        slab.enter(7);
        // Re-insert the same key `7` while the first generation is active on the stack.
        drop(slab.insert(7, DropCounter(Arc::clone(&new_drops))));
        assert_eq!(old_drops.load(Ordering::SeqCst), 0);
        assert_eq!(new_drops.load(Ordering::SeqCst), 0);
        assert_eq!(slab.len(), 2);

        // Unwinding the first generation must reap ONLY the retired first entry; live `7` stays!
        assert!(slab.leave(7, true));
        drop(slab.reap_deferred());
        assert_eq!(old_drops.load(Ordering::SeqCst), 1);
        assert_eq!(new_drops.load(Ordering::SeqCst), 0);
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.iter().count(), 1);
    }

    #[test]
    fn evicted_values_drop_outside_refcell_borrow_preventing_reentrant_borrow_mut_panic() {
        struct ReentrantDrop {
            slab: Rc<RefCell<ReentrantSlotSlab<u64, ReentrantDrop>>>,
            probe_ran: Arc<AtomicUsize>,
        }
        impl Drop for ReentrantDrop {
            fn drop(&mut self) {
                // Re-entrantly borrow the same RefCell during Drop: must NOT panic!
                let _ = self.slab.borrow_mut().len();
                self.probe_ran.fetch_add(1, Ordering::SeqCst);
            }
        }
        let slab = Rc::new(RefCell::new(ReentrantSlotSlab::new()));
        let ran = Arc::new(AtomicUsize::new(0));
        let ev1 = slab.borrow_mut().insert(
            1,
            ReentrantDrop {
                slab: Rc::clone(&slab),
                probe_ran: Arc::clone(&ran),
            },
        );
        drop(ev1);
        let ev2 = slab.borrow_mut().remove_or_defer(1);
        // RefCell borrow is now released before dropping `ev2`!
        drop(ev2);
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }
}
