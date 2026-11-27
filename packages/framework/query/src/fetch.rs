//! Sans-I/O fetch lifecycle (`begin_fetch`, `begin_fetch_for`, `complete_fetch`, cancellation)
//! and outside-lock optimistic layer recomputation on [`QueryClient`].

use nve_hash::xxh64;
use nve_rpc::{CallReply, CodecFormat, ProcId, Procedure, Query, RawCall, RpcError};

use crate::engine::{run_side_effects, QueryClient, BODY_HASH_SEED};
use crate::optimistic::{
    fold_recompute_job, prepare_recompute_job, settle_layers_up_to, RecomputeJob,
};
use crate::slab::{
    QuerySlotId, FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_FETCHING,
    FLAG_IS_INVALIDATED,
};
use crate::types::{into_shared_any, FetchTicket, QueryHandle, Shared};

pub(crate) enum FetchStart {
    Started(FetchTicket, RawCall),
    Wait { completed_seq: u32, inv_ver: u64 },
    Unavailable,
}

impl<T, C> QueryClient<T, C> {
    /// Sans-I/O fetch start: transitions `handle` to `IS_FETCHING` and returns its deduplicated
    /// `(FetchTicket, RawCall)`, or `None` if already fetching (dedup!), invalid, or missing a
    /// procedure name (prefer [`Self::begin_fetch_for`] for hydrated slots).
    pub fn begin_fetch(&self, handle: QueryHandle) -> Option<(FetchTicket, RawCall)>
    where
        C: CodecFormat,
    {
        match self.begin_fetch_inner(handle, None, None) {
            FetchStart::Started(ticket, call) => Some((ticket, call)),
            FetchStart::Wait { .. } | FetchStart::Unavailable => None,
        }
    }

    /// Typed sans-I/O fetch start verifying `meta.proc_id == P::ID` and binding `P::NAME`
    /// explicitly so even hydrated-only slots always produce a valid RPC route path.
    pub fn begin_fetch_for<P>(&self, handle: QueryHandle) -> Option<(FetchTicket, RawCall)>
    where
        P: Procedure<Effect = Query>,
        C: CodecFormat,
    {
        match self.begin_fetch_inner(handle, Some(P::ID), Some(P::NAME)) {
            FetchStart::Started(ticket, call) => Some((ticket, call)),
            FetchStart::Wait { .. } | FetchStart::Unavailable => None,
        }
    }

    /// Atomically starts a fetch for `(P, handle)` OR returns the `(completed_seq, inv_ver)` wait
    /// snapshot in one lock acquisition so multi-threaded callers never observe a TOCTOU gap
    /// between checking `is_fetching()` and recording the pre-wait watermark.
    pub(crate) fn begin_fetch_or_wait_snapshot<P>(&self, handle: QueryHandle) -> FetchStart
    where
        P: Procedure<Effect = Query>,
        C: CodecFormat,
    {
        self.begin_fetch_inner(handle, Some(P::ID), Some(P::NAME))
    }

    fn begin_fetch_inner(
        &self,
        handle: QueryHandle,
        expected_proc_id: Option<ProcId>,
        proc_name_override: Option<&'static str>,
    ) -> FetchStart
    where
        C: CodecFormat,
    {
        let (out, wake) = self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return (FetchStart::Unavailable, None);
            }
            let idx = handle.slot.index();
            let meta = &mut s.metas[idx];
            if let Some(pid) = expected_proc_id
                && meta.proc_id != pid
            {
                return (FetchStart::Unavailable, None);
            }
            if meta.is_fetching() {
                let inv_ver = ((u64::from(s.wildcard_epoch)) << 32) | u64::from(meta.fetch_gen);
                return (
                    FetchStart::Wait {
                        completed_seq: s.payloads[idx].completed_seq,
                        inv_ver,
                    },
                    None,
                );
            }
            let payload = &mut s.payloads[idx];
            if let Some(name) = proc_name_override {
                payload.proc_name = name;
            }
            if payload.proc_name.is_empty() {
                return (FetchStart::Unavailable, None);
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
            (FetchStart::Started(ticket, call), wake)
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
                let (wake, waiters) =
                    if meta.proc_id == P::ID && meta.fetch_gen == ticket.fetch_gen {
                        meta.flags &= !FLAG_IS_FETCHING;
                        let waiters = core::mem::take(&mut s.payloads[idx].in_flight_waiters);
                        let wake = s.mark_dirty(ticket.handle.slot);
                        (wake, waiters)
                    } else {
                        (None, Vec::new())
                    };
                return (false, wake, waiters, None);
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

                    meta.updated_at_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
                    meta.invalidated_epoch = ticket.wildcard_epoch;
                    meta.flags |= FLAG_HAS_DATA;
                    meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED);
                    payload.completed_seq = payload.completed_seq.wrapping_add(1).max(1);

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
                    payload.completed_seq = payload.completed_seq.wrapping_add(1).max(1);
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake, waiters, None)
                }
            }
        });
        let recompute_wake = job.and_then(|j| self.recompute_jobs_outside_lock(vec![j]));
        run_side_effects(recompute_wake.or(wake), waiters);
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
    /// if a concurrent mutation raced). Returns the registered wake callback if any slot became
    /// dirty so callers can fire ONE coalesced wake after releasing all locks.
    pub(crate) fn recompute_jobs_outside_lock(
        &self,
        mut jobs: Vec<RecomputeJob>,
    ) -> Option<crate::state::WakeCallback> {
        let mut wake_out = None;
        while !jobs.is_empty() {
            let computed: Vec<(QuerySlotId, u32, u16, crate::types::SharedAny)> = jobs
                .iter()
                .map(|job| (job.slot, job.generation, job.rev, fold_recompute_job(job)))
                .collect();
            jobs.clear();

            let wake = self.inner.state.with(|s| {
                let mut wake_cb = None;
                for (slot, generation, rev, value) in computed {
                    let handle = QueryHandle { slot, generation };
                    if !s.is_live(handle) {
                        continue;
                    }
                    let idx = slot.index();
                    if s.metas[idx].optimistic_rev == rev {
                        s.payloads[idx].effective_optimistic = Some(value);
                        if let Some(cb) = s.mark_dirty(slot) {
                            wake_cb = Some(cb);
                        }
                    } else if let Some(retry) =
                        prepare_recompute_job(slot, &mut s.metas[idx], &mut s.payloads[idx])
                    {
                        jobs.push(retry);
                    }
                }
                wake_cb
            });
            if wake.is_some() {
                wake_out = wake;
            }
        }
        wake_out
    }
}

#[cfg(test)]
mod tests {
    use nve_rpc::{procedures, BoxFuture, CborCodec, RawCall, RawReply, Transport, TransportError};

    use super::FetchStart;
    use crate::{QueryClient, QueryOptions};

    procedures! {
        query FetchA = "fetch.v1.Test/A" (u64) -> u64, ();
        query FetchB = "fetch.v1.Test/B" (u64) -> u64, ();
    }

    #[derive(Clone, Default)]
    struct DummyTransport;

    impl Transport for DummyTransport {
        fn call_raw<'a>(&'a self, _req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
            Box::pin(async { Err(TransportError::network("unused")) })
        }
    }

    #[test]
    fn begin_fetch_or_wait_snapshot_transitions_atomically_across_cancel_and_complete() {
        let clock = crate::Shared::new(crate::ManualClock::new(1_000));
        let client = QueryClient::with_clock(
            DummyTransport,
            CborCodec::default(),
            clock.clone(),
            crate::QueryClientConfig::default(),
        );
        let h = client
            .subscribe::<FetchA>(&1, QueryOptions::default().gc_time_ms(0))
            .unwrap();

        // 1. Cold slot starts atomically and marks slot in-flight.
        let ticket1 = match client.begin_fetch_or_wait_snapshot::<FetchA>(h) {
            FetchStart::Started(ticket, call) => {
                assert_eq!(call.name, <FetchA as nve_rpc::Procedure>::NAME);
                ticket
            }
            _ => panic!("expected FetchStart::Started on cold slot"),
        };

        // 2. Concurrent follower sees exact pre-wait `(completed_seq=0, inv_ver)` in one lock.
        let expected_inv_ver = client.read::<FetchA>(h).invalidation_version;
        match client.begin_fetch_or_wait_snapshot::<FetchA>(h) {
            FetchStart::Wait {
                completed_seq,
                inv_ver,
            } => {
                assert_eq!(completed_seq, 0);
                assert_eq!(inv_ver, expected_inv_ver);
            }
            _ => panic!("expected FetchStart::Wait while in-flight"),
        }

        // 3. Mismatched procedure returns Unavailable even while slot is live & in-flight.
        assert!(matches!(
            client.begin_fetch_or_wait_snapshot::<FetchB>(h),
            FetchStart::Unavailable
        ));

        // 4. Immediately after leader cancel, next caller atomically takes over (`Started`, never
        //    `Unavailable` or stale `Wait`).
        client.cancel_fetch_ticket(ticket1);
        let ticket2 = match client.begin_fetch_or_wait_snapshot::<FetchA>(h) {
            FetchStart::Started(ticket, _) => ticket,
            _ => panic!("expected atomic takeover FetchStart::Started after cancel"),
        };

        // 5. Completing ticket2 bumps `completed_seq` to 1; a subsequent fetch exposes `1` to waiters.
        let reply = nve_rpc::CallReply {
            value: 42u64,
            raw_body: bytes::Bytes::from_static(&[0x18, 0x2a]),
            invalidated_tags: Vec::new(),
            invalidate_all: false,
        };
        assert!(client.complete_fetch::<FetchA>(ticket2, Ok(reply)));
        let ticket3 = match client.begin_fetch_or_wait_snapshot::<FetchA>(h) {
            FetchStart::Started(ticket, _) => ticket,
            _ => panic!("expected Started after prior fetch completed"),
        };
        match client.begin_fetch_or_wait_snapshot::<FetchA>(h) {
            FetchStart::Wait { completed_seq, .. } => assert_eq!(completed_seq, 1),
            _ => panic!("expected Wait with completed_seq=1"),
        }
        client.cancel_fetch_ticket(ticket3);

        // 6. Evicted handle returns Unavailable.
        client.unsubscribe(h);
        clock.advance(1);
        assert_eq!(client.gc_sweep(64), 1);
        assert!(matches!(
            client.begin_fetch_or_wait_snapshot::<FetchA>(h),
            FetchStart::Unavailable
        ));
    }
}
