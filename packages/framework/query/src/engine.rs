//! Sans-I/O data-oriented [`QueryClient`] core: subscriptions, typed zero-clone reads,
//! deduplicated fetch lifecycle, stale-response generation guards, and prefetching.

use bytes::Bytes;
use nve_hash::xxh64;
use nve_rpc::{
    encode_args, CborCodec, CodecError, MaybeSend, MaybeSync, ProcId, Procedure, Query, RpcCodec,
    RpcError, TagSink, Transport,
};

use core::task::Waker;

use crate::optimistic::{prepare_recompute_job, settle_layers_up_to};
use crate::slab::{
    QuerySlotId, FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IN_DIRTY_QUEUE,
    FLAG_IS_FETCHING, FLAG_IS_INVALIDATED, FLAG_REFETCH_ON_FOCUS, FLAG_REFETCH_ON_RECONNECT,
    FLAG_SUBSCRIBED_ALL, NONE_IDX,
};
use crate::state::CacheState;
#[cfg(not(target_arch = "wasm32"))]
use crate::types::SystemClock;
use crate::types::{
    downcast_shared, into_shared_any, Clock, QueryClientConfig, QueryHandle, QueryMode,
    QueryOptions, QuerySnapshot, Shared,
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

impl<T, C> PartialEq for QueryClient<T, C> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Shared::ptr_eq(&self.inner, &other.inner)
    }
}

impl<T, C> Eq for QueryClient<T, C> {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Transport> QueryClient<T, CborCodec> {
    /// Creates a client in interactive [`QueryMode::Client`] using [`CborCodec::default`] and
    /// [`SystemClock::new`]. On `wasm32`, construct with [`Self::with_clock`] passing a real
    /// browser clock (e.g. `nve_http_web::BrowserClock`).
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

    /// Registers a host wake callback invoked whenever one or more cache slots change state,
    /// returning a monotonic token for [`Self::clear_wake_callback`].
    pub fn set_wake_callback(&self, cb: WakeCallback) -> u64 {
        self.inner.state.with(|s| {
            s.wake_cb_id = s.wake_cb_id.wrapping_add(1).max(1);
            s.wake_cb = Some(cb);
            s.wake_cb_id
        })
    }

    /// Detaches the wake callback registered under `token` (no-op if another callback has since
    /// replaced it), preventing unmounted providers from retaining dead UI wakers.
    pub fn clear_wake_callback(&self, token: u64) {
        self.inner.state.with(|s| {
            if s.wake_cb_id == token {
                s.wake_cb = None;
            }
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

        let (handle, waiters) = self.inner.state.with(|s| {
            let mut waiters = Vec::new();
            let handle = if let Some(slot) = s.lookup_slot(P::ID, key_hash, &encoded_args) {
                let idx = slot.index();
                let meta = &mut s.metas[idx];
                if meta.subscriber_count == 0 {
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
                    s.payloads[idx].options = options;
                } else {
                    // Merge shared-slot options conservatively across concurrent subscribers.
                    meta.stale_time_ms = meta.stale_time_ms.min(options.stale_time_ms);
                    meta.gc_time_ms = meta.gc_time_ms.max(options.gc_time_ms);
                    if options.refetch_on_focus {
                        meta.flags |= FLAG_REFETCH_ON_FOCUS;
                    }
                    if options.refetch_on_reconnect {
                        meta.flags |= FLAG_REFETCH_ON_RECONNECT;
                    }
                }
                s.payloads[idx].proc_name = P::NAME;
                if !sink.is_overflow_all() && !sink.is_empty() {
                    let mut first_edge = s.metas[idx].first_tag_edge;
                    s.tags.attach_tags(slot, &mut first_edge, sink.as_slice());
                    s.metas[idx].first_tag_edge = first_edge;
                } else if sink.is_overflow_all()
                    && (s.metas[idx].flags & FLAG_SUBSCRIBED_ALL) == 0
                {
                    s.metas[idx].flags |= FLAG_SUBSCRIBED_ALL;
                    s.subscribed_all_count += 1;
                }
                QueryHandle {
                    slot,
                    generation: s.metas[idx].slot_gen,
                }
            } else {
                let target_cap = self.inner.config.max_entries.saturating_sub(1);
                s.gc_sweep(now_ms, 16, target_cap, &mut waiters);
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
            meta.subscriber_count = meta
                .subscriber_count
                .checked_add(1)
                .expect("query subscriber_count overflowed u32");
            if was_zero {
                let pos = s.active_slots.len() as u32;
                s.active_slots.push(handle.slot);
                s.payloads[idx].active_pos = pos;
            }
            (handle, waiters)
        });
        run_side_effects(None, waiters);
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
    /// lazily decodes `raw_response` once for hydrated entries outside the cache lock).
    pub fn read<P>(&self, handle: QueryHandle) -> QuerySnapshot<P::Output, P::Error>
    where
        P: Procedure<Effect = Query>,
        C: RpcCodec<P::Output>,
    {
        let now_ms = self.inner.clock.now_ms();
        let grace = self.inner.config.hydration_grace_ms;
        let fast = self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return Ok(QuerySnapshot {
                    data: None,
                    error: None,
                    is_fetching: false,
                    is_stale: true,
                    is_invalidated: false,
                    data_version: 0,
                    invalidation_version: 0,
                });
            }
            let idx = handle.slot.index();
            let meta = s.metas[idx];
            if meta.proc_id != P::ID {
                return Ok(QuerySnapshot {
                    data: None,
                    error: None,
                    is_fetching: false,
                    is_stale: true,
                    is_invalidated: false,
                    data_version: 0,
                    invalidation_version: 0,
                });
            }
            let payload = &s.payloads[idx];

            if payload.committed.is_none()
                && let Some(raw) = payload.raw_response.as_ref()
            {
                return Err((raw.clone(), payload.raw_body_hash));
            }

            Ok(QuerySnapshot {
                data: payload
                    .visible_data()
                    .and_then(downcast_shared::<P::Output>),
                error: payload
                    .committed_error
                    .as_ref()
                    .and_then(downcast_shared::<RpcError<P::Error>>),
                is_fetching: meta.is_fetching(),
                is_stale: meta.is_stale(now_ms, s.wildcard_epoch, grace),
                is_invalidated: meta.is_invalidated(s.wildcard_epoch),
                data_version: meta.data_version,
                invalidation_version: ((u64::from(s.wildcard_epoch)) << 32)
                    | u64::from(meta.fetch_gen),
            })
        });

        match fast {
            Ok(snap) => snap,
            Err((raw_bytes, expected_hash)) => {
                let decoded: Result<P::Output, CodecError> =
                    self.inner.codec.decode_bounded(&raw_bytes);
                let job = self.inner.state.with(|s| {
                    if !s.is_live(handle) {
                        return None;
                    }
                    let idx = handle.slot.index();
                    if s.metas[idx].proc_id != P::ID {
                        return None;
                    }
                    match decoded {
                        Ok(val) => {
                            let payload = &mut s.payloads[idx];
                            if payload.committed.is_none() && payload.raw_body_hash == expected_hash
                            {
                                payload.committed = Some(into_shared_any(val));
                                s.metas[idx].flags |= FLAG_HAS_DATA;
                                return prepare_recompute_job(
                                    handle.slot,
                                    &mut s.metas[idx],
                                    &mut s.payloads[idx],
                                );
                            }
                        }
                        Err(err) => {
                            let payload = &mut s.payloads[idx];
                            if payload.committed.is_none() && payload.raw_body_hash == expected_hash
                            {
                                payload.raw_response = None;
                                payload.committed_error =
                                    Some(into_shared_any(RpcError::<P::Error>::Codec(err)));
                                s.metas[idx].flags |= FLAG_HAS_ERROR;
                                s.metas[idx].flags &= !FLAG_HAS_DATA;
                            }
                        }
                    }
                    None
                });
                if let Some(job) = job {
                    let _ = self.recompute_jobs_outside_lock(vec![job]);
                }
                self.inner.state.with(|s| {
                    if !s.is_live(handle) {
                        return QuerySnapshot {
                            data: None,
                            error: None,
                            is_fetching: false,
                            is_stale: true,
                            is_invalidated: false,
                            data_version: 0,
                            invalidation_version: 0,
                        };
                    }
                    let idx = handle.slot.index();
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
                        is_stale: meta.is_stale(now_ms, s.wildcard_epoch, grace),
                        is_invalidated: meta.is_invalidated(s.wildcard_epoch),
                        data_version: meta.data_version,
                        invalidation_version: ((u64::from(s.wildcard_epoch)) << 32)
                            | u64::from(meta.fetch_gen),
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
        let grace = self.inner.config.hydration_grace_ms;
        self.inner.state.with(|s| {
            if !s.is_live(handle) {
                return false;
            }
            let meta = s.metas[handle.slot.index()];
            !meta.is_fetching() && meta.is_stale(now_ms, s.wildcard_epoch, grace)
        })
    }

    /// Directly writes confirmed data into the cache for `(P, args)`, bumping `fetch_gen` so
    /// slower in-flight fetches cannot overwrite it, and rebasing any live optimistic layers
    /// strictly outside the cache lock.
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

        let (handle, wake, waiters, job) = self.inner.state.with(|s| {
            let mut waiters = Vec::new();
            let handle = if let Some(slot) = s.lookup_slot(P::ID, key_hash, &encoded_args) {
                let idx = slot.index();
                s.payloads[idx].proc_name = P::NAME;
                QueryHandle {
                    slot,
                    generation: s.metas[idx].slot_gen,
                }
            } else {
                let target_cap = self.inner.config.max_entries.saturating_sub(1);
                s.gc_sweep(now_ms, 16, target_cap, &mut waiters);
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
            let new_fetch_gen = meta.fetch_gen;
            meta.updated_at_ms = now_ms;
            meta.invalidated_epoch = s.wildcard_epoch;
            meta.flags |= FLAG_HAS_DATA;
            meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING | FLAG_HYDRATED);

            let payload = &mut s.payloads[idx];
            waiters.append(&mut payload.in_flight_waiters);
            payload.raw_response = Some(raw_body);
            payload.raw_body_hash = raw_hash;
            payload.committed = Some(into_shared_any(value));
            payload.committed_error = None;
            settle_layers_up_to(payload, new_fetch_gen);
            let job = prepare_recompute_job(handle.slot, &mut s.metas[idx], &mut s.payloads[idx]);
            let wake = s.mark_dirty(handle.slot);
            (handle, wake, waiters, job)
        });
        let recompute_wake = job.and_then(|j| self.recompute_jobs_outside_lock(vec![j]));
        run_side_effects(recompute_wake.or(wake), waiters);
        Ok(handle)
    }

}

pub(crate) fn run_side_effects(wake: Option<WakeCallback>, waiters: Vec<Waker>) {
    for waker in waiters {
        waker.wake();
    }
    if let Some(cb) = wake {
        cb();
    }
}

#[inline]
pub(crate) fn query_key_hash(proc_id: ProcId, encoded_args: &[u8]) -> u64 {
    xxh64(encoded_args, proc_id.raw() ^ QUERY_ARGS_SEED)
}
