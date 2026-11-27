//! Optimistic patch transactions, action execution with automatic invalidation, O(k) tag & O(1)
//! wildcard invalidation, focus/reconnect hooks, bounded GC, and `NVEQ` dehydration/hydration.

use core::future::Future;

use nve_hash::xxh64;
use nve_rpc::{
    call_with_codec, decode_reply, Action, CallReply, MaybeSend, MaybeSync, Procedure, Query,
    RpcCodec, RpcError, Tag64, TagSink, Transport,
};

use crate::engine::{query_key_hash, QueryClient, BODY_HASH_SEED};
use crate::hydrate::{
    decode_hydration_entries, encode_hydration_entries, DehydrateEntryRef, DehydrateError,
};
use crate::optimistic::{
    apply_layer_to_slot, mark_mutation_settling, remove_mutation_layers, OptimisticHandle,
    OptimisticTx, PendingPatchOp,
};
use crate::slab::{
    QuerySlotId, FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_INVALIDATED,
    FLAG_REFETCH_ON_FOCUS, FLAG_REFETCH_ON_RECONNECT, FLAG_SUBSCRIBED_ALL,
};
use crate::state::WakeCallback;
use crate::types::{MutationId, QueryHandle, Shared};

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    /// Executes one network fetch attempt for `handle` without holding the cache lock across
    /// `.await`. Deduplicates concurrent calls on the same slot with zero error cloning.
    pub async fn fetch_handle<P>(
        &self,
        handle: QueryHandle,
    ) -> Result<Option<Shared<P::Output>>, Shared<RpcError<P::Error>>>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Output> + RpcCodec<P::Error>,
    {
        let Some((ticket, raw_call)) = self.begin_fetch(handle) else {
            return Ok(self.read::<P>(handle).data);
        };
        let raw_res = self.inner.transport.call_raw(raw_call).await;
        let decoded = match raw_res {
            Ok(raw_reply) => decode_reply::<P, C>(&self.inner.codec, raw_reply),
            Err(terr) => Err(RpcError::Transport(terr)),
        };
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
        let res = self.fetch_handle::<P>(handle).await;
        self.unsubscribe(handle);
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

        let (handle, wake) = self.inner.state.with(|s| {
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
                            if s.payloads[idx].committed.is_some() {
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                wake_cb = s.mark_dirty(slot);
                                touched.push(QueryHandle {
                                    slot,
                                    generation: s.metas[idx].slot_gen,
                                });
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
                                && s.payloads[idx].committed.is_some()
                            {
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                wake_cb = s.mark_dirty(slot);
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
                                && s.payloads[idx].committed.is_some()
                            {
                                let slot = QuerySlotId(idx as u32);
                                apply_layer_to_slot(&mut s.payloads[idx], make_layer(mutation_id));
                                wake_cb = s.mark_dirty(slot);
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

            (
                OptimisticHandle {
                    mutation_id,
                    touched,
                },
                wake_cb,
            )
        });

        if let Some(cb) = wake {
            cb();
        }
        handle
    }

    /// Rolls back all optimistic layers belonging to `handle` without disturbing sibling mutations.
    pub fn rollback_optimistic(&self, handle: &OptimisticHandle) {
        let wake = self.inner.state.with(|s| {
            let mut wake_cb: Option<WakeCallback> = None;
            for &qh in &handle.touched {
                if !s.is_live(qh) {
                    continue;
                }
                if remove_mutation_layers(&mut s.payloads[qh.slot.index()], handle.mutation_id) {
                    wake_cb = s.mark_dirty(qh.slot);
                }
            }
            wake_cb
        });
        if let Some(cb) = wake {
            cb();
        }
    }

    /// Marks all slots touched by `handle` as invalidated and arms their optimistic layers to
    /// retire automatically once the post-action refetch commits (zero UI flicker).
    pub fn settle_optimistic(&self, handle: &OptimisticHandle) -> Vec<QueryHandle> {
        let (to_refetch, wake) = self.inner.state.with(|s| {
            let mut active = Vec::new();
            let mut wake_cb: Option<WakeCallback> = None;
            for &qh in &handle.touched {
                if !s.is_live(qh) {
                    continue;
                }
                s.invalidate_slot(qh.slot, &mut active);
                let target_gen = s.metas[qh.slot.index()].fetch_gen;
                if mark_mutation_settling(
                    &mut s.payloads[qh.slot.index()],
                    handle.mutation_id,
                    target_gen,
                ) {
                    wake_cb = s.mark_dirty(qh.slot);
                }
                if s.metas[qh.slot.index()].subscriber_count == 0 {
                    remove_mutation_layers(&mut s.payloads[qh.slot.index()], handle.mutation_id);
                }
            }
            (active, wake_cb)
        });
        if let Some(cb) = wake {
            cb();
        }
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
        let res = call_with_codec::<P, C>(&self.inner.transport, &self.inner.codec, args).await;

        let mut static_tags = TagSink::new();
        P::invalidates(args, &mut static_tags);

        match res {
            Ok(reply) => {
                let mut to_refetch = if reply.invalidate_all || static_tags.is_overflow_all() {
                    self.invalidate_all()
                } else {
                    let mut all_tags =
                        Vec::with_capacity(reply.invalidated_tags.len() + static_tags.len());
                    all_tags.extend_from_slice(&reply.invalidated_tags);
                    for &t in static_tags.as_slice() {
                        if !all_tags.contains(&t) {
                            all_tags.push(t);
                        }
                    }
                    self.invalidate_tags(&all_tags)
                };
                if opt_handle.touched_count() > 0 {
                    for h in self.settle_optimistic(&opt_handle) {
                        if !to_refetch.contains(&h) {
                            to_refetch.push(h);
                        }
                    }
                }
                Ok((reply, to_refetch))
            }
            Err(err @ RpcError::Transport(_)) => {
                self.rollback_optimistic(&opt_handle);
                if static_tags.is_overflow_all() {
                    let _ = self.invalidate_all();
                } else if !static_tags.is_empty() {
                    let _ = self.invalidate_tags(static_tags.as_slice());
                }
                Err(err)
            }
            Err(err) => {
                self.rollback_optimistic(&opt_handle);
                Err(err)
            }
        }
    }

    /// Invalidates all queries tagged with any tag in `tags` (O(k) via flat `TagEdge` pool) into
    /// caller-reused scratch buffer `active_out` (zero heap allocations when pre-sized).
    pub fn invalidate_tags_into(&self, tags: &[Tag64], active_out: &mut Vec<QueryHandle>) {
        active_out.clear();
        let wake = self.inner.state.with(|s| {
            let mut visited_count = 0usize;
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
                        s.invalidate_slot(slot, active_out);
                        visited_count += 1;
                    }
                } else {
                    for &slot in &cur_slots[..len] {
                        s.invalidate_slot(slot, active_out);
                        visited_count += 1;
                    }
                }
            }
            if !tags.is_empty() {
                let active_len = s.active_slots.len();
                for i in 0..active_len {
                    let slot = s.active_slots[i];
                    if (s.metas[slot.index()].flags & FLAG_SUBSCRIBED_ALL) != 0 {
                        s.invalidate_slot(slot, active_out);
                        visited_count += 1;
                    }
                }
            }
            if visited_count > 0 {
                s.wake_cb.clone()
            } else {
                None
            }
        });
        if let Some(cb) = wake {
            cb();
        }
    }

    /// Invalidates all queries tagged with any tag in `tags`, returning active mounted handles
    /// that should refetch.
    pub fn invalidate_tags(&self, tags: &[Tag64]) -> Vec<QueryHandle> {
        let mut active = Vec::new();
        self.invalidate_tags_into(tags, &mut active);
        active
    }

    /// O(1) wildcard `*` invalidation (`wildcard_epoch += 1`), marking only active mounted
    /// queries dirty and returning their handles for refetch.
    pub fn invalidate_all(&self) -> Vec<QueryHandle> {
        let (active, wake) = self.inner.state.with(|s| {
            s.wildcard_epoch = s.wildcard_epoch.wrapping_add(1);
            let mut out = Vec::with_capacity(s.active_slots.len());
            let active_len = s.active_slots.len();
            for i in 0..active_len {
                let slot = s.active_slots[i];
                s.invalidate_slot(slot, &mut out);
            }
            (out, s.wake_cb.clone())
        });
        if let Some(cb) = wake {
            cb();
        }
        active
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
        self.inner.state.with(|s| {
            let mut out = Vec::new();
            for &slot in &s.active_slots {
                let meta = s.metas[slot.index()];
                if (meta.flags & required_flag) != 0
                    && !meta.is_fetching()
                    && (meta.has_error() || meta.is_stale(now_ms, s.wildcard_epoch))
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
    /// expired inactive slots were evicted.
    pub fn gc_sweep(&self, max_steps: usize) -> usize {
        let now_ms = self.inner.clock.now_ms();
        self.inner
            .state
            .with(|s| s.gc_sweep(now_ms, max_steps, self.inner.config.max_entries))
    }

    /// Dehydrates all live entries with raw CBOR response bytes into a compact `NVEQ` binary blob
    /// for SSR -> client hydration (zero `serde_json`, zero type registry).
    pub fn dehydrate_bytes(&self) -> Vec<u8> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut tag_bufs: Vec<Vec<Tag64>> = Vec::new();
            let mut slot_indices: Vec<usize> = Vec::new();
            for (idx, meta) in s.metas.iter().enumerate() {
                if meta.is_free() || !meta.has_data() {
                    continue;
                }
                if s.payloads[idx].raw_response.is_some() {
                    let mut tags = Vec::new();
                    s.tags.collect_tags_for_slot(meta.first_tag_edge, &mut tags);
                    tag_bufs.push(tags);
                    slot_indices.push(idx);
                }
            }
            let refs: Vec<DehydrateEntryRef<'_>> = slot_indices
                .iter()
                .zip(tag_bufs.iter())
                .map(|(&idx, tags)| {
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    let age_ms = u32::try_from(now_ms.saturating_sub(meta.updated_at_ms))
                        .unwrap_or(u32::MAX);
                    DehydrateEntryRef {
                        proc_id: meta.proc_id,
                        age_ms,
                        tags: tags.as_slice(),
                        encoded_args: &payload.encoded_args,
                        raw_body: payload.raw_response.as_deref().unwrap_or(&[]),
                    }
                })
                .collect();
            encode_hydration_entries(&refs)
        })
    }

    /// Seeds cache slots from a dehydrated `NVEQ` binary blob without decoding typed values yet
    /// (decoding happens lazily on the first typed [`Self::read`] call, and tags are indexed
    /// immediately so pre-read invalidations work).
    pub fn hydrate_bytes(&self, bytes: &[u8]) -> Result<usize, DehydrateError> {
        let entries = decode_hydration_entries(bytes)?;
        let now_ms = self.inner.clock.now_ms();
        let count = entries.len();
        self.inner.state.with(|s| {
            for entry in entries {
                let key_hash = query_key_hash(entry.proc_id, &entry.encoded_args);
                let updated_at = now_ms.saturating_sub(u64::from(entry.age_ms));
                let body_hash = xxh64(&entry.raw_body, BODY_HASH_SEED);

                let slot = if let Some(existing) =
                    s.lookup_slot(entry.proc_id, key_hash, &entry.encoded_args)
                {
                    existing
                } else {
                    s.insert_slot(
                        entry.proc_id,
                        "",
                        key_hash,
                        entry.encoded_args,
                        self.inner.config.default_options,
                        &entry.tags,
                        false,
                        updated_at,
                    )
                    .slot
                };

                let idx = slot.index();
                let meta = &mut s.metas[idx];
                meta.updated_at_ms = updated_at;
                meta.invalidated_epoch = s.wildcard_epoch;
                meta.flags |= FLAG_HAS_DATA | FLAG_HYDRATED;
                meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED);

                let payload = &mut s.payloads[idx];
                payload.raw_response = Some(entry.raw_body);
                payload.raw_body_hash = body_hash;
                payload.committed = None;
                payload.committed_error = None;
            }
        });
        Ok(count)
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
