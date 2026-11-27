//! O(k) tag invalidation, O(1) wildcard invalidation, focus/reconnect refetch queries,
//! bounded GC sweep, and cache diagnostics on [`QueryClient`].

use nve_rpc::Tag64;

use crate::engine::{run_side_effects, QueryClient};
use crate::slab::{
    QuerySlotId, FLAG_REFETCH_ON_FOCUS, FLAG_REFETCH_ON_RECONNECT, FLAG_SUBSCRIBED_ALL,
};
use crate::types::QueryHandle;

impl<T, C> QueryClient<T, C> {
    /// Invalidates all queries tagged with any tag in `tags` (O(k) via flat `TagEdge` pool) into
    /// caller-reused scratch buffer `active_out` (zero heap allocations when `active_out` is
    /// pre-sized and each tag links `<= 32` slots; larger fan-outs spill cleanly to a temporary
    /// heap buffer).
    pub fn invalidate_tags_into(&self, tags: &[Tag64], active_out: &mut Vec<QueryHandle>) {
        active_out.clear();
        let (wake, waiters) = self.inner.state.with(|s| {
            let mut visited_count = 0usize;
            let mut waiters = Vec::new();
            for &tag in tags {
                let mut cur_slots = [QuerySlotId(0); 32];
                let mut len = 0usize;
                let mut overflow: Option<Vec<QuerySlotId>> = None;
                s.tags.for_each_slot(tag, |slot| {
                    if let Some(vec) = &mut overflow {
                        vec.push(slot);
                    } else if len < cur_slots.len() {
                        cur_slots[len] = slot;
                        len += 1;
                    } else {
                        let mut vec = Vec::with_capacity(64);
                        vec.extend_from_slice(&cur_slots);
                        vec.push(slot);
                        overflow = Some(vec);
                    }
                });
                if let Some(vec) = overflow {
                    for slot in vec {
                        s.invalidate_slot(slot, active_out, &mut waiters);
                        visited_count += 1;
                    }
                } else {
                    for &slot in &cur_slots[..len] {
                        s.invalidate_slot(slot, active_out, &mut waiters);
                        visited_count += 1;
                    }
                }
            }
            if !tags.is_empty() && s.subscribed_all_count > 0 {
                for idx in 0..s.metas.len() {
                    if !s.metas[idx].is_free()
                        && (s.metas[idx].flags & FLAG_SUBSCRIBED_ALL) != 0
                    {
                        s.invalidate_slot(QuerySlotId(idx as u32), active_out, &mut waiters);
                        visited_count += 1;
                    }
                }
            }
            let wake = if visited_count > 0 {
                s.wake_cb.clone()
            } else {
                None
            };
            (wake, waiters)
        });
        run_side_effects(wake, waiters);
    }

    /// Invalidates all queries tagged with any tag in `tags`, returning active mounted handles
    /// that should refetch.
    pub fn invalidate_tags(&self, tags: &[Tag64]) -> Vec<QueryHandle> {
        let mut active = Vec::new();
        self.invalidate_tags_into(tags, &mut active);
        active
    }

    /// O(active) wildcard `*` invalidation (`wildcard_epoch += 1`), marking active mounted queries
    /// dirty and returning their handles for refetch. Any in-flight fetches started on earlier
    /// epochs are rejected automatically on completion even on inactive slots, and every
    /// (also inactive) `StreamPending` slot is released so no waiter blocks on a stream chunk the
    /// client will now refuse.
    pub fn invalidate_all(&self) -> Vec<QueryHandle> {
        let (active, wake, waiters) = self.inner.state.with(|s| {
            s.wildcard_epoch = s.wildcard_epoch.wrapping_add(1);
            let mut out = Vec::with_capacity(s.active_slots.len());
            let mut waiters = Vec::new();
            let active_len = s.active_slots.len();
            for i in 0..active_len {
                let slot = s.active_slots[i];
                s.invalidate_slot(slot, &mut out, &mut waiters);
            }
            let _ = s.release_stream_pending(&mut waiters);
            (out, s.wake_cb.clone(), waiters)
        });
        run_side_effects(wake, waiters);
        active
    }

    /// Marks explicit live `handles` invalidated and dirty (bumping `fetch_gen` and waking the
    /// registered UI observer once), e.g. for handles returned by [`Self::on_window_focus`] or
    /// [`Self::on_network_online`].
    pub fn invalidate_handles(&self, handles: &[QueryHandle]) {
        if handles.is_empty() {
            return;
        }
        let (wake, waiters) = self.inner.state.with(|s| {
            let mut active_scratch = Vec::new();
            let mut waiters = Vec::new();
            let mut touched = false;
            for &handle in handles {
                if s.is_live(handle) {
                    s.invalidate_slot(handle.slot, &mut active_scratch, &mut waiters);
                    touched = true;
                }
            }
            let wake = if touched { s.wake_cb.clone() } else { None };
            (wake, waiters)
        });
        run_side_effects(wake, waiters);
    }

    /// Returns active mounted stale/errored handles eligible for refetch on window focus.
    pub fn on_window_focus(&self) -> Vec<QueryHandle> {
        self.collect_refetch_eligible(FLAG_REFETCH_ON_FOCUS)
    }

    /// Returns active mounted stale/errored handles eligible for refetch on network reconnect.
    pub fn on_network_online(&self) -> Vec<QueryHandle> {
        self.collect_refetch_eligible(FLAG_REFETCH_ON_RECONNECT)
    }

    fn collect_refetch_eligible(&self, required_flag: u16) -> Vec<QueryHandle> {
        let now_ms = self.inner.clock.now_ms();
        let grace = self.inner.config.hydration_grace_ms;
        self.inner.state.with(|s| {
            let mut out = Vec::new();
            for &slot in &s.active_slots {
                let meta = s.metas[slot.index()];
                if (meta.flags & required_flag) != 0
                    && !meta.is_fetching()
                    && (meta.has_error() || meta.is_stale(now_ms, s.wildcard_epoch, grace))
                {
                    out.push(QueryHandle {
                        slot,
                        generation: meta.slot_gen,
                    });
                }
            }
            out
        })
    }

    /// Runs a bounded incremental GC sweep over up to `max_steps` slots, returning how many
    /// expired inactive slots were evicted (waking any waiting callers strictly outside the lock).
    pub fn gc_sweep(&self, max_steps: usize) -> usize {
        let now_ms = self.inner.clock.now_ms();
        let (evicted, waiters) = self.inner.state.with(|s| {
            let mut waiters = Vec::new();
            let evicted = s.gc_sweep(
                now_ms,
                max_steps,
                self.inner.config.max_entries,
                &mut waiters,
            );
            (evicted, waiters)
        });
        run_side_effects(None, waiters);
        evicted
    }

    #[inline]
    pub fn live_slot_count(&self) -> usize {
        self.inner.state.with(|s| s.live_count)
    }

    #[inline]
    pub fn active_slot_count(&self) -> usize {
        self.inner.state.with(|s| s.active_slots.len())
    }

    #[inline]
    pub fn tag_edge_count(&self) -> usize {
        self.inner.state.with(|s| s.tags.live_edge_count())
    }
}
