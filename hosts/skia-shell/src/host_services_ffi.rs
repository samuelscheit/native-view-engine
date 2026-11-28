//! Owner-scoped C-ABI binding (`SkiaSlotHostBinding`) and panic-safe `extern "C"` trampolines
//! for [`SkiaHostServices`].

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nve_abi::{NveBufferSlice, NveHostCallbacks, NveHostStatus};
use nve_host_services::{
    dispatch_ffi_channel_publish, dispatch_ffi_invoke_with_cache, HostEvent, HostServiceProvider,
    InvokeResult, OversizedRetryCache, ServiceError,
};

use super::SkiaHostServices;

static NEXT_SLOT_OWNER_TAG: AtomicU64 = AtomicU64::new(1);

/// Per-window / per-generation C-ABI binding tagging all `Pending` host calls with a unique
/// `owner_tag` so sibling windows and transactional Tier-2 reloads never cancel or misdeliver
/// each other's in-flight requests.
pub struct SkiaSlotHostBinding {
    host: Arc<SkiaHostServices>,
    owner_tag: u64,
    retry_cache: OversizedRetryCache,
}

impl SkiaSlotHostBinding {
    pub(super) fn new(host: Arc<SkiaHostServices>) -> Arc<Self> {
        let owner_tag = NEXT_SLOT_OWNER_TAG.fetch_add(1, Ordering::Relaxed).max(1);
        Arc::new(Self {
            host,
            owner_tag,
            retry_cache: OversizedRetryCache::new(),
        })
    }

    #[inline]
    pub fn owner_tag(&self) -> u64 {
        self.owner_tag
    }

    pub fn callbacks(self: &Arc<Self>) -> NveHostCallbacks {
        NveHostCallbacks {
            host_ctx: Arc::as_ptr(self) as *mut c_void,
            invoke: Some(skia_slot_invoke),
            channel_publish: Some(skia_slot_channel_publish),
        }
    }
}

impl HostServiceProvider for SkiaSlotHostBinding {
    fn invoke_raw_with_id(
        &self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        self.host.invoke_raw_with_id_for_owner(
            self.owner_tag,
            service_id,
            method_id,
            request_id,
            request,
        )
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        self.host.channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.host.poll_host_events()
    }
}

#[cfg(test)]
pub(super) fn direct_service_callbacks(svc: &Arc<SkiaHostServices>) -> NveHostCallbacks {
    NveHostCallbacks {
        host_ctx: Arc::as_ptr(svc) as *mut c_void,
        invoke: Some(skia_host_invoke),
        channel_publish: Some(skia_host_channel_publish),
    }
}

#[cfg(test)]
unsafe extern "C" fn skia_host_invoke(
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
    // SAFETY: `host_ctx` is a pinned `Arc<SkiaHostServices>` pointer from `SkiaHostServices::callbacks`.
    let svc = unsafe { &*(host_ctx as *const SkiaHostServices) };
    // SAFETY: `host_ctx` and C-ABI buffers satisfy `dispatch_ffi_invoke_with_cache`.
    unsafe {
        dispatch_ffi_invoke_with_cache::<SkiaHostServices>(
            host_ctx,
            service_id,
            method_id,
            request_id,
            request,
            out_buf,
            out_cap,
            out_len,
            Some(&svc.retry_cache),
        )
    }
}

#[cfg(test)]
unsafe extern "C" fn skia_host_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    // SAFETY: `host_ctx` is a pinned `Arc<SkiaHostServices>` pointer from `SkiaHostServices::callbacks`.
    unsafe { dispatch_ffi_channel_publish::<SkiaHostServices>(host_ctx, channel_id, flags, payload) }
}

unsafe extern "C" fn skia_slot_invoke(
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
    // SAFETY: `host_ctx` is a pinned `Arc<SkiaSlotHostBinding>` pointer from `SkiaSlotHostBinding::callbacks`.
    let binding = unsafe { &*(host_ctx as *const SkiaSlotHostBinding) };
    // SAFETY: `host_ctx` and C-ABI buffers satisfy `dispatch_ffi_invoke_with_cache`.
    unsafe {
        dispatch_ffi_invoke_with_cache::<SkiaSlotHostBinding>(
            host_ctx,
            service_id,
            method_id,
            request_id,
            request,
            out_buf,
            out_cap,
            out_len,
            Some(&binding.retry_cache),
        )
    }
}

unsafe extern "C" fn skia_slot_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    // SAFETY: `host_ctx` is a pinned `Arc<SkiaSlotHostBinding>` pointer from `SkiaSlotHostBinding::callbacks`.
    unsafe {
        dispatch_ffi_channel_publish::<SkiaSlotHostBinding>(host_ctx, channel_id, flags, payload)
    }
}
