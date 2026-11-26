//! Heap-pinned C-ABI bridge exposing any [`HostServiceProvider`] as [`NveHostCallbacks`]
//! for `NVE_IFACE_HOST = 5` (`NveHostApi::bind_host`).

use std::ffi::c_void;

use nve_abi::{NveBufferSlice, NveHostCallbacks, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN};

use crate::provider::{HostServiceProvider, InvokeResult};

/// Heap-pinned C-ABI bridge exposing any [`HostServiceProvider`] as [`NveHostCallbacks`]
/// suitable for `NveHostApi::bind_host`.
pub struct HostCallbackBridge<P: HostServiceProvider> {
    provider: Box<P>,
}

impl<P: HostServiceProvider> HostCallbackBridge<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider: Box::new(provider),
        }
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Returns a C-ABI [`NveHostCallbacks`] valid for as long as `self` remains alive and unmoved.
    pub fn callbacks(&self) -> NveHostCallbacks {
        let ptr: *const P = &*self.provider;
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
    _request_id: u64,
    request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    if host_ctx.is_null() {
        return NveHostStatus::InvalidArg.as_i32();
    }
    // SAFETY: `host_ctx` is a non-null pinned `Box<P>` pointer from `HostCallbackBridge::callbacks`;
    // `request` is guaranteed valid for the duration of the FFI callback.
    let (provider, req_bytes) = unsafe { (&*(host_ctx as *const P), request.as_slice()) };
    match provider.invoke_raw(service_id, method_id, req_bytes) {
        Ok(InvokeResult::Ready(bytes)) => {
            if !out_len.is_null() {
                // SAFETY: Caller provided a valid writable `out_len` pointer.
                unsafe { *out_len = bytes.len() };
            }
            if bytes.len() > out_cap {
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
    // SAFETY: `host_ctx` is a non-null pinned `Box<P>` pointer from `HostCallbackBridge::callbacks`;
    // `payload` is valid for the duration of the FFI callback.
    let (provider, bytes) = unsafe { (&*(host_ctx as *const P), payload.as_slice()) };
    let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
    match provider.channel_publish(channel_id, retain, bytes) {
        Ok(()) => NveHostStatus::Ok.as_i32(),
        Err(err) => err.to_status().as_i32(),
    }
}
