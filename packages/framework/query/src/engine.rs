//! Sans-I/O data-oriented [`QueryClient`] core: subscriptions, typed zero-clone reads,
//! deduplicated fetch lifecycle, stale-response generation guards, and prefetching.

use bytes::Bytes;
use nve_hash::xxh64;
use nve_rpc::{
    encode_args, CallReply, CborCodec, CodecError, MaybeSend, MaybeSync, ProcId, Procedure, Query,
    RawCall, RpcCodec, RpcError, TagSink, Transport,
};

use crate::optimistic::settle_layers_up_to;
use crate::slab::{
    QuerySlotId, FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IN_DIRTY_QUEUE,
    FLAG_IS_FETCHING, FLAG_IS_INVALIDATED, FLAG_REFETCH_ON_FOCUS, FLAG_REFETCH_ON_RECONNECT,
    FLAG_SUBSCRIBED_ALL, NONE_IDX,
};
use crate::state::CacheState;
use crate::types::{
    downcast_shared, into_shared_any, Clock, FetchTicket, QueryClientConfig, QueryHandle,
    QueryMode, QueryOptions, QuerySnapshot, Shared, SystemClock,
};

pub use crate::hydrate::DehydrateError;
pub use crate::state::WakeCallback;

pub(crate) const QUERY_ARGS_SEED: u64 = 0x5155_4552_595F_4B45; // "QUERY_KE"
pub(crate) const BODY_HASH_SEED: u64 = 0x424F_4459_5F48_5348; // "BODY_HSH"

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct CacheLock<T>(std::sync::Mutex<T>);

#[cfg(not(target_arch = "wasm32"))]
impl<T> CacheLock<T> {
    fn new(val: T) -> Self {
        Self(std::sync::Mutex::new(val))
    }

    #[inline]
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct CacheLock<T>(core::cell::RefCell<T>);

#[cfg(target_arch = "wasm32")]
impl<T> CacheLock<T> {
    fn new(val: T) -> Self {
        Self(core::cell::RefCell::new(val))
    }

    #[inline]
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut guard = self.0.borrow_mut();
        f(&mut guard)
    }
}

pub(crate) struct QueryClientInner<T, C> {
    pub(crate) transport: T,
    pub(crate) codec: C,
    pub(crate) clock: Shared<dyn Clock>,
    pub(crate) config: QueryClientConfig,
    pub(crate) state: CacheLock<CacheState>,
}

/// Runtime-agnostic, sans-I/O query & mutation cache client (`Clone` via `Arc` on native / `Rc`
/// on `wasm32`).
///
/// The interior lock is NEVER held across `.await` or user callbacks.
pub struct QueryClient<T, C = CborCodec> {
    pub(crate) inner: Shared<QueryClientInner<T, C>>,
}

impl<T, C> Clone for QueryClient<T, C> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: Shared::clone(&self.inner),
        }
    }
}

impl<T: Transport> QueryClient<T, CborCodec> {
    /// Creates a client in interactive [`QueryMode::Client`] using [`CborCodec::default`] and
    /// [`SystemClock::new`].
    pub fn new(transport: T) -> Self {
        Self::with_clock(
            transport,
            CborCodec::default(),
            Shared::new(SystemClock::new()),
            QueryClientConfig::default(),
        )
    }

    /// Creates a per-request SSR client in [`QueryMode::SsrReadOnly`] (never fires background
    /// fetches during synchronous render; use [`Self::prefetch`] beforehand).
    pub fn ssr(transport: T) -> Self {
        Self::with_clock(
            transport,
            CborCodec::default(),
            Shared::new(SystemClock::new()),
            QueryClientConfig {
                mode: QueryMode::SsrReadOnly,
                ..QueryClientConfig::default()
            },
        )
    }
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    pub fn with_clock(
        transport: T,
        codec: C,
        clock: Shared<dyn Clock>,
        config: QueryClientConfig,
    ) -> Self {
        Self {
            inner: Shared::new(QueryClientInner {
                transport,
                codec,
                clock,
                config,
                state: CacheLock::new(CacheState::new()),
            }),
        }
    }

    #[inline]
    pub fn transport(&self) -> &T {
        &self.inner.transport
    }

    #[inline]
    pub fn codec(&self) -> &C {
        &self.inner.codec
    }

    #[inline]
    pub fn config(&self) -> QueryClientConfig {
        self.inner.config
    }

    #[inline]
    pub fn mode(&self) -> QueryMode {
        self.inner.config.mode
    }

    #[inline]
    pub fn now_ms(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    /// Registers a host wake callback invoked whenever one or more cache slots change state.
    pub fn set_wake_callback(&self, cb: WakeCallback) {
        self.inner.state.with(|s| {
            s.wake_cb = Some(cb);
        });
    }

    /// Drains dirty `(QuerySlotId, data_version)` pairs since the last drain for reactive UI
    /// bridges (e.g. `DioxusQueryBridge`). Zero heap allocations when reusing `out`.
    pub fn drain_dirty_slots(&self, out: &mut Vec<(QuerySlotId, u32)>) {
        out.clear();
        self.inner.state.with(|s| {
            for slot in s.dirty_slots.drain(..) {
                let meta = &mut s.metas[slot.index()];
                meta.flags &= !FLAG_IN_DIRTY_QUEUE;
                if !meta.is_free() {
                    out.push((slot, meta.data_version));
                }
            }
        });
    }

    /// Subscribes an observer to query `(P, args)`, creating or reusing its dense cache slot and
    /// attaching its static `P::tags(args)` to the inverted `Tag64` index.
    pub fn subscribe<P>(
        &self,
        args: &P::Args,
        options: QueryOptions,
    ) -> Result<QueryHandle, CodecError>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Args>,
    {
        let encoded_args = encode_args::<P, C>(&self.inner.codec, args)?;
        let key_hash = query_key_hash(P::ID, &encoded_args);
        let now_ms = self.inner.clock.now_ms();
        let mut sink = TagSink::new();
        P::tags(args, &mut sink);

        let handle = self.inner.state.with(|s| {
            let handle = if let Some(slot) = s.lookup_slot(P::ID, key_hash, &encoded_args) {
                let idx = slot.index();
                let meta = &mut s.metas[idx];
                meta.stale_time_ms = options.stale_time_ms;
                meta.gc_time_ms = options.gc_time_ms;
                if options.refetch_on_focus {
                    meta.flags |= FLAG_REFETCH_ON_FOCUS;
                } else {
                    meta.flags &= !FLAG_REFETCH_ON_FOCUS;
                }
                if options.refetch_on_reconnect {
                    meta.flags |= FLAG_REFETCH_ON_RECONNECT;
                } else {
                    meta.flags &= !FLAG_REFETCH_ON_RECONNECT;
                }
                let payload = &mut s.payloads[idx];
                payload.proc_name = P::NAME;
                payload.options = options;
                if !sink.is_overflow_all() && !sink.is_empty() {
                    let mut first_edge = meta.first_tag_edge;
                    s.tags.attach_tags(slot, &mut first_edge, sink.as_slice());
                    s.metas[idx].first_tag_edge = first_edge;
                } else if sink.is_overflow_all() {
                    s.metas[idx].flags |= FLAG_SUBSCRIBED_ALL;
                }
                QueryHandle {
                    slot,
                    generation: s.metas[idx].slot_gen,
                }
            } else {
                s.gc_sweep(now_ms, 16, self.inner.config.max_entries);
                s.insert_slot(
                    P::ID,
                    P::NAME,
                    key_hash,
                    encoded_args,
                    options,
                    sink.as_slice(),
                    sink.is_overflow_all(),
                    now_ms,
                )
            };

            let idx = handle.slot.index();
            let meta = &mut s.metas[idx];
            let was_zero = meta.subscriber_count == 0;
            meta.subscriber_count = meta.subscriber_count.saturating_add(1);
            if was_zero {
                let pos = s.active_slots.len() as u32;
                s.active_slots.push(handle.slot);
                s.payloads[idx].active_pos = pos;
            }
            handle
        });
        Ok(handle)
    }

    /// Unsubscribes one observer from `handle`. Starts the GC countdown when `subscriber_count`
    /// reaches `0`.
    pub fn unsubscribe(&self, handle: QueryHandle) {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return;
            }
            let idx = handle.slot.index();
            let meta = &mut s.metas[idx];
            meta.subscriber_count = meta.subscriber_count.saturating_sub(1);
            if meta.subscriber_count == 0 {
                let pos = s.payloads[idx].active_pos;
                if pos != NONE_IDX {
                    let pos_usize = pos as usize;
                    s.active_slots.swap_remove(pos_usize);
                    if let Some(&moved) = s.active_slots.get(pos_usize) {
                        s.payloads[moved.index()].active_pos = pos;
                    }
                    s.payloads[idx].active_pos = NONE_IDX;
                }
                s.payloads[idx].last_inactive_ms = now_ms;
            }
        });
    }

    /// Reads the current typed [`QuerySnapshot`] for `handle` (zero heap allocations on cache hit;
    /// lazily decodes `raw_response` once for hydrated entries).
    pub fn read<P>(&self, handle: QueryHandle) -> QuerySnapshot<P::Output, P::Error>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Output>,
    {
        let now_ms = self.inner.clock.now_ms();
        let fast = self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return Ok(QuerySnapshot {
                    data: None,
                    error: None,
                    is_fetching: false,
                    is_stale: true,
                    data_version: 0,
                });
            }
            let idx = handle.slot.index();
            let meta = s.metas[idx];
            let payload = &s.payloads[idx];

            if payload.committed.is_none()
                && let Some(raw) = payload.raw_response.as_ref()
            {
                return Err(raw.clone());
            }

            let data = payload
                .visible_data()
                .and_then(downcast_shared::<P::Output>);
            let error = payload
                .committed_error
                .as_ref()
                .and_then(downcast_shared::<RpcError<P::Error>>);
            Ok(QuerySnapshot {
                data,
                error,
                is_fetching: meta.is_fetching(),
                is_stale: meta.is_stale(now_ms, s.wildcard_epoch),
                data_version: meta.data_version,
            })
        });

        match fast {
            Ok(snap) => snap,
            Err(raw_bytes) => {
                let decoded: Result<P::Output, CodecError> =
                    self.inner.codec.decode_bounded(&raw_bytes);
                self.inner.state.with(|s| {
                    if !s.is_live(handle) {
                        return QuerySnapshot {
                            data: None,
                            error: None,
                            is_fetching: false,
                            is_stale: true,
                            data_version: 0,
                        };
                    }
                    let idx = handle.slot.index();
                    match decoded {
                        Ok(val) => {
                            let shared = into_shared_any(val);
                            let payload = &mut s.payloads[idx];
                            if payload.committed.is_none() {
                                payload.committed = Some(shared);
                                crate::optimistic::recompute_effective(payload);
                            }
                            s.metas[idx].flags |= FLAG_HAS_DATA;
                        }
                        Err(err) => {
                            let payload = &mut s.payloads[idx];
                            payload.raw_response = None;
                            payload.committed_error =
                                Some(into_shared_any(RpcError::<P::Error>::Codec(err)));
                            s.metas[idx].flags |= FLAG_HAS_ERROR;
                            s.metas[idx].flags &= !FLAG_HAS_DATA;
                        }
                    }
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    QuerySnapshot {
                        data: payload
                            .visible_data()
                            .and_then(downcast_shared::<P::Output>),
                        error: payload
                            .committed_error
                            .as_ref()
                            .and_then(downcast_shared::<RpcError<P::Error>>),
                        is_fetching: meta.is_fetching(),
                        is_stale: meta.is_stale(now_ms, s.wildcard_epoch),
                        data_version: meta.data_version,
                    }
                })
            }
        }
    }

    /// Returns `true` when `handle` should initiate a fetch in the current [`QueryMode`].
    pub fn needs_fetch(&self, handle: QueryHandle) -> bool {
        if self.inner.config.mode == QueryMode::SsrReadOnly {
            return false;
        }
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return false;
            }
            let meta = s.metas[handle.slot.index()];
            !meta.is_fetching() && meta.is_stale(now_ms, s.wildcard_epoch)
        })
    }

    /// Directly writes confirmed data into the cache for `(P, args)`, bumping `fetch_gen` so
    /// slower in-flight fetches cannot overwrite it, and rebasing any live optimistic layers.
    pub fn set_query_data<P>(
        &self,
        args: &P::Args,
        value: P::Output,
    ) -> Result<QueryHandle, CodecError>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Args> + RpcCodec<P::Output>,
    {
        let encoded_args = encode_args::<P, C>(&self.inner.codec, args)?;
        let mut raw_buf = Vec::with_capacity(64);
        self.inner.codec.encode_into(&value, &mut raw_buf)?;
        let raw_body = Bytes::from(raw_buf);
        let raw_hash = xxh64(&raw_body, BODY_HASH_SEED);
        let key_hash = query_key_hash(P::ID, &encoded_args);
        let now_ms = self.inner.clock.now_ms();
        let mut sink = TagSink::new();
        P::tags(args, &mut sink);

        let (handle, wake) = self.inner.state.with(|s| {
            let handle = if let Some(slot) = s.lookup_slot(P::ID, key_hash, &encoded_args) {
                QueryHandle {
                    slot,
                    generation: s.metas[slot.index()].slot_gen,
                }
            } else {
                s.insert_slot(
                    P::ID,
                    P::NAME,
                    key_hash,
                    encoded_args,
                    self.inner.config.default_options,
                    sink.as_slice(),
                    sink.is_overflow_all(),
                    now_ms,
                )
            };
            let idx = handle.slot.index();
            let meta = &mut s.metas[idx];
            meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
            meta.updated_at_ms = now_ms;
            meta.invalidated_epoch = s.wildcard_epoch;
            meta.flags |= FLAG_HAS_DATA;
            meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING | FLAG_HYDRATED);

            let payload = &mut s.payloads[idx];
            if let Some(abort) = payload.in_flight_abort.take() {
                abort();
            }
            payload.raw_response = Some(raw_body);
            payload.raw_body_hash = raw_hash;
            payload.committed = Some(into_shared_any(value));
            payload.committed_error = None;
            crate::optimistic::recompute_effective(payload);

            let wake = s.mark_dirty(handle.slot);
            (handle, wake)
        });
        if let Some(cb) = wake {
            cb();
        }
        Ok(handle)
    }

    /// Sans-I/O fetch start: transitions `handle` to `IS_FETCHING` and returns its deduplicated
    /// `(FetchTicket, RawCall)`, or `None` if already fetching (dedup!) or invalid.
    pub fn begin_fetch(&self, handle: QueryHandle) -> Option<(FetchTicket, RawCall)>
    where
        C: nve_rpc::CodecFormat,
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
            meta.flags |= FLAG_IS_FETCHING;
            let ticket = FetchTicket {
                handle,
                fetch_gen: meta.fetch_gen,
            };
            let payload = &s.payloads[idx];
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

    /// Sans-I/O fetch completion: commits `result` if `ticket` is still current (`slot_gen` and
    /// `fetch_gen` match), performs structural-sharing equality check, retires settled optimistic
    /// layers, and notifies observers. Returns `false` if the reply was stale and discarded.
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
        let (accepted, wake) = self.inner.state.with(|s| {
            if !s.is_live(ticket.handle) {
                return (false, None);
            }
            let idx = ticket.handle.slot.index();
            let meta = &mut s.metas[idx];
            if meta.fetch_gen != ticket.fetch_gen {
                return (false, None);
            }
            meta.flags &= !(FLAG_IS_FETCHING | FLAG_HYDRATED);
            let payload = &mut s.payloads[idx];
            payload.in_flight_abort = None;

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
                    meta.invalidated_epoch = s.wildcard_epoch;
                    meta.flags |= FLAG_HAS_DATA;
                    meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED);

                    if !same_bytes || has_layers {
                        payload.raw_response = Some(reply.raw_body);
                        payload.raw_body_hash = body_hash;
                        payload.committed = Some(into_shared_any(reply.value));
                        payload.committed_error = None;
                        settle_layers_up_to(payload, ticket.fetch_gen);
                    }
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake)
                }
                Err(ref err)
                    if matches!(
                        err.as_ref(),
                        RpcError::Transport(terr)
                            if terr.kind == nve_rpc::TransportErrorKind::Cancelled
                    ) =>
                {
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake)
                }
                Err(shared_err) => {
                    meta.flags |= FLAG_HAS_ERROR;
                    payload.committed_error = Some(shared_err);
                    let wake = s.mark_dirty(ticket.handle.slot);
                    (true, wake)
                }
            }
        });
        if let Some(cb) = wake {
            cb();
        }
        accepted
    }

}

#[inline]
pub(crate) fn query_key_hash(proc_id: ProcId, encoded_args: &[u8]) -> u64 {
    xxh64(encoded_args, proc_id.raw() ^ QUERY_ARGS_SEED)
}
