//! Re-entrant-safe keyed slot slab (`ReentrantSlotSlab<K, V>`) preventing use-after-free when a
//! callback (e.g., a `wasm-bindgen` `Closure`) unmounts or cancels its own slot while executing.
//!
//! Shared across `nve-clipboard-web`, `nve-system-env-web`, and `nve-backend-web` with zero
//! platform dependencies so every JS/native callback registry shares one audited implementation.

/// Re-entrant-safe keyed container deferring `Drop` of any slot whose key is currently executing
/// on the call stack until after the active callback unwinds.
pub struct ReentrantSlotSlab<K, V> {
    slots: Vec<(K, V)>,
    active_stack: Vec<K>,
    deferred: Vec<K>,
}

impl<K, V> Default for ReentrantSlotSlab<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> ReentrantSlotSlab<K, V> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            active_stack: Vec::new(),
            deferred: Vec::new(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &(K, V)> {
        self.slots.iter()
    }
}

impl<K: Copy + Eq, V> ReentrantSlotSlab<K, V> {
    /// Returns `true` if `key` is currently active anywhere on the re-entrancy stack.
    #[inline]
    pub fn is_active(&self, key: K) -> bool {
        self.active_stack.contains(&key)
    }

    /// Marks `key` as actively executing on the call stack (supports nested re-entrancy).
    #[inline]
    pub fn enter(&mut self, key: K) {
        self.active_stack.push(key);
    }

    /// Pops one execution frame for `key` from the active stack. When `mark_settled` is `true`,
    /// `key` is queued for deferred removal so one-shot callbacks drop cleanly on the next reap.
    ///
    /// Returns `true` when the active stack is now empty AND deferred drops are waiting to be
    /// reaped (signaling the caller to schedule an asynchronous deferred reap).
    pub fn leave(&mut self, key: K, mark_settled: bool) -> bool {
        if let Some(pos) = self.active_stack.iter().rposition(|&k| k == key) {
            self.active_stack.remove(pos);
        }
        if mark_settled && !self.deferred.contains(&key) {
            self.deferred.push(key);
        }
        self.active_stack.is_empty() && !self.deferred.is_empty()
    }

    /// Inserts `(key, value)`, replacing any inactive existing entry with the same key.
    pub fn insert(&mut self, key: K, value: V) {
        self.reap_deferred();
        if !self.is_active(key) {
            self.slots.retain(|(k, _)| *k != key);
        }
        self.slots.push((key, value));
    }

    /// Removes `key` immediately if inactive, or defers dropping its value until after the active
    /// callback unwinds if `key` is currently executing.
    pub fn remove_or_defer(&mut self, key: K) {
        if self.is_active(key) {
            if !self.deferred.contains(&key) {
                self.deferred.push(key);
            }
            return;
        }
        self.slots.retain(|(k, _)| *k != key);
        self.reap_deferred();
    }

    /// Collects keys matching `pred`, calls `on_remove` for each, and removes every matching slot
    /// (deferring any slot whose key is currently active on the call stack).
    pub fn remove_where(
        &mut self,
        mut pred: impl FnMut(K, &V) -> bool,
        mut on_remove: impl FnMut(K),
    ) -> Vec<K> {
        self.reap_deferred();
        let matched: Vec<K> = self
            .slots
            .iter()
            .filter_map(|(k, v)| pred(*k, v).then_some(*k))
            .collect();
        for &k in &matched {
            on_remove(k);
        }
        let mut newly_deferred = Vec::new();
        let active = &self.active_stack;
        self.slots.retain(|(k, _)| {
            if !matched.contains(k) {
                return true;
            }
            if active.contains(k) {
                newly_deferred.push(*k);
                true
            } else {
                false
            }
        });
        for k in newly_deferred {
            if !self.deferred.contains(&k) {
                self.deferred.push(k);
            }
        }
        matched
    }

    /// Drops every deferred slot that is no longer active on the call stack.
    pub fn reap_deferred(&mut self) {
        if self.deferred.is_empty() {
            return;
        }
        let deferred = std::mem::take(&mut self.deferred);
        let mut still_active = Vec::new();
        let active = &self.active_stack;
        self.slots.retain(|(k, _)| {
            if !deferred.contains(k) {
                return true;
            }
            if active.contains(k) {
                still_active.push(*k);
                true
            } else {
                false
            }
        });
        self.deferred = still_active;
    }
}

#[cfg(test)]
mod tests {
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
        slab.insert(1, DropCounter(Arc::clone(&drops)));
        slab.insert(2, DropCounter(Arc::clone(&drops)));

        // Enter key 1 twice (nested re-entrancy) and cancel both keys inside the callback.
        slab.enter(1);
        slab.enter(1);
        let removed = slab.remove_where(|_, _| true, |_| {});
        assert_eq!(removed, vec![1, 2]);
        // Key 2 dropped immediately; key 1 MUST stay alive while active!
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(slab.len(), 1);

        // First leave still leaves key 1 active (depth 1) -> must NOT request reap yet.
        assert!(!slab.leave(1, true));
        slab.reap_deferred();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(slab.len(), 1);

        // Final leave unwinds completely -> requests deferred reap; reap drops key 1 cleanly.
        assert!(slab.leave(1, true));
        slab.reap_deferred();
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(slab.is_empty());
    }
}
