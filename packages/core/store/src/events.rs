//! Per-surface refcounted event listener tracker (`[u32; 32]` counts per tracked table).
//!
//! Tracks `EventSet<E>` listener masks across committed `ChangeSet`s with zero hash maps and zero
//! steady-state heap allocations. Backends call [`SurfaceEventTracker::sync`] after each commit to
//! observe exact `0 -> 1` (subscribe) and `1 -> 0` (unsubscribe) transitions per `(TableSlot, EventId)`
//! for lazy platform listener delegation (e.g. DOM root listeners on Web).

use nve_abi::table::{MAX_TABLE_SLOTS, NO_VAL_WORD};
use nve_abi::{EventEnum, EventId, EventSet};

use crate::changes::ChangeSet;
use crate::id::NodeId;
use crate::store::NodeStore;
use crate::table::{HasEvents, TableSlot};

#[derive(Debug, Clone)]
struct TrackedTable {
    slot: TableSlot,
    events_word: u8,
    counts: [u32; 32],
    /// Dense node-index-indexed shadow of each live node's last observed `events_word` mask.
    masks: Vec<u32>,
}

impl TrackedTable {
    fn new(slot: TableSlot, events_word: u8) -> Self {
        Self {
            slot,
            events_word,
            counts: [0; 32],
            masks: Vec::new(),
        }
    }
}

/// Refcounted listener tracker over every `#[nve_table(events = E)]` table in a surface's
/// [`NodeStore`].
#[derive(Debug, Clone)]
pub struct SurfaceEventTracker {
    /// Maps `TableSlot.0` -> index in `tables` (`u8::MAX` = untracked).
    slot_to_idx: [u8; MAX_TABLE_SLOTS],
    tables: Vec<TrackedTable>,
}

impl Default for SurfaceEventTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceEventTracker {
    pub const fn new() -> Self {
        Self {
            slot_to_idx: [u8::MAX; MAX_TABLE_SLOTS],
            tables: Vec::new(),
        }
    }

    /// Pre-registers a compiled `#[nve_table(events = E)]` table so the first frame's `sync` does
    /// not allocate a table entry.
    pub fn track<T: HasEvents>(&mut self) {
        self.track_slot(T::slot(), T::DESC.events_word);
    }

    /// Pre-registers `slot` with its `events_word` (`0..32`).
    pub fn track_slot(&mut self, slot: TableSlot, events_word: u8) {
        let _ = self.ensure_tracked(slot, events_word);
    }

    /// Pre-sizes dense per-node shadow masks across all currently tracked tables so steady-state
    /// [`Self::sync`] performs zero heap allocations up to `node_capacity` nodes.
    pub fn reserve_nodes(&mut self, node_capacity: usize) {
        for t in &mut self.tables {
            if t.masks.len() < node_capacity {
                t.masks.resize(node_capacity, 0);
            }
        }
    }

    #[inline]
    fn ensure_tracked(&mut self, slot: TableSlot, events_word: u8) -> Option<usize> {
        let slot_idx = slot.0 as usize;
        if slot_idx >= MAX_TABLE_SLOTS || events_word >= 32 {
            return None;
        }
        let existing = self.slot_to_idx[slot_idx];
        if existing != u8::MAX {
            return Some(existing as usize);
        }
        let idx = self.tables.len();
        debug_assert!(idx < MAX_TABLE_SLOTS);
        self.tables.push(TrackedTable::new(slot, events_word));
        self.slot_to_idx[slot_idx] = idx as u8;
        Some(idx)
    }

    #[inline]
    fn table_ref(&self, slot: TableSlot) -> Option<&TrackedTable> {
        let idx = *self.slot_to_idx.get(slot.0 as usize)?;
        if idx == u8::MAX {
            None
        } else {
            self.tables.get(idx as usize)
        }
    }

    #[inline]
    fn apply_mask_delta(
        t: &mut TrackedTable,
        node_index: usize,
        new_mask: u32,
        on_transition: &mut impl FnMut(TableSlot, EventId, bool),
    ) {
        let old_mask = t.masks.get(node_index).copied().unwrap_or(0);
        if old_mask == new_mask {
            return;
        }
        if new_mask != 0 && node_index >= t.masks.len() {
            t.masks.resize(node_index + 1, 0);
        }
        if node_index < t.masks.len() {
            t.masks[node_index] = new_mask;
        }
        let mut added = new_mask & !old_mask;
        while added != 0 {
            let bit = added.trailing_zeros() as usize;
            added &= added - 1;
            let prev = t.counts[bit];
            t.counts[bit] = prev + 1;
            if prev == 0 {
                on_transition(t.slot, EventId(bit as u32), true);
            }
        }
        let mut removed = old_mask & !new_mask;
        while removed != 0 {
            let bit = removed.trailing_zeros() as usize;
            removed &= removed - 1;
            let next = t.counts[bit].saturating_sub(1);
            t.counts[bit] = next;
            if next == 0 {
                on_transition(t.slot, EventId(bit as u32), false);
            }
        }
    }

    /// Updates refcounts from one committed [`ChangeSet`] against `store`, invoking
    /// `on_transition(slot, event_id, true)` when an event transitions `0 -> 1` listeners and
    /// `on_transition(slot, event_id, false)` when it transitions `1 -> 0`.
    pub fn sync(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        mut on_transition: impl FnMut(TableSlot, EventId, bool),
    ) {
        if !changes.removed.is_empty() {
            for &id in &changes.removed {
                let idx = id.index();
                for t in &mut self.tables {
                    if idx < t.masks.len() && t.masks[idx] != 0 {
                        Self::apply_mask_delta(t, idx, 0, &mut on_transition);
                    }
                }
            }
        }

        for tc in changes.tables() {
            let ew = store.events_word(tc.slot);
            if ew == NO_VAL_WORD {
                continue;
            }
            let Some(t_idx) = self.ensure_tracked(tc.slot, ew) else {
                continue;
            };
            let t = &mut self.tables[t_idx];
            let ew = t.events_word;
            let ev_bit = 1u32 << ew;
            for &id in &tc.detached {
                let idx = id.index();
                if idx < t.masks.len() && t.masks[idx] != 0 {
                    Self::apply_mask_delta(t, idx, 0, &mut on_transition);
                }
            }
            for (id, dirty_mask) in tc.iter() {
                if (dirty_mask & ev_bit) == 0 {
                    continue;
                }
                let new_mask = store
                    .words(id, tc.slot)
                    .and_then(|w| w.get(ew as usize).copied())
                    .unwrap_or(0);
                Self::apply_mask_delta(t, id.index(), new_mask, &mut on_transition);
            }
        }
    }

    /// Reconciles tracker state against the entire live `store` (e.g. initial mount or snapshot
    /// restore), firing exact `0 -> 1` and `1 -> 0` transitions.
    pub fn sync_full(
        &mut self,
        store: &NodeStore,
        mut on_transition: impl FnMut(TableSlot, EventId, bool),
    ) {
        for slot in store.table_slots() {
            let ew = store.events_word(slot);
            if ew != NO_VAL_WORD {
                let _ = self.ensure_tracked(slot, ew);
            }
        }
        let cap = store.capacity();
        for t in &mut self.tables {
            let limit = cap.max(t.masks.len());
            for idx in 0..limit {
                let new_mask = store
                    .id_at(idx)
                    .map_or(0, |id| store.node_events_mask(t.slot, id));
                Self::apply_mask_delta(t, idx, new_mask, &mut on_transition);
            }
        }
    }

    /// Clears all active counts and per-node masks, invoking `on_transition(slot, event_id, false)`
    /// for every event that currently has at least one listener.
    pub fn reset_with(&mut self, mut on_transition: impl FnMut(TableSlot, EventId, bool)) {
        for t in &mut self.tables {
            for bit in 0..32 {
                if t.counts[bit] > 0 {
                    t.counts[bit] = 0;
                    on_transition(t.slot, EventId(bit as u32), false);
                }
            }
            t.masks.fill(0);
        }
    }

    /// Silently zeroes all active counts and per-node masks (retains allocated capacity).
    pub fn reset(&mut self) {
        for t in &mut self.tables {
            t.counts = [0; 32];
            t.masks.fill(0);
        }
    }

    /// Raw 32-bit listener mask tracked for `id` in `slot` (`0` if untracked or unset).
    #[inline]
    pub fn listeners(&self, slot: TableSlot, id: NodeId) -> u32 {
        self.table_ref(slot)
            .and_then(|t| t.masks.get(id.index()).copied())
            .unwrap_or(0)
    }

    /// Typed [`EventSet`] tracked for `id` in table `T`.
    #[inline]
    pub fn listeners_of<T: HasEvents>(&self, id: NodeId) -> EventSet<T::Event> {
        EventSet::from_raw(self.listeners(T::slot(), id))
    }

    /// Number of live nodes currently subscribed to `(slot, event)`.
    #[inline]
    pub fn active_count(&self, slot: TableSlot, event: EventId) -> u32 {
        let bit = event.raw() as usize;
        if bit >= 32 {
            return 0;
        }
        self.table_ref(slot).map_or(0, |t| t.counts[bit])
    }

    /// Number of live nodes currently subscribed to typed `event` on table `T`.
    #[inline]
    pub fn active_count_of<T: HasEvents>(&self, event: T::Event) -> u32 {
        self.active_count(T::slot(), event.event_id())
    }

    /// Whether at least one live node is subscribed to `(slot, event)`.
    #[inline]
    pub fn has_active(&self, slot: TableSlot, event: EventId) -> bool {
        self.active_count(slot, event) > 0
    }

    /// Whether at least one live node is subscribed to typed `event` on table `T`.
    #[inline]
    pub fn has_active_of<T: HasEvents>(&self, event: T::Event) -> bool {
        self.active_count_of::<T>(event) > 0
    }

    /// Bitmask of every event bit `0..32` with `active_count > 0` on `slot`.
    #[inline]
    pub fn active_mask(&self, slot: TableSlot) -> u32 {
        let Some(t) = self.table_ref(slot) else {
            return 0;
        };
        let mut mask = 0u32;
        for (bit, &count) in t.counts.iter().enumerate() {
            if count > 0 {
                mask |= 1u32 << bit;
            }
        }
        mask
    }

    /// Typed [`EventSet`] of every active event on table `T`.
    #[inline]
    pub fn active_set<T: HasEvents>(&self) -> EventSet<T::Event> {
        EventSet::from_raw(self.active_mask(T::slot()))
    }
}
