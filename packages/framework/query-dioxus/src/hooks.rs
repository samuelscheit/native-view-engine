//! Ergonomic `Copy` Dioxus 0.7 hooks (`use_query`, `use_query_with`, `use_action`,
//! `use_query_client`) with zero-leak lifecycle cleanup and rebaseable optimistic mutations.

use core::marker::PhantomData;

use bytes::Bytes;
use dioxus_core::{use_drop, use_hook};
use dioxus_signals::{CopyValue, ReadableExt, Signal, WritableExt};
use nve_query::{
    OptimisticHandle, OptimisticTx, QueryHandle, QueryMode, QueryOptions, QuerySnapshot, Shared,
};
use nve_rpc::{
    encode_args, Action, CallReply, CborCodec, Procedure, Query, RpcCodec, RpcError,
};

use crate::bridge::{consume_bridge, DioxusQueryBridge, ErasedQueryClient};

/// Returns the ambient [`ErasedQueryClient`] from the nearest [`crate::QueryClientProvider`].
pub fn use_query_client() -> ErasedQueryClient {
    consume_bridge().client().clone()
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
        if bridge.client().mode() == QueryMode::SsrReadOnly {
            return;
        }
        let inv_ver = bridge.client().read::<P>(handle).invalidation_version;
        spawn_query_fetch::<P>(bridge, handle, inv_ver);
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
        })
    });

    // Register cleanup once per hook instance.
    use_drop(move || {
        let (bridge, handle) = {
            let st = state.read();
            (st.bridge.clone(), st.handle)
        };
        bridge.client().unsubscribe(handle);
        bridge.release_slot_signal(handle.slot);
    });

    // Handle key switches cleanly without leaking the previous slot or signal.
    let key_changed = state.read().encoded_args != encoded_args;
    if key_changed {
        let (old_handle, old_data) = {
            let st = state.read();
            let prev = if options.keep_previous_data {
                st.bridge
                    .client()
                    .read::<P>(st.handle)
                    .data
                    .or_else(|| st.previous_data.as_ref().map(Shared::clone))
            } else {
                None
            };
            (st.handle, prev)
        };
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

struct ActionUiState<O, E> {
    pending_count: u32,
    data: Option<Shared<O>>,
    error: Option<Shared<RpcError<E>>>,
}

impl<O, E> Default for ActionUiState<O, E> {
    fn default() -> Self {
        Self {
            pending_count: 0,
            data: None,
            error: None,
        }
    }
}

pub(crate) struct PendingGuard<O: 'static, E: 'static> {
    sig: Option<Signal<ActionUiState<O, E>>>,
}

impl<O: 'static, E: 'static> Drop for PendingGuard<O, E> {
    fn drop(&mut self) {
        if let Some(mut sig) = self.sig.take()
            && let Ok(mut w) = sig.try_write()
        {
            w.pending_count = w.pending_count.saturating_sub(1);
        }
    }
}

struct OptimisticDropGuard {
    client: ErasedQueryClient,
    handle: Option<OptimisticHandle>,
}

impl Drop for OptimisticDropGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take()
            && handle.touched_count() > 0
        {
            self.client.rollback_optimistic(&handle);
        }
    }
}

/// `Copy` reactive mutation handle returned by [`use_action`].
pub struct UseAction<P: Procedure<Effect = Action>> {
    bridge: CopyValue<DioxusQueryBridge>,
    ui: Signal<ActionUiState<P::Output, P::Error>>,
    _marker: PhantomData<fn() -> P>,
}

impl<P: Procedure<Effect = Action>> Clone for UseAction<P> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: Procedure<Effect = Action>> Copy for UseAction<P> {}

impl<P> UseAction<P>
where
    P: Procedure<Effect = Action>,
    CborCodec: RpcCodec<P::Args> + RpcCodec<P::Output> + RpcCodec<P::Error>,
{
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.ui.read().pending_count > 0
    }

    #[inline]
    pub fn data(&self) -> Option<Shared<P::Output>> {
        self.ui.read().data.as_ref().map(Shared::clone)
    }

    #[inline]
    pub fn error(&self) -> Option<Shared<RpcError<P::Error>>> {
        self.ui.read().error.as_ref().map(Shared::clone)
    }

    pub fn reset(&self) {
        let mut ui = self.ui;
        let mut w = ui.write();
        w.data = None;
        w.error = None;
    }

    /// Awaits action `P` directly (useful inside async handlers and [`crate::ActionForm`]).
    pub async fn call(
        &self,
        args: P::Args,
    ) -> Result<CallReply<P::Output>, Shared<RpcError<P::Error>>>
    where
        P::Output: Clone,
    {
        self.call_optimistic(args, |_| {}).await
    }

    /// Applies `optimistic` patches synchronously BEFORE awaiting the network call, then settles
    /// atomically on `Ok` or rolls back on `Err` (and automatically on future drop).
    pub async fn call_optimistic<Opt>(
        &self,
        args: P::Args,
        optimistic: Opt,
    ) -> Result<CallReply<P::Output>, Shared<RpcError<P::Error>>>
    where
        P::Output: Clone,
        Opt: FnOnce(&mut OptimisticTx<'_, CborCodec>),
    {
        let client = self.bridge.read().client().clone();
        let mut ui = self.ui;
        {
            let mut w = ui.write();
            w.pending_count = w.pending_count.saturating_add(1);
            w.error = None;
        }
        let _pending_guard = PendingGuard { sig: Some(ui) };

        match client.execute_action_optimistic::<P, _>(&args, optimistic).await {
            Ok((reply, _refetch)) => {
                if let Ok(mut w) = ui.try_write() {
                    w.data = Some(Shared::new(reply.value.clone()));
                    w.error = None;
                }
                Ok(reply)
            }
            Err(err) => {
                let shared = Shared::new(err);
                if let Ok(mut w) = ui.try_write() {
                    w.error = Some(Shared::clone(&shared));
                }
                Err(shared)
            }
        }
    }

    /// Synchronously resolves the provider-owned [`ErasedQueryClient`], increments `pending_count`,
    /// and returns an RAII guard BEFORE spawning so same-frame component unmount never panics on
    /// `self.bridge.read()` and double submits in [`crate::ActionForm`] are blocked immediately.
    pub(crate) fn begin_pending_guard(
        &self,
    ) -> (ErasedQueryClient, PendingGuard<P::Output, P::Error>) {
        let client = self.bridge.read().client().clone();
        let mut ui = self.ui;
        {
            let mut w = ui.write();
            w.pending_count = w.pending_count.saturating_add(1);
            w.error = None;
        }
        (client, PendingGuard { sig: Some(ui) })
    }

    pub(crate) async fn call_with_pending_guard(
        &self,
        client: ErasedQueryClient,
        args: P::Args,
        _pending_guard: PendingGuard<P::Output, P::Error>,
    ) -> Result<CallReply<P::Output>, Shared<RpcError<P::Error>>>
    where
        P::Output: Clone,
    {
        let mut ui = self.ui;
        match client.execute_action::<P>(&args).await {
            Ok((reply, _refetch)) => {
                if let Ok(mut w) = ui.try_write() {
                    w.data = Some(Shared::new(reply.value.clone()));
                    w.error = None;
                }
                Ok(reply)
            }
            Err(err) => {
                let shared = Shared::new(err);
                if let Ok(mut w) = ui.try_write() {
                    w.error = Some(Shared::clone(&shared));
                }
                Err(shared)
            }
        }
    }

    pub(crate) fn spawn_in_provider(&self, fut: impl core::future::Future<Output = ()> + 'static) {
        let _ = self.bridge.read().spawn_in_provider(fut);
    }

    /// Fire-and-forget action trigger spawning inside [`crate::QueryClientProvider`]'s scope so
    /// unmounting the triggering row/modal never cancels or panics the in-flight mutation.
    pub fn run(&self, args: P::Args)
    where
        P::Output: Clone,
    {
        let this = *self;
        let (client, pending_guard) = self.begin_pending_guard();
        self.spawn_in_provider(async move {
            let _ = this
                .call_with_pending_guard(client, args, pending_guard)
                .await;
        });
    }

    /// Fire-and-forget optimistic action trigger: applies optimistic patches synchronously on
    /// the current click frame BEFORE spawning the async RPC call inside
    /// [`crate::QueryClientProvider`]'s scope, with pre-spawn RAII guards guaranteeing cleanup
    /// even if the provider unmounts before the spawned task's first poll.
    pub fn run_optimistic<Opt>(&self, args: P::Args, optimistic: Opt)
    where
        P::Output: Clone,
        Opt: FnOnce(&mut OptimisticTx<'_, CborCodec>),
    {
        let bridge = self.bridge.read().clone();
        let (client, pending_guard) = self.begin_pending_guard();
        let opt_handle = client.apply_optimistic(optimistic);
        let mut opt_guard = OptimisticDropGuard {
            client: client.clone(),
            handle: Some(opt_handle),
        };
        let mut ui = self.ui;
        let _ = bridge.spawn_in_provider(async move {
            let _pending_guard = pending_guard;
            let Some(handle) = opt_guard.handle.take() else {
                return;
            };
            match client.execute_action_with_handle::<P>(&args, handle).await {
                Ok((reply, _refetch)) => {
                    if let Ok(mut w) = ui.try_write() {
                        w.data = Some(Shared::new(reply.value));
                        w.error = None;
                    }
                }
                Err(err) => {
                    if let Ok(mut w) = ui.try_write() {
                        w.error = Some(Shared::new(err));
                    }
                }
            }
        });
    }
}

/// Creates a `Copy` reactive handle for invoking RPC [`Action`] procedure `P`.
pub fn use_action<P>() -> UseAction<P>
where
    P: Procedure<Effect = Action>,
{
    let bridge = use_hook(|| CopyValue::new(consume_bridge()));
    let ui = use_hook(|| Signal::new(ActionUiState::default()));
    UseAction {
        bridge,
        ui,
        _marker: PhantomData,
    }
}
