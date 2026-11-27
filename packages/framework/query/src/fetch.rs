//! Sans-I/O fetch lifecycle (`begin_fetch`, `begin_fetch_for`, `complete_fetch`, cancellation)
//! and outside-lock optimistic layer recomputation on [`QueryClient`].

use nve_hash::xxh64;
use nve_rpc::{CallReply, CodecFormat, Procedure, Query, RawCall, RpcError};

use crate::engine::{run_side_effects, QueryClient, BODY_HASH_SEED};
use crate::optimistic::{
    fold_recompute_job, prepare_recompute_job, settle_layers_up_to, RecomputeJob,
};
use crate::slab::{
    QuerySlotId, FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_FETCHING,
    FLAG_IS_INVALIDATED,
};
use crate::types::{into_shared_any, FetchTicket, QueryHandle, Shared};

impl<T, C> QueryClient<T, C> {
    /// Sans-I/O fetch start: transitions `handle` to `IS_FETCHING` and returns its deduplicated
    /// `(FetchTicket, RawCall)`, or `None` if already fetching (dedup!), invalid, or missing a
    /// procedure name (prefer [`Self::begin_fetch_for`] for hydrated slots).
    pub fn begin_fetch(&self, handle: QueryHandle) -> Option<(FetchTicket, RawCall)>
    where
        C: CodecFormat,
    {
        self.begin_fetch_named(handle, None)
    }

    /// Typed sans-I/O fetch start binding `P::NAME` explicitly so even hydrated-only slots always
    /// produce a valid RPC route path.
    pub fn begin_fetch_for<P>(&self, handle: QueryHandle) -> Option<(FetchTicket, RawCall)>
    where
        P: Procedure<Effect = Query>,
        C: CodecFormat,
    {
        self.begin_fetch_named(handle, Some(P::NAME))
    }

    fn begin_fetch_named(
        &self,
        handle: QueryHandle,
        proc_name_override: Option<&'static str>,
    ) -> Option<(FetchTicket, RawCall)>
    where
        C: CodecFormat,
    {
        let (out, wake) = self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return (None, None);
            }
            let idx = handle.slot.index();
            let meta = &mut s.metas[idx];
            if meta.is_fetching() {
                return (None, None);
            }
            let payload = &mut s.payloads[idx];
            if let Some(name) = proc_name_override {
                payload.proc_name = name;
            }
            if payload.proc_name.is_empty() {
                return (None, None);
            }
            meta.flags |= FLAG_IS_FETCHING;
            let ticket = FetchTicket {
                handle,
                fetch_gen: meta.fetch_gen,
                wildcard_epoch: s.wildcard_epoch,
            };
            let call = RawCall {
                proc_id: meta.proc_id,
                name: payload.proc_name,
                effect: nve_rpc::Effect::Query,
                content_type: C::CONTENT_TYPE,
                body: payload.encoded_args.clone(),
            };
            let wake = s.mark_dirty(handle.slot);
            (Some((ticket, call)), wake)
        });
        if let Some(cb) = wake {
            cb();
        }
        out
    }

    /// Sans-I/O fetch completion: commits `result` if `ticket` is still current (`slot_gen`,
    /// `fetch_gen`, and `wildcard_epoch` all match), performs structural-sharing equality check,
    /// retires settled optimistic layers (recomputing outside the lock), wakes deduplicated
    /// waiters, and notifies observers. Returns `false` if the reply was stale and discarded.
    pub fn complete_fetch<P>(
        &self,
        ticket: FetchTicket,
        result: Result<CallReply<P::Output>, RpcError<P::Error>>,
    ) -> bool
    where
        P: Procedure<Effect = Query>,
    {
        self.complete_fetch_shared::<P>(ticket, result.map_err(Shared::new))
    }

    pub(crate) fn complete_fetch_shared<P>(
        &self,
        ticket: FetchTicket,
        result: Result<CallReply<P::Output>, Shared<RpcError<P::Error>>>,
    ) -> bool
    where
        P: Procedure<Effect = Query>,
    {
        let now_ms = self.inner.clock.now_ms();
        let (accepted, wake, waiters, job) = self.inner.state.with(|s| {
            if !s.is_live(ticket.handle) {
                return (false, None, Vec::new(), None);
            }
            let idx = ticket.handle.slot.index();
            let meta = &mut s.metas[idx];
            if meta.proc_id != P::ID
                || meta.fetch_gen != ticket.fetch_gen
                || s.wildcard_epoch != ticket.wildcard_epoch
            {
                let waiters = if meta.proc_id == P::ID && meta.fetch_gen == ticket.fetch_gen {
                    meta.flags &= !FLAG_IS_FETCHING;
                    core::mem::take(&mut s.payloads[idx].in_flight_waiters)
                } else {
                    Vec::new()
                };
                return (false, None, waiters, None);
            }
            meta.flags &= !(FLAG_IS_FETCHING | FLAG_HYDRATED);
            let payload = &mut s.payloads[idx];
            let waiters = core::mem::take(&mut payload.in_flight_waiters);

            match result {
                Ok(reply) => {
                    let body_hash = xxh64(&reply.raw_body, BODY_HASH_SEED);
                    let same_bytes = meta.has_data()
                        && !meta.has_error()
                        && payload.raw_body_hash == body_hash
                        && payload
                            .raw_response
                            .as_ref()
                            .is_some_and(|prev| prev.as_ref() == reply.raw_body.as_ref());
                    let has_layers = !payload.optimistic_layers.is_empty();

                    meta.updated_at_ms = now_ms;
                    meta.invalidated_epoch = ticket.wildcard_epoch;
                    meta.flags |= FLAG_HAS_DATA;
                    meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED);

                    let job = if !same_bytes || has_layers {
                        payload.raw_response = Some(reply.raw_body);
                        payload.raw_body_hash = body_hash;
                        payload.committed = Some(into_shared_any(reply.value));
                        payload.committed_error = None;
                        settle_layers_up_to(payload, ticket.fetch_gen);
                        prepare_recompute_job(ticket.handle.slot, meta, payload)
                    } else {
                        None
                    };
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake, waiters, job)
                }
                Err(ref err)
                    if matches!(
                        err.as_ref(),
                        RpcError::Transport(terr)
                            if terr.kind == nve_rpc::TransportErrorKind::Cancelled
                    ) =>
                {
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake, waiters, None)
                }
                Err(shared_err) => {
                    meta.flags |= FLAG_HAS_ERROR;
                    payload.committed_error = Some(shared_err);
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake, waiters, None)
                }
            }
        });
        if let Some(job) = job {
            self.recompute_jobs_outside_lock(vec![job]);
        }
        run_side_effects(wake, waiters);
        accepted
    }

    /// Cancels an in-flight ticket when the driving future is dropped mid-`.await`, clearing
    /// `FLAG_IS_FETCHING` and waking any deduplicated waiters so the slot never wedges.
    pub(crate) fn cancel_fetch_ticket(&self, ticket: FetchTicket) {
        let (wake, waiters) = self.inner.state.with(|s| {
            if !s.is_live(ticket.handle) {
                return (None, Vec::new());
            }
            let idx = ticket.handle.slot.index();
            let meta = &mut s.metas[idx];
            if meta.fetch_gen != ticket.fetch_gen || !meta.is_fetching() {
                return (None, Vec::new());
            }
            meta.flags &= !FLAG_IS_FETCHING;
            let waiters = core::mem::take(&mut s.payloads[idx].in_flight_waiters);
            let wake = s.mark_dirty(ticket.handle.slot);
            (wake, waiters)
        });
        run_side_effects(wake, waiters);
    }

    /// Executes every optimistic layer closure strictly OUTSIDE the cache lock and commits the
    /// folded values only if each slot's `slot_gen` and `optimistic_rev` still match (retrying
    /// if a concurrent mutation raced).
    pub(crate) fn recompute_jobs_outside_lock(&self, mut jobs: Vec<RecomputeJob>) {
        while !jobs.is_empty() {
            let computed: Vec<(QuerySlotId, u32, u16, crate::types::SharedAny)> = jobs
                .iter()
                .map(|job| (job.slot, job.generation, job.rev, fold_recompute_job(job)))
                .collect();
            jobs.clear();

            self.inner.state.with(|s| {
                for (slot, generation, rev, value) in computed {
                    let handle = QueryHandle { slot, generation };
                    if !s.is_live(handle) {
                        continue;
                    }
                    let idx = slot.index();
                    if s.metas[idx].optimistic_rev == rev {
                        s.payloads[idx].effective_optimistic = Some(value);
                    } else if let Some(retry) =
                        prepare_recompute_job(slot, &mut s.metas[idx], &mut s.payloads[idx])
                    {
                        jobs.push(retry);
                    }
                }
            });
        }
    }
}
