//! Optimistic patch transactions, action execution with automatic invalidation, O(k) tag & O(1)
//! wildcard invalidation, focus/reconnect hooks, bounded GC, and `NVEQ` dehydration/hydration.

use core::future::{poll_fn, Future};
use core::task::Poll;

use nve_rpc::{
    call_with_codec, decode_reply, Action, CallReply, MaybeSend, MaybeSync, Procedure, Query,
    RpcCodec, RpcError, Tag64, TagSink, Transport,
};

use crate::engine::{query_key_hash, run_side_effects, QueryClient};
use crate::optimistic::{
    apply_layer_to_slot, mark_mutation_settling, prepare_recompute_job, remove_mutation_layers,
    OptimisticHandle, OptimisticTx, PendingPatchOp,
};
use crate::slab::{QuerySlotId, FLAG_SUBSCRIBED_ALL};
use crate::state::WakeCallback;
use crate::types::{FetchTicket, MutationId, QueryHandle, Shared};

struct FetchCancelGuard<'a, T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> {
    client: &'a QueryClient<T, C>,
    ticket: Option<FetchTicket>,
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> Drop for FetchCancelGuard<'_, T, C> {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.client.cancel_fetch_ticket(ticket);
        }
    }
}

struct SubCancelGuard<'a, T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> {
    client: &'a QueryClient<T, C>,
    handle: Option<QueryHandle>,
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> Drop for SubCancelGuard<'_, T, C> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.client.unsubscribe(handle);
        }
    }
}

struct OptimisticCancelGuard<'a, T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> {
    client: &'a QueryClient<T, C>,
    handle: Option<OptimisticHandle>,
    static_tags_on_cancel: Option<TagSink>,
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> Drop
    for OptimisticCancelGuard<'_, T, C>
{
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.client.rollback_optimistic(&handle);
        }
        if let Some(tags) = self.static_tags_on_cancel.take() {
            if tags.is_overflow_all() {
                let _ = self.client.invalidate_all();
            } else if !tags.is_empty() {
                let _ = self.client.invalidate_tags(tags.as_slice());
            }
        }
    }
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    /// Suspends until `handle` is no longer `IS_FETCHING` (or is evicted), enabling deduplicated
    /// concurrent `fetch_handle` / `prefetch` callers to share a single in-flight network request.
    async fn wait_for_in_flight(&self, handle: QueryHandle) {
        poll_fn(|cx| {
            self.inner.state.with(|s| {
                if !s.is_live(handle) {
                    return Poll::Ready(());
                }
                let idx = handle.slot.index();
                if !s.metas[idx].is_fetching() {
                    return Poll::Ready(());
                }
                let waiters = &mut s.payloads[idx].in_flight_waiters;
                if !waiters.iter().any(|w| w.will_wake(cx.waker())) {
                    waiters.push(cx.waker().clone());
                }
                Poll::Pending
            })
        })
        .await;
    }

    /// Executes one network fetch attempt for `handle` without holding the cache lock across
    /// `.await`. Cancellation-safe: dropping the future mid-`.await` clears `FLAG_IS_FETCHING`
    /// and wakes deduplicated waiters automatically so waiting callers can take over cleanly.
    pub async fn fetch_handle<P>(
        &self,
        handle: QueryHandle,
    ) -> Result<Option<Shared<P::Output>>, Shared<RpcError<P::Error>>>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Output> + RpcCodec<P::Error>,
    {
        const MAX_TAKEOVERS: usize = 8;
        let mut ticket_and_call = None;
        for _ in 0..MAX_TAKEOVERS {
            if let Some(pair) = self.begin_fetch_for::<P>(handle) {
                ticket_and_call = Some(pair);
                break;
            }
            let wait_snapshot = self.inner.state.with(|s| {
                if !s.is_live(handle) {
                    return None;
                }
                let idx = handle.slot.index();
                let meta = s.metas[idx];
                if meta.proc_id != P::ID || !meta.is_fetching() {
                    return None;
                }
                let inv_ver = ((u64::from(s.wildcard_epoch)) << 32) | u64::from(meta.fetch_gen);
                Some((s.payloads[idx].completed_seq, inv_ver))
            });
            let Some((pre_completed_seq, pre_inv_ver)) = wait_snapshot else {
                let snap = self.read::<P>(handle);
                if let Some(err) = snap.error {
                    return Err(err);
                }
                return Ok(snap.data);
            };
            self.wait_for_in_flight(handle).await;
            let settled_by_leader = self.inner.state.with(|s| {
                if !s.is_live(handle) {
                    return true;
                }
                let idx = handle.slot.index();
                let meta = s.metas[idx];
                if meta.proc_id != P::ID {
                    return true;
                }
                let cur_completed_seq = s.payloads[idx].completed_seq;
                if cur_completed_seq == pre_completed_seq {
                    return false;
                }
                let cur_inv_ver = ((u64::from(s.wildcard_epoch)) << 32) | u64::from(meta.fetch_gen);
                cur_inv_ver == pre_inv_ver
                    || (!meta.is_invalidated(s.wildcard_epoch) && meta.has_data())
            });
            if settled_by_leader {
                let snap = self.read::<P>(handle);
                if let Some(err) = snap.error {
                    return Err(err);
                }
                return Ok(snap.data);
            }
        }
        let Some((ticket, raw_call)) = ticket_and_call else {
            let snap = self.read::<P>(handle);
            if let Some(err) = snap.error {
                return Err(err);
            }
            return Ok(snap.data);
        };
        let mut cancel_guard = FetchCancelGuard {
            client: self,
            ticket: Some(ticket),
        };
        let raw_res = self.inner.transport.call_raw(raw_call).await;
        let decoded = match raw_res {
            Ok(raw_reply) => decode_reply::<P, C>(&self.inner.codec, raw_reply),
            Err(terr) => Err(RpcError::Transport(terr)),
        };
        cancel_guard.ticket = None;
        match decoded {
            Ok(reply) => {
                self.complete_fetch_shared::<P>(ticket, Ok(reply));
                Ok(self.read::<P>(handle).data)
            }
            Err(err) => {
                let shared_err = Shared::new(err);
                self.complete_fetch_shared::<P>(ticket, Err(Shared::clone(&shared_err)));
                Err(shared_err)
            }
        }
    }

    /// Executes a query with exponential backoff retries on retryable errors (`Network`,
    /// `Timeout`, `429`, `503`, `408`) using the caller-provided platform sleep function.
    pub async fn fetch_handle_with_retry<P, SleepFn, SleepFut>(
        &self,
        handle: QueryHandle,
        mut sleep_ms: SleepFn,
    ) -> Result<Option<Shared<P::Output>>, Shared<RpcError<P::Error>>>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Output> + RpcCodec<P::Error>,
        SleepFn: FnMut(u32) -> SleepFut,
        SleepFut: Future<Output = ()>,
    {
        let options = self.inner.state.with(|s| {
            if s.is_live(handle) {
                s.payloads[handle.slot.index()].options
            } else {
                self.inner.config.default_options
            }
        });

        let mut attempt = 0u8;
        loop {
            match self.fetch_handle::<P>(handle).await {
                Ok(val) => return Ok(val),
                Err(err) if err.is_retryable() && attempt < options.max_retries => {
                    let delay = options.retry_delay_ms(attempt);
                    attempt = attempt.saturating_add(1);
                    sleep_ms(delay).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Prefetches query `(P, args)` into the cache (usable in SSR loaders or hover prefetchers)
    /// without needing an active UI subscriber.
    pub async fn prefetch<P>(
        &self,
        args: &P::Args,
    ) -> Result<Shared<P::Output>, Shared<RpcError<P::Error>>>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
    {
        let handle = self
            .subscribe::<P>(args, self.inner.config.default_options)
            .map_err(|e| Shared::new(RpcError::Codec(e)))?;
        let mut sub_guard = SubCancelGuard {
            client: self,
            handle: Some(handle),
        };
        let res = self.fetch_handle::<P>(handle).await;
        if let Some(h) = sub_guard.handle.take() {
            self.unsubscribe(h);
        }
        match res {
            Ok(Some(val)) => Ok(val),
            Ok(None) => Err(Shared::new(RpcError::Transport(
                nve_rpc::TransportError::new("prefetched query completed without data"),
            ))),
            Err(err) => Err(err),
        }
    }
    /// Applies an [`OptimisticTx`] built outside the cache lock and returns an [`OptimisticHandle`]
    /// for settling or rolling back.
    pub fn apply_optimistic<F>(&self, build: F) -> OptimisticHandle
    where
        F: FnOnce(&mut OptimisticTx<'_, C>),
    {
        let mut tx = OptimisticTx::new(&self.inner.codec);
        build(&mut tx);
        if tx.ops.is_empty() {
            return OptimisticHandle {
                mutation_id: MutationId(0),
                touched: Vec::new(),
            };
        }

        let (handle, jobs, wake) = self.inner.state.with(|s| {
            let mutation_id = MutationId(s.next_mutation_id);
            s.next_mutation_id = s.next_mutation_id.wrapping_add(1).max(1);

            let mut touched = Vec::new();
            let mut wake_cb: Option<WakeCallback> = None;

            for op in tx.ops {
                match op {
                    PendingPatchOp::ExactKey {
                        proc_id,
                        encoded_args,
                        make_layer,
                    } => {
                        let key_hash = query_key_hash(proc_id, &encoded_args);
                        if let Some(slot) = s.lookup_slot(proc_id, key_hash, &encoded_args) {
                            let idx = slot.index();
                            if s.metas[idx].has_data() {
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                let h = QueryHandle {
                                    slot,
                                    generation: s.metas[idx].slot_gen,
                                };
                                if !touched.contains(&h) {
                                    touched.push(h);
                                }
                            }
                        }
                    }
                    PendingPatchOp::ByTag {
                        proc_id,
                        tag,
                        make_layer,
                    } => {
                        let mut slots = Vec::new();
                        s.tags.for_each_slot(tag, |slot| slots.push(slot));
                        for slot in slots {
                            let idx = slot.index();
                            if !s.metas[idx].is_free()
                                && s.metas[idx].proc_id == proc_id
                                && s.metas[idx].has_data()
                            {
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                let h = QueryHandle {
                                    slot,
                                    generation: s.metas[idx].slot_gen,
                                };
                                if !touched.contains(&h) {
                                    touched.push(h);
                                }
                            }
                        }
                    }
                    PendingPatchOp::AllOfProc {
                        proc_id,
                        make_layer,
                    } => {
                        for idx in 0..s.metas.len() {
                            if !s.metas[idx].is_free()
                                && s.metas[idx].proc_id == proc_id
                                && s.metas[idx].has_data()
                            {
                                let slot = QuerySlotId(idx as u32);
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                let h = QueryHandle {
                                    slot,
                                    generation: s.metas[idx].slot_gen,
                                };
                                if !touched.contains(&h) {
                                    touched.push(h);
                                }
                            }
                        }
                    }
                }
            }

            let mut jobs = Vec::with_capacity(touched.len());
            for &qh in &touched {
                let idx = qh.slot.index();
                if let Some(job) =
                    prepare_recompute_job(qh.slot, &mut s.metas[idx], &mut s.payloads[idx])
                {
                    jobs.push(job);
                } else if let Some(cb) = s.mark_dirty(qh.slot) {
                    wake_cb = Some(cb);
                }
            }

            (
                OptimisticHandle {
                    mutation_id,
                    touched,
                },
                jobs,
                wake_cb,
            )
        });

        let recompute_wake = self.recompute_jobs_outside_lock(jobs);
        if let Some(cb) = recompute_wake.or(wake) {
            cb();
        }
        handle
    }

    /// Rolls back all optimistic layers belonging to `handle` without disturbing sibling mutations,
    /// recomputing remaining layers strictly outside the cache lock.
    pub fn rollback_optimistic(&self, handle: &OptimisticHandle) {
        let (jobs, wake) = self.inner.state.with(|s| {
            let mut wake_cb: Option<WakeCallback> = None;
            let mut jobs = Vec::new();
            for &qh in &handle.touched {
                if !s.is_live(qh) {
                    continue;
                }
                let idx = qh.slot.index();
                if remove_mutation_layers(&mut s.payloads[idx], handle.mutation_id) {
                    if let Some(job) =
                        prepare_recompute_job(qh.slot, &mut s.metas[idx], &mut s.payloads[idx])
                    {
                        jobs.push(job);
                    } else if let Some(cb) = s.mark_dirty(qh.slot) {
                        wake_cb = Some(cb);
                    }
                }
            }
            (jobs, wake_cb)
        });
        let recompute_wake = self.recompute_jobs_outside_lock(jobs);
        if let Some(cb) = recompute_wake.or(wake) {
            cb();
        }
    }

    /// Marks all slots touched by `handle` as invalidated and arms their optimistic layers to
    /// retire automatically once the post-action refetch commits (zero UI flicker).
    pub fn settle_optimistic(&self, handle: &OptimisticHandle) -> Vec<QueryHandle> {
        self.settle_action_atomically(handle, &[], false)
    }

    /// Single-lock atomic post-action settlement: applies wildcard/tag invalidations AND arms
    /// optimistic layers to settle on the exact post-invalidation `fetch_gen` with ONE
    /// `invalidate_slot` per slot and ONE `wake_cb()` call after releasing the lock.
    fn settle_action_atomically(
        &self,
        opt_handle: &OptimisticHandle,
        tags: &[Tag64],
        invalidate_all: bool,
    ) -> Vec<QueryHandle> {
        let (to_refetch, jobs, waiters, wake) = self.inner.state.with(|s| {
            let mut active = Vec::new();
            let mut jobs = Vec::new();
            let mut waiters = Vec::new();
            let mut invalidated_slots = Vec::new();
            let mut touched_any = false;

            if invalidate_all {
                s.wildcard_epoch = s.wildcard_epoch.wrapping_add(1);
                let active_len = s.active_slots.len();
                for i in 0..active_len {
                    let slot = s.active_slots[i];
                    s.invalidate_slot(slot, &mut active, &mut waiters);
                    invalidated_slots.push(slot);
                    touched_any = true;
                }
            } else {
                for &tag in tags {
                    let mut slots = Vec::new();
                    s.tags.for_each_slot(tag, |slot| slots.push(slot));
                    for slot in slots {
                        if !invalidated_slots.contains(&slot) {
                            s.invalidate_slot(slot, &mut active, &mut waiters);
                            invalidated_slots.push(slot);
                            touched_any = true;
                        }
                    }
                }
                if !tags.is_empty() && s.subscribed_all_count > 0 {
                    for idx in 0..s.metas.len() {
                        if !s.metas[idx].is_free()
                            && (s.metas[idx].flags & FLAG_SUBSCRIBED_ALL) != 0
                        {
                            let slot = QuerySlotId(idx as u32);
                            if !invalidated_slots.contains(&slot) {
                                s.invalidate_slot(slot, &mut active, &mut waiters);
                                invalidated_slots.push(slot);
                                touched_any = true;
                            }
                        }
                    }
                }
            }

            for &qh in &opt_handle.touched {
                if !s.is_live(qh) {
                    continue;
                }
                if !invalidated_slots.contains(&qh.slot) {
                    s.invalidate_slot(qh.slot, &mut active, &mut waiters);
                    invalidated_slots.push(qh.slot);
                    touched_any = true;
                }
                let idx = qh.slot.index();
                let target_gen = s.metas[idx].fetch_gen;
                if mark_mutation_settling(&mut s.payloads[idx], opt_handle.mutation_id, target_gen)
                {
                    let _ = s.mark_dirty(qh.slot);
                    touched_any = true;
                }
                if s.metas[idx].subscriber_count == 0
                    && remove_mutation_layers(&mut s.payloads[idx], opt_handle.mutation_id)
                    && let Some(job) =
                        prepare_recompute_job(qh.slot, &mut s.metas[idx], &mut s.payloads[idx])
                {
                    jobs.push(job);
                }
            }

            let wake = if touched_any || invalidate_all {
                s.wake_cb.clone()
            } else {
                None
            };
            (active, jobs, waiters, wake)
        });
        let recompute_wake = self.recompute_jobs_outside_lock(jobs);
        run_side_effects(recompute_wake.or(wake), waiters);
        to_refetch
    }

    /// Executes action `P`, applying server `Nve-Invalidate` + static `P::invalidates(args)` on
    /// success and invalidating static tags on ambiguous transport errors.
    pub async fn execute_action<P>(
        &self,
        args: &P::Args,
    ) -> Result<(CallReply<P::Output>, Vec<QueryHandle>), RpcError<P::Error>>
    where
        P: Procedure<Effect = Action>,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
    {
        self.execute_action_optimistic::<P, _>(args, |_| {}).await
    }

    /// Executes action `P` with an optimistic patch builder `optimistic` applied immediately
    /// before sending the RPC request.
    pub async fn execute_action_optimistic<P, Opt>(
        &self,
        args: &P::Args,
        optimistic: Opt,
    ) -> Result<(CallReply<P::Output>, Vec<QueryHandle>), RpcError<P::Error>>
    where
        P: Procedure<Effect = Action>,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
        Opt: FnOnce(&mut OptimisticTx<'_, C>),
    {
        let opt_handle = self.apply_optimistic(optimistic);
        self.execute_action_with_handle::<P>(args, opt_handle).await
    }

    /// Executes action `P` for an already-applied [`OptimisticHandle`], atomically settling
    /// invalidations and optimistic layers in one lock acquisition on `Ok`, or rolling back on
    /// `Err` / future drop.
    pub async fn execute_action_with_handle<P>(
        &self,
        args: &P::Args,
        opt_handle: OptimisticHandle,
    ) -> Result<(CallReply<P::Output>, Vec<QueryHandle>), RpcError<P::Error>>
    where
        P: Procedure<Effect = Action>,
        C: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
    {
        let mut static_tags_init = TagSink::new();
        P::invalidates(args, &mut static_tags_init);
        let mut cancel_guard = OptimisticCancelGuard {
            client: self,
            handle: (opt_handle.touched_count() > 0).then_some(opt_handle),
            static_tags_on_cancel: Some(static_tags_init),
        };
        let res = call_with_codec::<P, C>(&self.inner.transport, &self.inner.codec, args).await;
        let opt_handle = cancel_guard.handle.take().unwrap_or(OptimisticHandle {
            mutation_id: MutationId(0),
            touched: Vec::new(),
        });
        let static_tags = cancel_guard
            .static_tags_on_cancel
            .take()
            .unwrap_or_default();

        match res {
            Ok(reply) => {
                let inv_all = reply.invalidate_all || static_tags.is_overflow_all();
                let mut all_tags = if inv_all {
                    Vec::new()
                } else {
                    let mut v =
                        Vec::with_capacity(reply.invalidated_tags.len() + static_tags.len());
                    v.extend_from_slice(&reply.invalidated_tags);
                    for &t in static_tags.as_slice() {
                        if !v.contains(&t) {
                            v.push(t);
                        }
                    }
                    v
                };
                let to_refetch = self.settle_action_atomically(&opt_handle, &all_tags, inv_all);
                all_tags.clear();
                Ok((reply, to_refetch))
            }
            Err(err @ RpcError::Transport(_)) => {
                if opt_handle.touched_count() > 0 {
                    self.rollback_optimistic(&opt_handle);
                }
                if static_tags.is_overflow_all() {
                    let _ = self.invalidate_all();
                } else if !static_tags.is_empty() {
                    let _ = self.invalidate_tags(static_tags.as_slice());
                }
                Err(err)
            }
            Err(err) => {
                if opt_handle.touched_count() > 0 {
                    self.rollback_optimistic(&opt_handle);
                }
                Err(err)
            }
        }
    }

}
