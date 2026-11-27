//! Ergonomic `Copy` Dioxus 0.7 query hooks (`use_query`, `use_query_with`, `use_query_client`,
//! `UseQuery::suspend`, `UseQuery::suspend_result`) with zero-leak lifecycle cleanup and per-hook
//! suspense waiters safe across sibling `<QuerySuspense>` boundaries.

use core::cell::Cell;
use core::marker::PhantomData;
use std::rc::Rc;

use bytes::Bytes;
use dioxus_core::{
    current_scope_id, needs_update_any, spawn, try_consume_context, use_drop, use_hook,
    CapturedError, RenderError, SuspendedFuture, Task,
};
use dioxus_signals::{CopyValue, ReadableExt, Signal, WritableExt};
use nve_query::{QueryHandle, QueryMode, QueryOptions, QuerySnapshot, Shared};
use nve_rpc::{encode_args, CborCodec, Procedure, Query, RpcCodec, RpcError};

use crate::bridge::{consume_bridge, DioxusQueryBridge, ErasedQueryClient};
use crate::suspense::BoundaryQuerySet;

/// Settled typed outcome returned inside `Ok(...)` by [`UseQuery::suspend_result`].
pub type QueryOutcome<T, E> = Result<Shared<T>, Shared<RpcError<E>>>;

/// Returns the ambient [`ErasedQueryClient`] from the nearest [`crate::QueryClientProvider`].
pub fn use_query_client() -> ErasedQueryClient {
    consume_bridge().client().clone()
}

struct SuspendWaiter {
    handle: QueryHandle,
    inv_ver: u64,
    task: Task,
    alive: Rc<Cell<bool>>,
}

struct WaiterAliveGuard(Rc<Cell<bool>>);

impl Drop for WaiterAliveGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

struct QueryHookState<T> {
    bridge: DioxusQueryBridge,
    handle: QueryHandle,
    slot_signal: Signal<u32>,
    encoded_args: Bytes,
    options: QueryOptions,
    last_invalidation_ver: u64,
    fetched_for_key: bool,
    external_wait_seq: Option<u32>,
    previous_data: Option<Shared<T>>,
    suspend_waiter: Option<SuspendWaiter>,
}

/// `Copy` reactive handle returned by [`use_query`] / [`use_query_with`].
pub struct UseQuery<P: Procedure<Effect = Query>> {
    state: CopyValue<QueryHookState<P::Output>>,
    _marker: PhantomData<fn() -> P>,
}

impl<P: Procedure<Effect = Query>> Clone for UseQuery<P> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: Procedure<Effect = Query>> Copy for UseQuery<P> {}

impl<P> UseQuery<P>
where
    P: Procedure<Effect = Query>,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    fn cancel_suspend_waiter(&self) {
        let mut state = self.state;
        let old = state.write().suspend_waiter.take();
        if let Some(waiter) = old
            && waiter.alive.get()
        {
            waiter.task.cancel();
        }
    }

    /// Reads a typed [`QuerySnapshot`] and subscribes the calling component to this query slot's
    /// dense version `Signal<u32>`.
    pub fn snapshot(&self) -> QuerySnapshot<P::Output, P::Error> {
        let (bridge, handle, slot_sig) = {
            let st = self.state.read();
            (st.bridge.clone(), st.handle, st.slot_signal)
        };
        let _ver = *slot_sig.read();
        bridge.client().read::<P>(handle)
    }

    /// Returns cached (or optimistic) data, falling back to the previous key's data when
    /// `QueryOptions::keep_previous_data(true)` is enabled and the new key is still loading.
    pub fn data(&self) -> Option<Shared<P::Output>> {
        let snap = self.snapshot();
        if snap.data.is_some() {
            return snap.data;
        }
        let st = self.state.read();
        if st.options.keep_previous_data {
            st.previous_data.as_ref().map(Shared::clone)
        } else {
            None
        }
    }

    /// Returns the last committed RPC/transport error, if any.
    pub fn error(&self) -> Option<Shared<RpcError<P::Error>>> {
        self.snapshot().error
    }

    /// Returns `true` during initial load before any data or error has arrived.
    pub fn is_loading(&self) -> bool {
        let snap = self.snapshot();
        snap.data.is_none() && snap.is_fetching
    }

    /// Returns `true` whenever a network request is currently in flight (initial or background).
    pub fn is_fetching(&self) -> bool {
        self.snapshot().is_fetching
    }

    /// Returns `true` when cached data is stale or invalidated.
    pub fn is_stale(&self) -> bool {
        self.snapshot().is_stale
    }

    /// Returns `true` when [`Self::data`] is currently displaying `keep_previous_data` placeholder
    /// content from the preceding key while the new key fetches.
    pub fn is_placeholder_data(&self) -> bool {
        let snap = self.snapshot();
        if snap.data.is_some() {
            return false;
        }
        let st = self.state.read();
        st.options.keep_previous_data && st.previous_data.is_some()
    }

    /// Forces a fresh background fetch for the current key regardless of `stale_time_ms`.
    pub fn refetch(&self) {
        let (bridge, handle) = {
            let st = self.state.read();
            (st.bridge.clone(), st.handle)
        };
        if bridge.client().mode() == QueryMode::Ssr {
            return;
        }
        let inv_ver = bridge.client().read::<P>(handle).invalidation_version;
        spawn_query_fetch::<P>(bridge, handle, inv_ver);
    }

    /// Suspends the calling component inside the nearest [`crate::QuerySuspense`] (or Dioxus
    /// `SuspenseBoundary`) until data resolves, surfacing settled errors into the nearest
    /// `ErrorBoundary` via [`RenderError::Error`].
    pub fn suspend(&self) -> Result<Shared<P::Output>, RenderError>
    where
        P::Error: core::fmt::Debug,
    {
        match self.suspend_result()? {
            Ok(data) => Ok(data),
            Err(err) => Err(RenderError::Error(CapturedError::from_display(err))),
        }
    }

    /// Like [`Self::suspend`], but returns settled query errors as `Ok(Err(err))` instead of
    /// throwing into an enclosing `ErrorBoundary` so components can render inline error UI.
    pub fn suspend_result(&self) -> Result<QueryOutcome<P::Output, P::Error>, RenderError> {
        let (bridge, handle, slot_sig, opts) = {
            let st = self.state.read();
            (st.bridge.clone(), st.handle, st.slot_signal, st.options)
        };

        // Always attribute `handle` to the nearest `<QuerySuspense>` boundary (if mounted inside
        // one) so SSR streaming dehydrates ONLY that boundary's handles.
        if let Some(boundary_set) = try_consume_context::<BoundaryQuerySet>() {
            boundary_set.record(handle);
        }

        let _ver = *slot_sig.read();
        let snap = bridge.client().read::<P>(handle);

        if let Some(data) = snap.data {
            self.cancel_suspend_waiter();
            return Ok(Ok(data));
        }

        let mode = bridge.client().mode();
        let inv_ver = snap.invalidation_version;
        // A client fetch still in flight or a slot task still retrying (backoff sleep between
        // attempts) means the committed error is transient: keep suspending until the retry
        // loop succeeds or exhausts instead of tripping the enclosing `ErrorBoundary` early.
        let retry_pending = mode == QueryMode::Client
            && (snap.is_fetching || bridge.has_active_slot_task_for(handle, inv_ver));
        let has_current_error = snap.error.is_some()
            && !retry_pending
            && (!snap.is_invalidated
                || snap.completed_invalidation_version == snap.invalidation_version);

        if mode == QueryMode::Client
            && opts.keep_previous_data
            && (!has_current_error || snap.is_fetching)
        {
            let prev = {
                let st = self.state.read();
                st.previous_data.as_ref().map(Shared::clone)
            };
            if let Some(prev) = prev {
                self.cancel_suspend_waiter();
                return Ok(Ok(prev));
            }
        }

        if has_current_error && let Some(err) = snap.error {
            self.cancel_suspend_waiter();
            return Ok(Err(err));
        }

        // Pending / fetching / `is_stream_pending` / retrying after invalidation.
        if mode == QueryMode::Client
            && !snap.is_fetching
            && !snap.is_stream_pending
            && !bridge.has_active_slot_task_for(handle, inv_ver)
        {
            spawn_query_fetch::<P>(bridge.clone(), handle, inv_ver);
        }

        // Reuse an already-alive per-hook waiter task keyed by `(handle, inv_ver)` so re-renders
        // never spawn duplicate tasks AND never share a single `DioxusQueryBridge` slot task
        // across sibling `<QuerySuspense>` boundaries (which would clobber `LocalTask::ty`).
        let existing_task = {
            let st = self.state.read();
            st.suspend_waiter.as_ref().and_then(|w| {
                (w.handle == handle && w.inv_ver == inv_ver && w.alive.get()).then_some(w.task)
            })
        };
        if let Some(task) = existing_task {
            return Err(RenderError::Suspended(SuspendedFuture::new(task)));
        }

        self.cancel_suspend_waiter();
        let origin_scope = current_scope_id();
        let client = bridge.client().clone();
        let bridge_for_waiter = bridge.clone();
        let waiting_for_stream = snap.is_stream_pending;
        let alive = Rc::new(Cell::new(true));
        let alive_for_task = Rc::clone(&alive);

        let task = spawn(async move {
            let _guard = WaiterAliveGuard(alive_for_task);
            match mode {
                QueryMode::Ssr => {
                    let _ = client.fetch_handle::<P>(handle).await;
                }
                QueryMode::Client => {
                    if waiting_for_stream {
                        client.wait_for_settled(handle).await;
                    } else if bridge_for_waiter.has_active_slot_task_for(handle, inv_ver) {
                        bridge_for_waiter.wait_for_slot_task(handle, inv_ver).await;
                    } else {
                        client.wait_for_settled(handle).await;
                    }
                }
            }
            needs_update_any(origin_scope);
        });

        let mut state = self.state;
        state.write().suspend_waiter = Some(SuspendWaiter {
            handle,
            inv_ver,
            task,
            alive,
        });
        Err(RenderError::Suspended(SuspendedFuture::new(task)))
    }
}

fn spawn_query_fetch<P>(bridge: DioxusQueryBridge, handle: QueryHandle, inv_ver: u64)
where
    P: Procedure<Effect = Query>,
    CborCodec: RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    let client = bridge.client().clone();
    let bridge_for_task = bridge.clone();
    bridge.spawn_slot_fetch(handle, inv_ver, async move {
        let _ = client
            .fetch_handle_with_retry::<P, _, _>(handle, |ms| bridge_for_task.sleep_ms(ms))
            .await;
    });
}

/// Subscribes the calling component to query `(P, args)` using the client's default options.
pub fn use_query<P>(args: P::Args) -> UseQuery<P>
where
    P: Procedure<Effect = Query>,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    let opts = consume_bridge().client().config().default_options;
    use_query_with::<P>(args, opts)
}

/// Subscribes the calling component to query `(P, args)` with custom [`QueryOptions`].
pub fn use_query_with<P>(args: P::Args, options: QueryOptions) -> UseQuery<P>
where
    P: Procedure<Effect = Query>,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    let bridge = consume_bridge();
    let encoded_args = encode_args::<P, CborCodec>(bridge.client().codec(), &args)
        .expect("query arguments must encode cleanly with CborCodec");

    let mut state: CopyValue<QueryHookState<P::Output>> = use_hook(|| {
        let handle = bridge
            .client()
            .subscribe::<P>(&args, options)
            .expect("subscribe failed after encode_args succeeded");
        let snap = bridge.client().read::<P>(handle);
        let slot_signal = bridge.acquire_slot_signal(handle.slot, snap.data_version);
        CopyValue::new(QueryHookState {
            bridge: bridge.clone(),
            handle,
            slot_signal,
            encoded_args: encoded_args.clone(),
            options,
            last_invalidation_ver: snap.invalidation_version,
            fetched_for_key: false,
            external_wait_seq: None,
            previous_data: None,
            suspend_waiter: None,
        })
    });

    // Register cleanup once per hook instance.
    use_drop(move || {
        let (bridge, handle, waiter) = {
            let mut st = state.write();
            (st.bridge.clone(), st.handle, st.suspend_waiter.take())
        };
        if let Some(waiter) = waiter
            && waiter.alive.get()
        {
            waiter.task.cancel();
        }
        bridge.client().unsubscribe(handle);
        bridge.release_slot_signal(handle.slot);
    });

    // Handle key switches cleanly without leaking the previous slot, signal, or suspend waiter.
    let key_changed = state.read().encoded_args != encoded_args;
    if key_changed {
        let (old_handle, old_data, old_waiter) = {
            let mut st = state.write();
            let prev = if options.keep_previous_data {
                st.bridge
                    .client()
                    .read::<P>(st.handle)
                    .data
                    .or_else(|| st.previous_data.as_ref().map(Shared::clone))
            } else {
                None
            };
            (st.handle, prev, st.suspend_waiter.take())
        };
        if let Some(waiter) = old_waiter
            && waiter.alive.get()
        {
            waiter.task.cancel();
        }
        // Keep the enclosing boundary's attribution bounded to live keys only.
        if let Some(boundary_set) = try_consume_context::<BoundaryQuerySet>() {
            boundary_set.forget(old_handle);
        }
        let new_handle = bridge
            .client()
            .subscribe::<P>(&args, options)
            .expect("subscribe failed on key change");
        let new_snap = bridge.client().read::<P>(new_handle);
        let new_sig = bridge.acquire_slot_signal(new_handle.slot, new_snap.data_version);
        bridge.client().unsubscribe(old_handle);
        bridge.release_slot_signal(old_handle.slot);

        let mut st = state.write();
        st.handle = new_handle;
        st.slot_signal = new_sig;
        st.encoded_args = encoded_args;
        st.options = options;
        st.last_invalidation_ver = new_snap.invalidation_version;
        st.fetched_for_key = false;
        st.external_wait_seq = None;
        st.previous_data = old_data;
    } else if state.read().options != options {
        state.write().options = options;
    }

    // Subscribe the calling component to this slot's reactive signal so invalidations wake it.
    let (handle, slot_sig, mut fetched_for_key, last_inv_ver, external_wait_seq) = {
        let st = state.read();
        (
            st.handle,
            st.slot_signal,
            st.fetched_for_key,
            st.last_invalidation_ver,
            st.external_wait_seq,
        )
    };
    let _ = *slot_sig.read();

    if bridge.client().mode() == QueryMode::Client {
        let snap = bridge.client().read::<P>(handle);
        let active_for_ver = bridge.has_active_slot_task_for(handle, snap.invalidation_version);
        if let Some(wait_seq) = external_wait_seq
            && !snap.is_fetching
        {
            let mut st = state.write();
            st.external_wait_seq = None;
            // Latch `fetched_for_key` when the awaited external commit settles (data or error).
            // `should_refetch_invalidated` below guards against re-fetching an invalidation already
            // answered by an error commit via `completed_invalidation_version != invalidation_version`,
            // while any subsequent invalidation advances `invalidation_version` and still refetches.
            if snap.completed_seq != wait_seq {
                st.fetched_for_key = true;
                fetched_for_key = true;
            }
        }
        let should_initial = !fetched_for_key && !snap.is_fetching && snap.is_stale;
        let should_refetch_invalidated = snap.is_invalidated
            && !snap.is_fetching
            && last_inv_ver != snap.invalidation_version
            && snap.completed_invalidation_version != snap.invalidation_version;
        if should_initial || should_refetch_invalidated {
            {
                let mut st = state.write();
                st.fetched_for_key = true;
                st.last_invalidation_ver = snap.invalidation_version;
                st.external_wait_seq = None;
            }
            if !active_for_ver {
                spawn_query_fetch::<P>(bridge, handle, snap.invalidation_version);
            }
        } else if !fetched_for_key {
            let mut st = state.write();
            if !snap.is_fetching || active_for_ver {
                st.fetched_for_key = true;
                st.last_invalidation_ver = snap.invalidation_version;
                st.external_wait_seq = None;
            } else if st.external_wait_seq.is_none() {
                st.external_wait_seq = Some(snap.completed_seq);
            }
        } else if active_for_ver && last_inv_ver != snap.invalidation_version {
            state.write().last_invalidation_ver = snap.invalidation_version;
        }
    }

    UseQuery {
        state,
        _marker: PhantomData,
    }
}
