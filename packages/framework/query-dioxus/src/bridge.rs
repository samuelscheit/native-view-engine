//! Transport-erased [`ErasedQueryClient`], lock-free cross-thread wake notifier, dense recycled
//! `QuerySlotId` -> `Signal<u32>` bridge, and [`QueryClientProvider`].

use core::cell::RefCell;
use core::future::{poll_fn, Future};
use std::rc::Rc;

use dioxus_core::{
    current_scope_id, provide_context, spawn, try_consume_context, use_drop, use_hook, Element,
    Runtime, ScopeId, Task,
};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_query::{Clock, QueryClient, QueryClientConfig, QueryHandle, QuerySlotId, Shared};
use nve_rpc::{BoxFuture, CborCodec, RawCall, RawReply, Transport, TransportError};

use crate::sleep::{default_sleep_callback, SleepHandle};

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

struct SlotTaskEntry {
    seq: u64,
    slot_gen: u32,
    inv_ver: u64,
    task: Task,
}

struct BridgeSlots {
    alive: bool,
    next_task_seq: u64,
    signals: Vec<Option<(Signal<u32>, u32)>>,
    slot_tasks: Vec<Option<SlotTaskEntry>>,
    dirty_buf: Vec<(QuerySlotId, u32)>,
    to_update_buf: Vec<(Signal<u32>, u32)>,
}

struct SlotTaskGuard {
    slots: Rc<RefCell<BridgeSlots>>,
    slot: QuerySlotId,
    seq: u64,
}

impl Drop for SlotTaskGuard {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.slots.try_borrow_mut()
            && let Some(entry) = slots.slot_tasks.get_mut(self.slot.index())
            && entry.as_ref().is_some_and(|e| e.seq == self.seq)
        {
            *entry = None;
        }
    }
}

/// Reactive UI-thread bridge connecting an [`ErasedQueryClient`] to dense recycled per-slot
/// `Signal<u32>`s and provider-scoped tasks owned by [`QueryClientProvider`]'s scope.
#[derive(Clone)]
pub struct DioxusQueryBridge {
    client: ErasedQueryClient,
    provider_scope: ScopeId,
    sleep: SleepHandle,
    slots: Rc<RefCell<BridgeSlots>>,
}

impl PartialEq for DioxusQueryBridge {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.slots, &other.slots)
    }
}

impl DioxusQueryBridge {
    fn new(client: ErasedQueryClient, provider_scope: ScopeId, sleep: SleepHandle) -> Self {
        Self {
            client,
            provider_scope,
            sleep,
            slots: Rc::new(RefCell::new(BridgeSlots {
                alive: true,
                next_task_seq: 1,
                signals: Vec::new(),
                slot_tasks: Vec::new(),
                dirty_buf: Vec::with_capacity(16),
                to_update_buf: Vec::with_capacity(16),
            })),
        }
    }

    fn mark_unmounted(&self) {
        self.slots.borrow_mut().alive = false;
    }

    #[inline]
    pub fn client(&self) -> &ErasedQueryClient {
        &self.client
    }

    #[inline]
    pub(crate) fn sleep_ms(&self, ms: u32) -> BoxFuture<'static, ()> {
        self.sleep.sleep_ms(ms)
    }

    /// Spawns a future inside [`QueryClientProvider`]'s scope (registered in `spawned_tasks`) so
    /// leaf component unmounts never cancel shared fetches or mutations while provider unmount
    /// drops every pending task cleanly before its first poll.
    #[inline]
    pub(crate) fn spawn_in_provider(&self, fut: impl Future<Output = ()> + 'static) -> Option<Task> {
        if !self.slots.borrow().alive {
            return None;
        }
        Some(Runtime::current().in_scope(self.provider_scope, || spawn(fut)))
    }

    /// Returns `true` when an active provider-owned fetch or retry-backoff task is currently
    /// running for `(handle.slot, handle.generation, inv_ver)`.
    pub(crate) fn has_active_slot_task_for(&self, handle: QueryHandle, inv_ver: u64) -> bool {
        self.slots
            .borrow()
            .slot_tasks
            .get(handle.slot.index())
            .and_then(Option::as_ref)
            .is_some_and(|entry| entry.slot_gen == handle.generation && entry.inv_ver == inv_ver)
    }

    /// Spawns (or replaces) the provider-owned fetch task for `(handle, inv_ver)`, cancelling any
    /// superseded prior task outside the `RefCell` borrow and clearing the entry automatically via
    /// RAII when the task completes or drops so finished failures never block later mounts.
    pub(crate) fn spawn_slot_fetch(
        &self,
        handle: QueryHandle,
        inv_ver: u64,
        fut: impl Future<Output = ()> + 'static,
    ) {
        let seq = {
            let mut slots = self.slots.borrow_mut();
            if !slots.alive {
                return;
            }
            let seq = slots.next_task_seq;
            slots.next_task_seq = slots.next_task_seq.wrapping_add(1).max(1);
            seq
        };
        let guard = SlotTaskGuard {
            slots: Rc::clone(&self.slots),
            slot: handle.slot,
            seq,
        };
        let Some(task) = self.spawn_in_provider(async move {
            let _guard = guard;
            fut.await;
        }) else {
            return;
        };
        let prev = {
            let mut slots = self.slots.borrow_mut();
            let idx = handle.slot.index();
            if idx >= slots.slot_tasks.len() {
                slots.slot_tasks.resize_with(idx + 1, || None);
            }
            slots.slot_tasks[idx].replace(SlotTaskEntry {
                seq,
                slot_gen: handle.generation,
                inv_ver,
                task,
            })
        };
        if let Some(prev) = prev {
            prev.task.cancel();
        }
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

    /// Decrements the UI subscriber refcount for `slot`, cancelling any slot-owned fetch task and
    /// calling `manually_drop()` when it reaches zero so recycled slots never leak Dioxus signals.
    pub(crate) fn release_slot_signal(&self, slot: QuerySlotId) {
        let mut slots = self.slots.borrow_mut();
        let idx = slot.index();
        if let Some((sig, count)) = slots.signals.get_mut(idx).and_then(Option::as_mut) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                let sig = *sig;
                slots.signals[idx] = None;
                let entry = slots.slot_tasks.get_mut(idx).and_then(Option::take);
                drop(slots);
                if let Some(entry) = entry {
                    entry.task.cancel();
                }
                sig.manually_drop();
            }
        }
    }

    /// Drains dirty `(QuerySlotId, data_version)` pairs from [`QueryClient`] and updates ONLY the
    /// affected slot `Signal<u32>`s on the Dioxus UI thread (zero bridge heap allocations in
    /// steady state).
    pub fn flush_dirty_slots(&self) {
        let mut to_update = {
            let mut slots = self.slots.borrow_mut();
            let BridgeSlots {
                signals,
                dirty_buf,
                to_update_buf,
                ..
            } = &mut *slots;
            to_update_buf.clear();
            self.client.drain_dirty_slots(dirty_buf);
            for &(slot, ver) in dirty_buf.iter() {
                if let Some((sig, _)) = signals.get(slot.index()).and_then(Option::as_ref)
                    && *sig.peek() != ver
                {
                    to_update_buf.push((*sig, ver));
                }
            }
            core::mem::take(to_update_buf)
        };
        for &(mut sig, ver) in &to_update {
            sig.set(ver);
        }
        to_update.clear();
        let mut slots = self.slots.borrow_mut();
        if slots.to_update_buf.capacity() < to_update.capacity() {
            slots.to_update_buf = to_update;
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
///
/// Pass an optional `sleep` handle to override the exponential retry-backoff timer (e.g.
/// `SleepHandle::new(nve_http_web::browser_sleep_ms)` on `wasm32` or a deterministic mock in tests).
#[component]
pub fn QueryClientProvider(
    client: ErasedQueryClient,
    #[props(default)] sleep: Option<SleepHandle>,
    children: Element,
) -> Element {
    let client_for_drop = client.clone();
    let (wake_token, bridge_for_drop) = use_hook(move || {
        let scope = current_scope_id();
        let sleep_handle = sleep.unwrap_or_else(|| SleepHandle(default_sleep_callback()));
        let bridge = DioxusQueryBridge::new(client.clone(), scope, sleep_handle);
        let notifier = WakeNotifier::default();
        let wake_notifier = notifier.clone();
        let token = client.set_wake_callback(Shared::new(move || {
            wake_notifier.notify();
        }));
        // Trigger one initial async drain after mount so pre-existing dirty slots flush without
        // mutating signals inside `QueryClientProvider`'s own render body.
        notifier.notify();
        let bridge_for_task = bridge.clone();
        spawn(async move {
            loop {
                poll_fn(|cx| notifier.poll_wait(cx)).await;
                bridge_for_task.flush_dirty_slots();
            }
        });
        provide_context(bridge.clone());
        (token, bridge)
    });

    use_drop(move || {
        bridge_for_drop.mark_unmounted();
        client_for_drop.clear_wake_callback(wake_token);
    });

    rsx! { {children} }
}
