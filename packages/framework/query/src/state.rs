//! Pure in-memory `CacheState` backing [`crate::QueryClient`]: hot/cold dense slabs, intrusive
//! hash-collision chain, `TagIndex` ownership, active/dirty slot queues, and bounded GC.

use std::collections::HashMap;

use bytes::Bytes;
use nve_rpc::{ProcId, Tag64};

use core::task::Waker;

use crate::slab::{
    QueryMeta, QueryPayload, QuerySlotId, TagIndex, FLAG_HYDRATED, FLAG_IN_DIRTY_QUEUE,
    FLAG_IS_FETCHING, FLAG_IS_INVALIDATED, FLAG_REFETCH_ON_FOCUS, FLAG_REFETCH_ON_RECONNECT,
    FLAG_SUBSCRIBED_ALL, NONE_IDX,
};
use crate::types::{IdentityBuildHasher, QueryHandle, QueryOptions};

#[cfg(not(target_arch = "wasm32"))]
pub type WakeCallback = std::sync::Arc<dyn Fn() + Send + Sync>;

#[cfg(target_arch = "wasm32")]
pub type WakeCallback = std::rc::Rc<dyn Fn()>;

pub(crate) struct CacheState {
    pub metas: Vec<QueryMeta>,
    pub payloads: Vec<QueryPayload>,
    pub free_slots: Vec<QuerySlotId>,
    pub key_index: HashMap<u64, u32, IdentityBuildHasher>,
    pub tags: TagIndex,
    pub active_slots: Vec<QuerySlotId>,
    pub dirty_slots: Vec<QuerySlotId>,
    pub wildcard_epoch: u32,
    pub next_mutation_id: u64,
    pub gc_cursor: usize,
    pub live_count: usize,
    pub subscribed_all_count: usize,
    pub wake_cb: Option<WakeCallback>,
    pub wake_cb_id: u64,
}

impl CacheState {
    pub(crate) fn new() -> Self {
        Self {
            metas: Vec::new(),
            payloads: Vec::new(),
            free_slots: Vec::new(),
            key_index: HashMap::with_hasher(IdentityBuildHasher),
            tags: TagIndex::default(),
            active_slots: Vec::new(),
            dirty_slots: Vec::new(),
            wildcard_epoch: 0,
            next_mutation_id: 1,
            gc_cursor: 0,
            live_count: 0,
            subscribed_all_count: 0,
            wake_cb: None,
            wake_cb_id: 0,
        }
    }

    #[inline]
    pub(crate) fn is_live(&self, handle: QueryHandle) -> bool {
        let idx = handle.slot.index();
        self.metas
            .get(idx)
            .is_some_and(|m| !m.is_free() && m.slot_gen == handle.generation)
    }

    pub(crate) fn lookup_slot(
        &self,
        proc_id: ProcId,
        key_hash: u64,
        encoded_args: &[u8],
    ) -> Option<QuerySlotId> {
        let mut cur = self.key_index.get(&key_hash).copied().unwrap_or(NONE_IDX);
        while cur != NONE_IDX {
            let meta = &self.metas[cur as usize];
            if meta.proc_id == proc_id
                && meta.args_hash == key_hash
                && self.payloads[cur as usize].encoded_args.as_ref() == encoded_args
            {
                return Some(QuerySlotId(cur));
            }
            cur = meta.hash_next;
        }
        None
    }

    pub(crate) fn mark_dirty(&mut self, slot: QuerySlotId) -> Option<WakeCallback> {
        let meta = &mut self.metas[slot.index()];
        meta.data_version = meta.data_version.wrapping_add(1).max(1);
        if (meta.flags & FLAG_IN_DIRTY_QUEUE) == 0 {
            meta.flags |= FLAG_IN_DIRTY_QUEUE;
            self.dirty_slots.push(slot);
        }
        self.wake_cb.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_slot(
        &mut self,
        proc_id: ProcId,
        proc_name: &'static str,
        key_hash: u64,
        encoded_args: Bytes,
        options: QueryOptions,
        tags: &[Tag64],
        overflow_all: bool,
        now_ms: u64,
    ) -> QueryHandle {
        let (slot, slot_gen) = if let Some(reused) = self.free_slots.pop() {
            let next_gen = self.metas[reused.index()].slot_gen.wrapping_add(1).max(1);
            (reused, next_gen)
        } else {
            let idx = u32::try_from(self.metas.len()).expect("query cache exceeded u32::MAX slots");
            self.metas.push(QueryMeta::free(1));
            self.payloads.push(QueryPayload::empty());
            (QuerySlotId(idx), 1)
        };

        let old_head = self.key_index.get(&key_hash).copied().unwrap_or(NONE_IDX);
        // Preserve `FLAG_IN_DIRTY_QUEUE` across evict+reuse so `dirty_slots.len() <= metas.len()`.
        let mut flags = self.metas[slot.index()].flags & FLAG_IN_DIRTY_QUEUE;
        if options.refetch_on_focus {
            flags |= FLAG_REFETCH_ON_FOCUS;
        }
        if options.refetch_on_reconnect {
            flags |= FLAG_REFETCH_ON_RECONNECT;
        }
        if overflow_all {
            flags |= FLAG_SUBSCRIBED_ALL;
            self.subscribed_all_count += 1;
        }

        let mut first_tag_edge = NONE_IDX;
        if !overflow_all && !tags.is_empty() {
            self.tags.attach_tags(slot, &mut first_tag_edge, tags);
        }

        self.metas[slot.index()] = QueryMeta {
            proc_id,
            args_hash: key_hash,
            updated_at_ms: now_ms,
            invalidated_epoch: self.wildcard_epoch,
            stale_time_ms: options.stale_time_ms,
            gc_time_ms: options.gc_time_ms,
            fetch_gen: 1,
            data_version: 1,
            slot_gen,
            hash_next: old_head,
            first_tag_edge,
            subscriber_count: 0,
            flags,
            optimistic_rev: 0,
        };
        let payload = &mut self.payloads[slot.index()];
        let _ = payload.reset();
        payload.proc_name = proc_name;
        payload.encoded_args = encoded_args;
        payload.last_inactive_ms = now_ms;
        payload.options = options;

        self.key_index.insert(key_hash, slot.raw());
        self.live_count += 1;

        QueryHandle {
            slot,
            generation: slot_gen,
        }
    }

    pub(crate) fn evict_slot(&mut self, slot: QuerySlotId, waiters_out: &mut Vec<Waker>) {
        let idx = slot.index();
        if self.metas[idx].is_free() || self.metas[idx].subscriber_count > 0 {
            return;
        }
        if (self.metas[idx].flags & FLAG_SUBSCRIBED_ALL) != 0 {
            self.subscribed_all_count = self.subscribed_all_count.saturating_sub(1);
        }

        let key_hash = self.metas[idx].args_hash;
        let next_in_chain = self.metas[idx].hash_next;

        if let Some(&head) = self.key_index.get(&key_hash) {
            if head == slot.raw() {
                if next_in_chain == NONE_IDX {
                    self.key_index.remove(&key_hash);
                } else {
                    self.key_index.insert(key_hash, next_in_chain);
                }
            } else {
                let mut cur = head;
                while cur != NONE_IDX {
                    let cur_next = self.metas[cur as usize].hash_next;
                    if cur_next == slot.raw() {
                        self.metas[cur as usize].hash_next = next_in_chain;
                        break;
                    }
                    cur = cur_next;
                }
            }
        }

        let mut first_edge = self.metas[idx].first_tag_edge;
        self.tags.unlink_slot(&mut first_edge);
        waiters_out.extend(self.payloads[idx].reset());

        let dirty_bit = self.metas[idx].flags & FLAG_IN_DIRTY_QUEUE;
        let next_gen = self.metas[idx].slot_gen.wrapping_add(1).max(1);
        let mut free_meta = QueryMeta::free(next_gen);
        free_meta.flags |= dirty_bit;
        self.metas[idx] = free_meta;
        self.free_slots.push(slot);
        self.live_count = self.live_count.saturating_sub(1);
    }

    pub(crate) fn invalidate_slot(
        &mut self,
        slot: QuerySlotId,
        active_out: &mut Vec<QueryHandle>,
        waiters_out: &mut Vec<Waker>,
    ) {
        let idx = slot.index();
        let meta = &mut self.metas[idx];
        if meta.is_free() {
            return;
        }
        // Bump `fetch_gen` BEFORE refetch so slow pre-mutation replies are discarded cleanly.
        meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
        meta.flags |= FLAG_IS_INVALIDATED;
        meta.flags &= !(FLAG_IS_FETCHING | FLAG_HYDRATED);
        let generation = meta.slot_gen;
        let is_active = meta.subscriber_count > 0;
        waiters_out.append(&mut self.payloads[idx].in_flight_waiters);
        let _ = self.mark_dirty(slot);
        if is_active {
            let handle = QueryHandle { slot, generation };
            if !active_out.contains(&handle) {
                active_out.push(handle);
            }
        }
    }

    pub(crate) fn gc_sweep(
        &mut self,
        now_ms: u64,
        max_steps: usize,
        max_entries: usize,
        waiters_out: &mut Vec<Waker>,
    ) -> usize {
        if self.metas.is_empty() {
            return 0;
        }
        let mut evicted = 0usize;
        let steps = max_steps.min(self.metas.len());
        for _ in 0..steps {
            if self.gc_cursor >= self.metas.len() {
                self.gc_cursor = 0;
            }
            let idx = self.gc_cursor;
            self.gc_cursor += 1;

            let meta = self.metas[idx];
            if meta.is_free() || meta.subscriber_count > 0 {
                continue;
            }
            let inactive_for = now_ms.saturating_sub(self.payloads[idx].last_inactive_ms);
            if inactive_for >= u64::from(meta.gc_time_ms) {
                self.evict_slot(QuerySlotId(idx as u32), waiters_out);
                evicted += 1;
            }
        }

        while self.live_count > max_entries {
            let mut oldest: Option<(QuerySlotId, u64)> = None;
            for (idx, meta) in self.metas.iter().enumerate() {
                if meta.is_free() || meta.subscriber_count > 0 {
                    continue;
                }
                let ts = self.payloads[idx].last_inactive_ms;
                if oldest.is_none_or(|(_, best_ts)| ts < best_ts) {
                    oldest = Some((QuerySlotId(idx as u32), ts));
                }
            }
            match oldest {
                Some((slot, _)) => {
                    self.evict_slot(slot, waiters_out);
                    evicted += 1;
                }
                None => break,
            }
        }
        evicted
    }
}
