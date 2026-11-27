//! Transport-erased [`ErasedQueryClient`], lock-free cross-thread wake notifier, dense recycled
//! `QuerySlotId` -> `Signal<u32>` bridge, and [`QueryClientProvider`].

use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Poll, Waker};
use std::rc::Rc;

use dioxus_core::{current_scope_id, provide_context, spawn, try_consume_context, use_hook, Element, ScopeId};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_query::{Clock, QueryClient, QueryClientConfig, QuerySlotId, Shared};
use nve_rpc::{BoxFuture, CborCodec, RawCall, RawReply, Transport, TransportError};

/// Object-safe type-erased [`Transport`] wrapper so shared cross-platform Dioxus UI components
/// never thread `<T: Transport>` generics across Web (`FetchTransport`), Native (`HyperTransport`),
/// and SSR (`DirectTransport`).
#[derive(Clone)]
pub struct DynTransport(Shared<dyn Transport>);

impl DynTransport {
    #[inline]
    pub fn new<T: Transport>(transport: T) -> Self {
        Self(Shared::new(transport))
    }
}

impl Transport for DynTransport {
    #[inline]
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        self.0.call_raw(req)
    }
}

/// Concrete transport-erased [`QueryClient`] used inside [`QueryClientProvider`] and UI hooks.
pub type ErasedQueryClient = QueryClient<DynTransport, CborCodec>;

/// Creates an interactive [`ErasedQueryClient`] backed by any native [`Transport`] and
/// [`nve_query::SystemClock`].
#[cfg(not(target_arch = "wasm32"))]
pub fn erased_query_client<T: Transport>(transport: T) -> ErasedQueryClient {
    QueryClient::new(DynTransport::new(transport))
}

/// Creates a per-request SSR [`ErasedQueryClient`] in [`nve_query::QueryMode::SsrReadOnly`].
#[cfg(not(target_arch = "wasm32"))]
pub fn erased_ssr_query_client<T: Transport>(transport: T) -> ErasedQueryClient {
    QueryClient::ssr(DynTransport::new(transport))
}

/// Creates an [`ErasedQueryClient`] with an explicit [`Clock`] and [`QueryClientConfig`]
/// (required on `wasm32`, e.g. with `nve_http_web::BrowserClock`).
pub fn erased_query_client_with_clock<T: Transport>(
    transport: T,
    clock: Shared<dyn Clock>,
    config: QueryClientConfig,
) -> ErasedQueryClient {
    QueryClient::with_clock(DynTransport::new(transport), CborCodec::default(), clock, config)
}

#[cfg(not(target_arch = "wasm32"))]
mod wake_notifier {
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::task::{Context, Poll, Waker};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub(crate) struct WakeNotifier(Arc<Inner>);

    #[derive(Default)]
    struct Inner {
        notified: AtomicBool,
        waker: Mutex<Option<Waker>>,
    }

    impl WakeNotifier {
        pub(crate) fn notify(&self) {
            self.0.notified.store(true, Ordering::Release);
            let waker = self.0.waker.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(w) = waker {
                w.wake();
            }
        }

        pub(crate) fn poll_wait(&self, cx: &mut Context<'_>) -> Poll<()> {
            if self.0.notified.swap(false, Ordering::AcqRel) {
                return Poll::Ready(());
            }
            let mut slot = self.0.waker.lock().unwrap_or_else(|e| e.into_inner());
            if self.0.notified.swap(false, Ordering::AcqRel) {
                return Poll::Ready(());
            }
            *slot = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wake_notifier {
    use core::cell::{Cell, RefCell};
    use core::task::{Context, Poll, Waker};
    use std::rc::Rc;

    #[derive(Clone, Default)]
    pub(crate) struct WakeNotifier(Rc<Inner>);

    #[derive(Default)]
    struct Inner {
        notified: Cell<bool>,
        waker: RefCell<Option<Waker>>,
    }

    impl WakeNotifier {
        pub(crate) fn notify(&self) {
            self.0.notified.set(true);
            if let Some(w) = self.0.waker.borrow_mut().take() {
                w.wake();
            }
        }

        pub(crate) fn poll_wait(&self, cx: &mut Context<'_>) -> Poll<()> {
            if self.0.notified.replace(false) {
                return Poll::Ready(());
            }
            *self.0.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

use wake_notifier::WakeNotifier;

struct BridgeSlots {
    signals: Vec<Option<(Signal<u32>, u32)>>,
    dirty_buf: Vec<(QuerySlotId, u32)>,
}

/// Reactive UI-thread bridge connecting an [`ErasedQueryClient`] to dense recycled per-slot
/// `Signal<u32>`s owned by [`QueryClientProvider`]'s scope.
#[derive(Clone)]
pub struct DioxusQueryBridge {
    client: ErasedQueryClient,
    provider_scope: ScopeId,
    slots: Rc<RefCell<BridgeSlots>>,
}

impl PartialEq for DioxusQueryBridge {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.slots, &other.slots)
    }
}

impl DioxusQueryBridge {
    fn new(client: ErasedQueryClient, provider_scope: ScopeId) -> Self {
        Self {
            client,
            provider_scope,
            slots: Rc::new(RefCell::new(BridgeSlots {
                signals: Vec::new(),
                dirty_buf: Vec::with_capacity(16),
            })),
        }
    }

    #[inline]
    pub fn client(&self) -> &ErasedQueryClient {
        &self.client
    }

    /// Acquires (or refcount-increments) the dense `Signal<u32>` for `slot`, allocating it inside
    /// `QueryClientProvider`'s scope so child unmounts never invalidate sibling subscribers.
    pub(crate) fn acquire_slot_signal(&self, slot: QuerySlotId, initial_ver: u32) -> Signal<u32> {
        let mut slots = self.slots.borrow_mut();
        let idx = slot.index();
        if idx >= slots.signals.len() {
            slots.signals.resize(idx + 1, None);
        }
        if let Some((sig, count)) = slots.signals[idx].as_mut() {
            *count = count.saturating_add(1);
            return *sig;
        }
        let sig = Signal::new_in_scope(initial_ver, self.provider_scope);
        slots.signals[idx] = Some((sig, 1));
        sig
    }

    /// Decrements the UI subscriber refcount for `slot`, calling `manually_drop()` when it reaches
    /// zero so recycled slots never leak Dioxus signals on long-lived apps.
    pub(crate) fn release_slot_signal(&self, slot: QuerySlotId) {
        let mut slots = self.slots.borrow_mut();
        let idx = slot.index();
        if let Some((sig, count)) = slots.signals.get_mut(idx).and_then(Option::as_mut) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                let sig = *sig;
                slots.signals[idx] = None;
                drop(slots);
                sig.manually_drop();
            }
        }
    }

    /// Drains dirty `(QuerySlotId, data_version)` pairs from [`QueryClient`] and updates ONLY the
    /// affected slot `Signal<u32>`s on the Dioxus UI thread (zero heap allocations in steady state).
    pub fn flush_dirty_slots(&self) {
        let mut to_update: Vec<(Signal<u32>, u32)> = Vec::new();
        {
            let mut slots = self.slots.borrow_mut();
            let BridgeSlots { signals, dirty_buf } = &mut *slots;
            self.client.drain_dirty_slots(dirty_buf);
            for &(slot, ver) in dirty_buf.iter() {
                if let Some((sig, _)) = signals.get(slot.index()).and_then(Option::as_ref)
                    && *sig.peek() != ver
                {
                    to_update.push((*sig, ver));
                }
            }
        }
        for (mut sig, ver) in to_update {
            sig.set(ver);
        }
    }

    /// Returns the number of live slot signals currently held by mounted UI hooks.
    pub fn live_signal_count(&self) -> usize {
        self.slots
            .borrow()
            .signals
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }
}

pub(crate) fn consume_bridge() -> DioxusQueryBridge {
    try_consume_context::<DioxusQueryBridge>().expect(
        "nve-query-dioxus hooks must be called inside a <QueryClientProvider>",
    )
}

/// Provides an [`ErasedQueryClient`] and its zero-polling wake bridge to descendant components.
#[component]
pub fn QueryClientProvider(client: ErasedQueryClient, children: Element) -> Element {
    let bridge = use_hook(|| {
        let scope = current_scope_id();
        let bridge = DioxusQueryBridge::new(client.clone(), scope);
        let notifier = WakeNotifier::default();
        let wake_notifier = notifier.clone();
        client.set_wake_callback(Shared::new(move || {
            wake_notifier.notify();
        }));
        let bridge_for_task = bridge.clone();
        spawn(async move {
            loop {
                poll_fn(|cx| notifier.poll_wait(cx)).await;
                bridge_for_task.flush_dirty_slots();
            }
        });
        provide_context(bridge.clone());
        bridge
    });
    // Drain any dirty updates that landed synchronously before the first render.
    bridge.flush_dirty_slots();

    rsx! { {children} }
}

/// Utility helper yielding once to the host scheduler without depending on Tokio or `web-sys`.
pub(crate) async fn yield_once() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            let w: Waker = cx.waker().clone();
            w.wake();
            Poll::Pending
        }
    })
    .await;
}
