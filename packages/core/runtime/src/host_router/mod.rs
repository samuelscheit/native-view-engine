//! Runtime-owned `NVE_IFACE_HOST = 5` router (`RuntimeHostRouter`).
//!
//! Owns one [`CompletionSink`], one [`OversizedRetryCache`], monotonic per-attach `owner_tag`
//! generation isolation across Tier-2 `cdylib` reloads, central handling of
//! `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME -> WakeBridge`, cross-thread host completion /
//! push / channel ingress, and zero-allocation idle steady-state batch draining inside
//! `SurfaceRuntime::tick`.

mod slots;

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nve_abi::{
    HostRequestId, HostServiceId, NveBufferSlice, NveHostCallbacks, NveHostEventBatchBuilder,
    NveHostStatus, METHOD_WINDOW_REQUEST_FRAME, NVE_CHANNEL_FLAG_RETAIN, SERVICE_WINDOW,
};
use nve_host_services::{CompletionSink, HostServiceProvider, InvokeResult, OversizedRetryCache};

use crate::app::RuntimeApp;
use crate::wake::WakeBridge;

pub(crate) use slots::OwnerBindingHandle;
use slots::{lookup_owner_binding, CallbackRetryIds, CallbacksSlot, SlotCallGuard};

enum HostBackend {
    None,
    Callbacks(Arc<CallbacksSlot>),
    Provider(Arc<dyn HostServiceProvider>),
}

enum AcquiredBackend {
    None,
    Callbacks(SlotCallGuard),
    Provider(Arc<dyn HostServiceProvider>),
}

/// RAII guard cleaning up an in-flight `CompletionSink` slot unless disarmed on `Pending`.
struct InFlightGuard<'a> {
    sink: &'a CompletionSink,
    host_req_id: u64,
    armed: bool,
}

impl<'a> InFlightGuard<'a> {
    #[inline]
    fn new(sink: &'a CompletionSink, host_req_id: u64) -> Self {
        Self {
            sink,
            host_req_id,
            armed: true,
        }
    }

    #[inline]
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.sink.finish_sync(self.host_req_id);
        }
    }
}

/// Shared state backing [`RuntimeHostRouter`] and every per-attach [`OwnerBindingHandle`].
pub(crate) struct HostRouterShared {
    sink: CompletionSink,
    retry_cache: OversizedRetryCache,
    callback_retry_ids: CallbackRetryIds,
    backend: RwLock<HostBackend>,
    wake: Arc<WakeBridge>,
    has_queued: Arc<AtomicBool>,
    next_owner_tag: AtomicU64,
}

impl HostRouterShared {
    fn new(wake: Arc<WakeBridge>) -> Self {
        let sink = CompletionSink::new();
        let has_queued = Arc::new(AtomicBool::new(false));
        let queued_flag = Arc::clone(&has_queued);
        let wake_cb = Arc::clone(&wake);
        sink.set_on_ready(Some(Arc::new(move || {
            // `WakeBridge::defer_guard` in `SurfaceRuntime::with_write` defers synchronous wake
            // delivery until after the surface write lock releases.
            queued_flag.store(true, Ordering::SeqCst);
            wake_cb.trigger();
        })));
        let backend = RwLock::new(HostBackend::None);
        // Prime OS rwlock storage eagerly so hot-path reads never lazily allocate.
        drop(backend.read());
        Self {
            sink,
            retry_cache: OversizedRetryCache::new(),
            callback_retry_ids: CallbackRetryIds::new(),
            backend,
            wake,
            has_queued,
            next_owner_tag: AtomicU64::new(1),
        }
    }

    fn alloc_owner_tag(&self) -> u64 {
        loop {
            let tag = self.next_owner_tag.fetch_add(1, Ordering::Relaxed);
            if tag != 0 {
                return tag;
            }
        }
    }

    fn acquire_backend(&self) -> AcquiredBackend {
        let guard = self.backend.read().unwrap_or_else(|e| e.into_inner());
        match &*guard {
            HostBackend::None => AcquiredBackend::None,
            HostBackend::Callbacks(slot) => AcquiredBackend::Callbacks(slot.enter()),
            HostBackend::Provider(provider) => AcquiredBackend::Provider(Arc::clone(provider)),
        }
    }

    fn swap_backend(&self, next: HostBackend) {
        let prev = {
            let mut guard = self.backend.write().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, next)
        };
        self.retry_cache.clear();
        self.callback_retry_ids.clear();
        if let HostBackend::Callbacks(old_slot) = prev {
            old_slot.drain_other_threads();
        }
    }
}

/// Runtime-owned `NVE_IFACE_HOST = 5` router attached to one [`crate::SurfaceRuntime`].
pub struct RuntimeHostRouter {
    shared: Arc<HostRouterShared>,
}

impl RuntimeHostRouter {
    pub(crate) fn new(wake: Arc<WakeBridge>) -> Self {
        Self {
            shared: Arc::new(HostRouterShared::new(wake)),
        }
    }

    /// Allocates a fresh generational [`OwnerBindingHandle`] with a unique monotonic `owner_tag`.
    pub(crate) fn new_owner_binding(&self) -> OwnerBindingHandle {
        OwnerBindingHandle::register(self.shared.alloc_owner_tag(), Arc::clone(&self.shared))
    }

    /// Installs or clears the host's raw C-ABI [`NveHostCallbacks`] table, synchronously draining
    /// any other threads' in-flight host calls before returning without deadlocking on same-thread
    /// re-entrancy.
    pub fn set_callbacks(&self, callbacks: Option<NveHostCallbacks>) {
        let next = match callbacks {
            Some(cb) => HostBackend::Callbacks(CallbacksSlot::new(cb)),
            None => HostBackend::None,
        };
        self.shared.swap_backend(next);
    }

    /// Installs or clears an in-process Rust [`HostServiceProvider`], synchronously draining any
    /// prior C-callback slot's cross-thread calls before returning.
    pub fn set_provider(&self, provider: Option<Arc<dyn HostServiceProvider>>) {
        let next = match provider {
            Some(p) => HostBackend::Provider(p),
            None => HostBackend::None,
        };
        self.shared.swap_backend(next);
    }

    /// Clears the installed callbacks/provider without touching pending sink queues (used early
    /// in teardown so no new host callbacks fire while draining readers).
    pub(crate) fn clear_callbacks(&self) {
        self.set_callbacks(None);
    }

    /// Clears `on_ready`, callbacks, retry caches, and pending sink state on runtime drop.
    pub(crate) fn shutdown(&self) {
        self.shared.sink.set_on_ready(None);
        self.set_callbacks(None);
        self.shared.sink.discard_all_pending();
        self.shared.has_queued.store(false, Ordering::Release);
    }

    /// Discards all in-flight and queued `Response` records belonging to `owner_tag` and clears
    /// cached oversized sync retries so a reloaded generation never sees stale responses.
    pub(crate) fn discard_pending_for_owner(&self, owner_tag: u64) -> usize {
        self.shared.retry_cache.clear();
        self.shared.callback_retry_ids.clear_owner(owner_tag);
        self.shared.sink.discard_pending_for_owner(owner_tag)
    }

    /// Completes an asynchronous `Pending` host request from ANY thread. Returns `true` when
    /// `request_id` was live and resolved, or `false` when stale/unknown/duplicate (safely dropped).
    pub fn complete_request(
        &self,
        request_id: HostRequestId,
        status: NveHostStatus,
        payload: &[u8],
    ) -> bool {
        self.shared
            .sink
            .complete(request_id.raw(), status, payload.to_vec())
    }

    /// Enqueues an unsolicited host push event from ANY thread and triggers [`WakeBridge`].
    pub fn push_event(&self, service_id: HostServiceId, event_id: u32, payload: &[u8]) {
        self.shared
            .sink
            .push_event(service_id.raw(), event_id, payload.to_vec());
    }

    /// Enqueues an inbound host channel message from ANY thread and triggers [`WakeBridge`].
    pub fn publish_channel(&self, channel_id: u32, flags: u32, payload: &[u8]) {
        let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
        self.shared
            .sink
            .push_channel(channel_id, retain, payload.to_vec());
    }

    /// Drains queued records for `owner_tag` (plus unscoped `Push`/`Channel` records) into
    /// `builder` and delivers the canonical `NVHB` v1 batch to `app.dispatch_host_events`.
    ///
    /// **Zero-allocation idle fast path**: when no records were queued since the last drain,
    /// returns `(false, false)` after a single atomic load without locking or touching `builder`.
    ///
    /// Returns `(app_dirtied, has_more_queued)`.
    pub(crate) fn drain_into_app(
        &self,
        owner_tag: u64,
        builder: &mut NveHostEventBatchBuilder,
        app: &mut dyn RuntimeApp,
    ) -> (bool, bool) {
        if !self.shared.has_queued.load(Ordering::Acquire) {
            return (false, false);
        }
        self.shared.has_queued.store(false, Ordering::SeqCst);
        builder.clear();
        let written = self
            .shared
            .sink
            .drain_owner_and_broadcasts_into(owner_tag, builder)
            .unwrap_or(0);
        let more_after_drain = self.shared.has_queued.load(Ordering::SeqCst);
        if written == 0 {
            return (false, more_after_drain);
        }
        let dirtied = app.dispatch_host_events(builder.as_bytes());
        let more = more_after_drain || self.shared.has_queued.load(Ordering::SeqCst);
        (dirtied, more)
    }

    /// Number of currently in-flight async `Pending` requests tracked by this router.
    #[inline]
    pub fn in_flight_count(&self) -> usize {
        self.shared.sink.in_flight_count()
    }
}

/// Safely unbinds (`active = false`, removes generational binding token, `bind_host(None)`),
/// discards `owner_tag`'s pending records, drops `old_app`, and drops `old_binding`.
pub(crate) fn unbind_and_drop_app(
    old_app: Option<Box<dyn RuntimeApp>>,
    old_binding: Option<OwnerBindingHandle>,
    router: &RuntimeHostRouter,
) {
    if let Some(ref b) = old_binding {
        b.deactivate();
    }
    if let Some(mut app) = old_app {
        app.bind_host(None);
        if let Some(ref b) = old_binding {
            let _ = router.discard_pending_for_owner(b.owner_tag());
        }
        drop(app);
    } else if let Some(ref b) = old_binding {
        let _ = router.discard_pending_for_owner(b.owner_tag());
    }
    drop(old_binding);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe extern "C" fn router_invoke(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    caller_request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let Some(binding) = lookup_owner_binding(host_ctx) else {
            return NveHostStatus::Cancelled.as_i32();
        };
        // SAFETY: Caller guarantees `request` is readable for `request.len` bytes when valid.
        let Some(req_bytes) = (unsafe { request.try_as_slice() }) else {
            return NveHostStatus::InvalidArg.as_i32();
        };

        // Handle `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME` centrally with 0 allocs.
        if service_id == SERVICE_WINDOW && method_id == METHOD_WINDOW_REQUEST_FRAME {
            if !out_len.is_null() {
                // SAFETY: Caller supplied a writable `out_len` pointer.
                unsafe { *out_len = 0 };
            }
            if binding.active.load(Ordering::Acquire) {
                binding.frame_requested.store(true, Ordering::Release);
                binding.shared.wake.trigger();
            }
            return NveHostStatus::Ok.as_i32();
        }

        if !binding.active.load(Ordering::Acquire) {
            return NveHostStatus::Cancelled.as_i32();
        }

        // Fast-path cached oversized synchronous retry (`BufferTooSmall` retry for Provider).
        if let Some(cached) = binding
            .shared
            .retry_cache
            .take(caller_request_id, service_id, method_id)
        {
            if !out_len.is_null() {
                // SAFETY: Caller supplied a writable `out_len` pointer.
                unsafe { *out_len = cached.len() };
            }
            if cached.len() > out_cap {
                binding
                    .shared
                    .retry_cache
                    .store(caller_request_id, service_id, method_id, cached);
                return NveHostStatus::BufferTooSmall.as_i32();
            }
            if !cached.is_empty() {
                if out_buf.is_null() {
                    return NveHostStatus::InvalidArg.as_i32();
                }
                // SAFETY: `out_buf` is non-null with capacity `out_cap >= cached.len()`.
                unsafe {
                    std::ptr::copy_nonoverlapping(cached.as_ptr(), out_buf, cached.len());
                }
            }
            return NveHostStatus::Ok.as_i32();
        }

        // Acquire backend and immediately release `backend.read()` BEFORE running host callbacks so
        // re-entrant `set_callbacks`/`set_provider` calls never deadlock.
        match binding.shared.acquire_backend() {
            AcquiredBackend::None => NveHostStatus::Unsupported.as_i32(),
            AcquiredBackend::Callbacks(slot_guard) => {
                let cb = slot_guard.callbacks();
                let Some(invoke_fn) = cb.invoke else {
                    return NveHostStatus::Unsupported.as_i32();
                };
                let retry_host_id = binding.shared.callback_retry_ids.take(
                    binding.owner_tag,
                    caller_request_id,
                    service_id,
                    method_id,
                );
                let Some(host_req_id) = binding.shared.sink.register_in_flight_for_owner_with_id(
                    binding.owner_tag,
                    caller_request_id,
                    retry_host_id,
                    service_id,
                    method_id,
                ) else {
                    return NveHostStatus::InvalidArg.as_i32();
                };
                let mut in_flight = InFlightGuard::new(&binding.shared.sink, host_req_id);
                if !binding.active.load(Ordering::Acquire) {
                    return NveHostStatus::Cancelled.as_i32();
                }
                // SAFETY: `slot_guard` drains cross-thread `set_callbacks(None)` until this call
                // returns; `request`, `out_buf`, `out_len` satisfy the C ABI contract.
                let rc = unsafe {
                    invoke_fn(
                        cb.host_ctx,
                        service_id,
                        method_id,
                        host_req_id,
                        request,
                        out_buf,
                        out_cap,
                        out_len,
                    )
                };
                match NveHostStatus::from_i32(rc) {
                    NveHostStatus::Pending => in_flight.disarm(),
                    NveHostStatus::BufferTooSmall => {
                        binding.shared.callback_retry_ids.store(
                            binding.owner_tag,
                            caller_request_id,
                            service_id,
                            method_id,
                            host_req_id,
                        );
                    }
                    _ => {}
                }
                rc
            }
            AcquiredBackend::Provider(provider) => {
                let Some(host_req_id) = binding.shared.sink.register_in_flight_for_owner(
                    binding.owner_tag,
                    caller_request_id,
                    service_id,
                    method_id,
                ) else {
                    return NveHostStatus::InvalidArg.as_i32();
                };
                let mut in_flight = InFlightGuard::new(&binding.shared.sink, host_req_id);
                if !binding.active.load(Ordering::Acquire) {
                    return NveHostStatus::Cancelled.as_i32();
                }
                match provider.invoke_raw_with_id(service_id, method_id, host_req_id, req_bytes) {
                    Ok(InvokeResult::Ready(bytes)) => {
                        drop(in_flight);
                        if !out_len.is_null() {
                            // SAFETY: Caller supplied a writable `out_len` pointer.
                            unsafe { *out_len = bytes.len() };
                        }
                        if bytes.len() > out_cap {
                            binding.shared.retry_cache.store(
                                caller_request_id,
                                service_id,
                                method_id,
                                bytes.into_vec(),
                            );
                            return NveHostStatus::BufferTooSmall.as_i32();
                        }
                        if !bytes.is_empty() {
                            if out_buf.is_null() {
                                return NveHostStatus::InvalidArg.as_i32();
                            }
                            // SAFETY: `out_buf` is non-null with capacity `out_cap >= bytes.len()`.
                            unsafe {
                                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
                            }
                        }
                        NveHostStatus::Ok.as_i32()
                    }
                    Ok(InvokeResult::Pending { .. }) => {
                        in_flight.disarm();
                        if !out_len.is_null() {
                            // SAFETY: Caller supplied a writable `out_len` pointer.
                            unsafe { *out_len = 0 };
                        }
                        NveHostStatus::Pending.as_i32()
                    }
                    Err(err) => err.to_status().as_i32(),
                }
            }
        }
    }));
    match res {
        Ok(code) => code,
        Err(_) => {
            if !out_len.is_null() {
                // SAFETY: Caller supplied a writable `out_len` pointer.
                unsafe { *out_len = 0 };
            }
            NveHostStatus::InternalError.as_i32()
        }
    }
}

pub(super) unsafe extern "C" fn router_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let Some(binding) = lookup_owner_binding(host_ctx) else {
            return NveHostStatus::Cancelled.as_i32();
        };
        // SAFETY: Caller guarantees `payload` is readable for `payload.len` bytes when valid.
        let Some(bytes) = (unsafe { payload.try_as_slice() }) else {
            return NveHostStatus::InvalidArg.as_i32();
        };
        if !binding.active.load(Ordering::Acquire) {
            return NveHostStatus::Cancelled.as_i32();
        }
        match binding.shared.acquire_backend() {
            AcquiredBackend::None => NveHostStatus::Unsupported.as_i32(),
            AcquiredBackend::Callbacks(slot_guard) => {
                let cb = slot_guard.callbacks();
                let Some(pub_fn) = cb.channel_publish else {
                    return NveHostStatus::Unsupported.as_i32();
                };
                // SAFETY: `slot_guard` drains cross-thread `set_callbacks(None)`; `payload` validated.
                unsafe { pub_fn(cb.host_ctx, channel_id, flags, payload) }
            }
            AcquiredBackend::Provider(provider) => {
                let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
                match provider.channel_publish(channel_id, retain, bytes) {
                    Ok(()) => NveHostStatus::Ok.as_i32(),
                    Err(err) => err.to_status().as_i32(),
                }
            }
        }
    }));
    res.unwrap_or_else(|_| NveHostStatus::InternalError.as_i32())
}
