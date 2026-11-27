//! `Copy` reactive mutation hook ([`use_action`] / [`UseAction`]) with rebaseable optimistic
//! updates and provider-scoped fire-and-forget execution.

use core::marker::PhantomData;

use dioxus_core::use_hook;
use dioxus_signals::{CopyValue, ReadableExt, Signal, WritableExt};
use nve_query::{OptimisticHandle, OptimisticTx, Shared};
use nve_rpc::{Action, CallReply, CborCodec, Procedure, RpcCodec, RpcError};

use crate::bridge::{consume_bridge, DioxusQueryBridge, ErasedQueryClient};

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

        match client
            .execute_action_optimistic::<P, _>(&args, optimistic)
            .await
        {
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
