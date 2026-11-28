//! Runtime-owned `NVE_IFACE_HOST = 5` router (`RuntimeHostRouter`).
//!
//! Owns one [`CompletionSink`], one [`OversizedRetryCache`], monotonic per-attach `owner_tag`
//! generation isolation across Tier-2 `cdylib` reloads, central handling of
//! `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME -> WakeBridge`, cross-thread host completion /
//! push / channel ingress, and zero-allocation idle steady-state batch draining inside
//! `SurfaceRuntime::tick`.

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

enum HostBackend {
    None,
    Callbacks(NveHostCallbacks),
    Provider(Arc<dyn HostServiceProvider>),
}

/// Shared state backing [`RuntimeHostRouter`] and every per-attach [`OwnerBinding`].
pub(crate) struct HostRouterShared {
    sink: CompletionSink,
    retry_cache: OversizedRetryCache,
    backend: RwLock<HostBackend>,
    wake: Arc<WakeBridge>,
    has_queued: Arc<AtomicBool>,
    in_drain: Arc<AtomicBool>,
    next_owner_tag: AtomicU64,
}

impl HostRouterShared {
    fn new(wake: Arc<WakeBridge>) -> Self {
        let sink = CompletionSink::new();
        let has_queued = Arc::new(AtomicBool::new(false));
        let in_drain = Arc::new(AtomicBool::new(false));
        let queued_flag = Arc::clone(&has_queued);
        let draining_flag = Arc::clone(&in_drain);
        let wake_cb = Arc::clone(&wake);
        sink.set_on_ready(Some(Arc::new(move || {
            // SeqCst pair with `drain_into_app`: when draining under the surface write lock,
            // defer host wake callbacks to `tick_shared` outside the lock without losing wakes.
            queued_flag.store(true, Ordering::SeqCst);
            if !draining_flag.load(Ordering::SeqCst) {
                wake_cb.trigger();
            }
        })));
        let backend = RwLock::new(HostBackend::None);
        // Prime OS rwlock storage eagerly so hot-path reads never lazily allocate.
        drop(backend.read());
        Self {
            sink,
            retry_cache: OversizedRetryCache::new(),
            backend,
            wake,
            has_queued,
            in_drain,
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
}

/// Heap-pinned per-attach host callback context handed to an attached [`RuntimeApp`] via
/// `bind_host`. Every attach / Tier-2 reload receives a fresh monotonic `owner_tag`; deactivating
/// it before unbinding and destroying the old app guarantees stale generations can never route
/// calls or collide on `request_id` with the replacement generation.
pub(crate) struct OwnerBinding {
    owner_tag: u64,
    active: AtomicBool,
    shared: Arc<HostRouterShared>,
}

impl OwnerBinding {
    #[inline]
    pub(crate) fn owner_tag(&self) -> u64 {
        self.owner_tag
    }

    #[inline]
    pub(crate) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    /// Returns the C-ABI [`NveHostCallbacks`] pointing to this heap-pinned binding.
    ///
    /// Valid until this [`OwnerBinding`] is dropped (which strictly happens AFTER the bound app
    /// has been unbound via `bind_host(None)` and dropped).
    pub(crate) fn callbacks(&self) -> NveHostCallbacks {
        let ctx: *const Self = self;
        NveHostCallbacks {
            host_ctx: ctx.cast_mut().cast::<c_void>(),
            invoke: Some(router_invoke),
            channel_publish: Some(router_channel_publish),
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

    /// Allocates a fresh pinned [`OwnerBinding`] with a unique monotonic `owner_tag`.
    pub(crate) fn new_owner_binding(&self) -> Box<OwnerBinding> {
        Box::new(OwnerBinding {
            owner_tag: self.shared.alloc_owner_tag(),
            active: AtomicBool::new(true),
            shared: Arc::clone(&self.shared),
        })
    }

    /// Installs or clears the host's raw C-ABI [`NveHostCallbacks`] table, synchronously draining
    /// any in-flight host calls before returning.
    pub fn set_callbacks(&self, callbacks: Option<NveHostCallbacks>) {
        let mut guard = self
            .shared
            .backend
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = match callbacks {
            Some(cb) => HostBackend::Callbacks(cb),
            None => HostBackend::None,
        };
        self.shared.retry_cache.clear();
    }

    /// Installs or clears an in-process Rust [`HostServiceProvider`], synchronously draining any
    /// in-flight calls before returning.
    pub fn set_provider(&self, provider: Option<Arc<dyn HostServiceProvider>>) {
        let mut guard = self
            .shared
            .backend
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = match provider {
            Some(p) => HostBackend::Provider(p),
            None => HostBackend::None,
        };
        self.shared.retry_cache.clear();
    }

    /// Clears the installed callbacks/provider without touching pending sink queues (used early
    /// in teardown so no new host callbacks fire while draining readers).
    pub(crate) fn clear_callbacks(&self) {
        self.set_callbacks(None);
    }

    /// Clears `on_ready`, callbacks, retry cache, and pending sink state on runtime drop.
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
        struct DrainGuard<'a>(&'a AtomicBool);
        impl Drop for DrainGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        // Suppress synchronous wake callbacks during drain (while the surface write lock is held)
        // and clear `has_queued` BEFORE locking `sink` so concurrent pushes or cap-deferred records
        // re-arm `has_queued` and trigger `follow_up` wake after `with_write` releases.
        self.shared.in_drain.store(true, Ordering::SeqCst);
        let written = {
            let _guard = DrainGuard(&self.shared.in_drain);
            self.shared.has_queued.store(false, Ordering::SeqCst);
            builder.clear();
            self.shared
                .sink
                .drain_owner_and_broadcasts_into(owner_tag, builder)
                .unwrap_or(0)
        };
        let more = self.shared.has_queued.load(Ordering::SeqCst);
        if written == 0 {
            return (false, more);
        }
        let dirtied = app.dispatch_host_events(builder.as_bytes());
        (dirtied, more)
    }

    /// Number of currently in-flight async `Pending` requests tracked by this router.
    #[inline]
    pub fn in_flight_count(&self) -> usize {
        self.shared.sink.in_flight_count()
    }
}

/// Safely unbinds (`active = false`, `bind_host(None)`), discards `owner_tag`'s pending records,
/// drops `old_app`, and ONLY THEN drops `old_binding` so the old app/dylib can never dereference
/// a freed `OwnerBinding` during teardown.
pub(crate) fn unbind_and_drop_app(
    old_app: Option<Box<dyn RuntimeApp>>,
    old_binding: Option<Box<OwnerBinding>>,
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
unsafe extern "C" fn router_invoke(
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
        // SAFETY: `host_ctx` points to a heap-pinned `OwnerBinding` outliving the bound app.
        let binding = unsafe { &*(host_ctx as *const OwnerBinding) };

        // Handle `SERVICE_WINDOW::METHOD_WINDOW_REQUEST_FRAME` centrally with 0 locks / 0 allocs.
        if service_id == SERVICE_WINDOW && method_id == METHOD_WINDOW_REQUEST_FRAME {
            if !out_len.is_null() {
                // SAFETY: Caller supplied a writable `out_len` pointer.
                unsafe { *out_len = 0 };
            }
            if binding.active.load(Ordering::Acquire) {
                binding.shared.wake.trigger();
            }
            return NveHostStatus::Ok.as_i32();
        }

        if !binding.active.load(Ordering::Acquire) {
            return NveHostStatus::Cancelled.as_i32();
        }

        // Fast-path cached oversized synchronous retry (`BufferTooSmall` retry).
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

        let guard = binding
            .shared
            .backend
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match &*guard {
            HostBackend::None => NveHostStatus::Unsupported.as_i32(),
            HostBackend::Callbacks(cb) => {
                let Some(invoke_fn) = cb.invoke else {
                    return NveHostStatus::Unsupported.as_i32();
                };
                let Some(host_req_id) = binding.shared.sink.register_in_flight_for_owner(
                    binding.owner_tag,
                    caller_request_id,
                    service_id,
                    method_id,
                ) else {
                    return NveHostStatus::InvalidArg.as_i32();
                };
                if !binding.active.load(Ordering::Acquire) {
                    binding.shared.sink.finish_sync(host_req_id);
                    return NveHostStatus::Cancelled.as_i32();
                }
                // SAFETY: `cb` is held behind the `backend` read lock so `set_callbacks(None)`
                // waits for this call to return; `request`, `out_buf`, `out_len` satisfy C ABI.
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
                if NveHostStatus::from_i32(rc) != NveHostStatus::Pending {
                    binding.shared.sink.finish_sync(host_req_id);
                }
                rc
            }
            HostBackend::Provider(provider) => {
                let Some(host_req_id) = binding.shared.sink.register_in_flight_for_owner(
                    binding.owner_tag,
                    caller_request_id,
                    service_id,
                    method_id,
                ) else {
                    return NveHostStatus::InvalidArg.as_i32();
                };
                if !binding.active.load(Ordering::Acquire) {
                    binding.shared.sink.finish_sync(host_req_id);
                    return NveHostStatus::Cancelled.as_i32();
                }
                // SAFETY: Caller guarantees `request` is valid for the synchronous invoke call.
                let req_bytes = unsafe { request.as_slice() };
                match provider.invoke_raw_with_id(service_id, method_id, host_req_id, req_bytes) {
                    Ok(InvokeResult::Ready(bytes)) => {
                        binding.shared.sink.finish_sync(host_req_id);
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
                        if !out_len.is_null() {
                            // SAFETY: Caller supplied a writable `out_len` pointer.
                            unsafe { *out_len = 0 };
                        }
                        NveHostStatus::Pending.as_i32()
                    }
                    Err(err) => {
                        binding.shared.sink.finish_sync(host_req_id);
                        err.to_status().as_i32()
                    }
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

unsafe extern "C" fn router_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `host_ctx` points to a heap-pinned `OwnerBinding` outliving the bound app.
        let binding = unsafe { &*(host_ctx as *const OwnerBinding) };
        if !binding.active.load(Ordering::Acquire) {
            return NveHostStatus::Cancelled.as_i32();
        }
        let guard = binding
            .shared
            .backend
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match &*guard {
            HostBackend::None => NveHostStatus::Unsupported.as_i32(),
            HostBackend::Callbacks(cb) => {
                let Some(pub_fn) = cb.channel_publish else {
                    return NveHostStatus::Unsupported.as_i32();
                };
                // SAFETY: `cb` is held behind the `backend` read lock; `payload` satisfies C ABI.
                unsafe { pub_fn(cb.host_ctx, channel_id, flags, payload) }
            }
            HostBackend::Provider(provider) => {
                // SAFETY: Caller guarantees `payload` is valid for the synchronous call.
                let bytes = unsafe { payload.as_slice() };
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
