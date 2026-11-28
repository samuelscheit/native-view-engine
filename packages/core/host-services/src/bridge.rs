//! Heap-pinned C-ABI bridge exposing any [`HostServiceProvider`] as [`NveHostCallbacks`]
//! for `NVE_IFACE_HOST = 5` (`NveHostApi::bind_host`), with single-execution retry caching for
//! synchronous responses larger than the caller's initial stack buffer (`BufferTooSmall`).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use nve_abi::{NveBufferSlice, NveHostCallbacks, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN};

use crate::provider::{HostServiceProvider, InvokeResult, ResponseBuf};

const MAX_RETRY_CACHE_ENTRIES: usize = 16;

struct CachedRetryEntry {
    request_id: u64,
    service_id: u32,
    method_id: u32,
    payload: ResponseBuf,
}

/// Bounded lock-free-checked single-retry cache for `Ready` payloads exceeding the caller's initial
/// stack buffer (`NVE_HOST_BUFFER_TOO_SMALL`).
///
/// The fast path pays a single relaxed atomic load (`has_entries == false`) with zero locking and
/// zero heap allocation; oversized responses are stashed once and served directly on the caller's
/// immediate sized retry so side-effectful handlers never execute twice.
pub struct OversizedRetryCache {
    has_entries: AtomicBool,
    entries: Mutex<VecDeque<CachedRetryEntry>>,
}

impl Default for OversizedRetryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OversizedRetryCache {
    pub fn new() -> Self {
        let entries = Mutex::new(VecDeque::new());
        drop(entries.lock());
        Self {
            has_entries: AtomicBool::new(false),
            entries,
        }
    }

    #[inline]
    fn take(&self, request_id: u64, service_id: u32, method_id: u32) -> Option<ResponseBuf> {
        if !self.has_entries.load(Ordering::Acquire) {
            return None;
        }
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let pos = guard.iter().position(|e| {
            e.request_id == request_id && e.service_id == service_id && e.method_id == method_id
        })?;
        let entry = guard.remove(pos)?;
        if guard.is_empty() {
            self.has_entries.store(false, Ordering::Release);
        }
        Some(entry.payload)
    }

    fn store(&self, request_id: u64, service_id: u32, method_id: u32, payload: ResponseBuf) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|e| {
            !(e.request_id == request_id && e.service_id == service_id && e.method_id == method_id)
        });
        while guard.len() >= MAX_RETRY_CACHE_ENTRIES {
            let _ = guard.pop_front();
        }
        guard.push_back(CachedRetryEntry {
            request_id,
            service_id,
            method_id,
            payload,
        });
        self.has_entries.store(true, Ordering::Release);
    }

    pub fn clear(&self) {
        if !self.has_entries.load(Ordering::Acquire) {
            return;
        }
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
        self.has_entries.store(false, Ordering::Release);
    }
}

/// Shared panic-safe C-ABI `invoke` dispatcher with optional [`OversizedRetryCache`] so oversized
/// `BufferTooSmall` retries return the cached response without executing the handler twice.
///
/// # Safety
/// - When `host_ctx` is non-null it must point to a valid, pinned `P` for the duration of the call.
/// - `request`, `out_buf` (for `out_cap` bytes when non-null), and `out_len` (when non-null) must
///   satisfy the `NveHostCallbacks::invoke` C ABI contract.
#[allow(clippy::too_many_arguments)]
pub unsafe fn dispatch_ffi_invoke_with_cache<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
    retry_cache: Option<&OversizedRetryCache>,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        let cached = retry_cache.and_then(|c| c.take(request_id, service_id, method_id));
        let outcome = if let Some(bytes) = cached {
            Ok(InvokeResult::Ready(bytes))
        } else {
            // SAFETY: Caller guarantees `host_ctx` points to a live pinned `P` and `request` is valid.
            let (provider, req_bytes) = unsafe { (&*(host_ctx as *const P), request.as_slice()) };
            provider.invoke_raw_with_id(service_id, method_id, request_id, req_bytes)
        };
        match outcome {
            Ok(InvokeResult::Ready(bytes)) => {
                if !out_len.is_null() {
                    // SAFETY: Caller provided a valid writable `out_len` pointer.
                    unsafe { *out_len = bytes.len() };
                }
                if bytes.len() > out_cap {
                    if let Some(cache) = retry_cache {
                        cache.store(request_id, service_id, method_id, bytes);
                    }
                    return NveHostStatus::BufferTooSmall.as_i32();
                }
                if !bytes.is_empty() {
                    if out_buf.is_null() {
                        return NveHostStatus::InvalidArg.as_i32();
                    }
                    // SAFETY: `out_buf` is non-null with capacity `out_cap >= bytes.len()` and non-overlapping.
                    unsafe {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf, bytes.len());
                    }
                }
                NveHostStatus::Ok.as_i32()
            }
            Ok(InvokeResult::Pending { .. }) => {
                if !out_len.is_null() {
                    // SAFETY: Caller provided a valid writable `out_len` pointer.
                    unsafe { *out_len = 0 };
                }
                NveHostStatus::Pending.as_i32()
            }
            Err(err) => err.to_status().as_i32(),
        }
    }));
    match result {
        Ok(code) => code,
        Err(_) => {
            if !out_len.is_null() {
                // SAFETY: Caller provided a valid writable `out_len` pointer.
                unsafe { *out_len = 0 };
            }
            NveHostStatus::InternalError.as_i32()
        }
    }
}

/// Shared panic-safe C-ABI `invoke` dispatcher (without retry cache).
///
/// # Safety
/// Same requirements as [`dispatch_ffi_invoke_with_cache`].
#[allow(clippy::too_many_arguments)]
pub unsafe fn dispatch_ffi_invoke<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    // SAFETY: Caller satisfies `dispatch_ffi_invoke_with_cache` pointer & slice contract.
    unsafe {
        dispatch_ffi_invoke_with_cache::<P>(
            host_ctx, service_id, method_id, request_id, request, out_buf, out_cap, out_len, None,
        )
    }
}

/// Shared panic-safe C-ABI `channel_publish` dispatcher used by [`HostCallbackBridge`] and
/// native shells so panics never unwind across `extern "C"` boundaries.
///
/// # Safety
/// - When `host_ctx` is non-null it must point to a valid, pinned `P` for the duration of the call.
/// - `payload` must satisfy the `NveHostCallbacks::channel_publish` C ABI contract.
pub unsafe fn dispatch_ffi_channel_publish<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Caller guarantees `host_ctx` points to a live pinned `P` and `payload` is valid.
        let (provider, bytes) = unsafe { (&*(host_ctx as *const P), payload.as_slice()) };
        let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
        match provider.channel_publish(channel_id, retain, bytes) {
            Ok(()) => NveHostStatus::Ok.as_i32(),
            Err(err) => err.to_status().as_i32(),
        }
    }));
    result.unwrap_or_else(|_| NveHostStatus::InternalError.as_i32())
}

struct BridgeState<P: HostServiceProvider> {
    provider: P,
    retry_cache: OversizedRetryCache,
}

/// Heap-pinned C-ABI bridge exposing any [`HostServiceProvider`] as [`NveHostCallbacks`]
/// suitable for `NveHostApi::bind_host`.
pub struct HostCallbackBridge<P: HostServiceProvider> {
    inner: Box<BridgeState<P>>,
}

impl<P: HostServiceProvider> HostCallbackBridge<P> {
    pub fn new(provider: P) -> Self {
        Self {
            inner: Box::new(BridgeState {
                provider,
                retry_cache: OversizedRetryCache::new(),
            }),
        }
    }

    pub fn provider(&self) -> &P {
        &self.inner.provider
    }

    /// Returns a C-ABI [`NveHostCallbacks`] valid for as long as `self` remains alive and unmoved.
    pub fn callbacks(&self) -> NveHostCallbacks {
        let ptr: *const BridgeState<P> = &*self.inner;
        NveHostCallbacks {
            host_ctx: ptr as *mut c_void,
            invoke: Some(bridge_invoke::<P>),
            channel_publish: Some(bridge_channel_publish::<P>),
        }
    }
}

unsafe extern "C" fn bridge_invoke<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    // SAFETY: `host_ctx` is a pinned `Box<BridgeState<P>>` pointer from `HostCallbackBridge::callbacks`.
    let state = unsafe { &*(host_ctx as *const BridgeState<P>) };
    let provider_ptr = (&state.provider as *const P) as *mut c_void;
    // SAFETY: `provider_ptr` points to the live pinned `P` inside `BridgeState<P>`.
    unsafe {
        dispatch_ffi_invoke_with_cache::<P>(
            provider_ptr,
            service_id,
            method_id,
            request_id,
            request,
            out_buf,
            out_cap,
            out_len,
            Some(&state.retry_cache),
        )
    }
}

unsafe extern "C" fn bridge_channel_publish<P: HostServiceProvider>(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    // SAFETY: `host_ctx` is a pinned `Box<BridgeState<P>>` pointer from `HostCallbackBridge::callbacks`.
    let state = unsafe { &*(host_ctx as *const BridgeState<P>) };
    let provider_ptr = (&state.provider as *const P) as *mut c_void;
    // SAFETY: `provider_ptr` points to the live pinned `P` inside `BridgeState<P>`.
    unsafe { dispatch_ffi_channel_publish::<P>(provider_ptr, channel_id, flags, payload) }
}
