//! Runtime ABI v6 host & measurement C ABI entry points:
//! `measure_surface`, `set_intrinsic_size_invalidated_callback`, `set_host_callbacks`,
//! `complete_host_request`, `push_host_event`, and `publish_host_channel`.

use std::ffi::c_void;
use std::sync::Arc;

use nve_abi::{
    HostRequestId, HostServiceId, NveBufferSlice, NveHostCallbacks, NveHostStatus,
    NveRuntimeHandle, NveRuntimeIntrinsicSizeInvalidatedFn, NveSizeConstraints, NveSurfaceMeasure,
};

use super::{host_bytes, required, with_runtime, SendPtr};
use crate::builder::RuntimeError;

pub(super) unsafe extern "C" fn rt_measure_surface(
    handle: NveRuntimeHandle,
    constraints: *const NveSizeConstraints,
    flags: u32,
    out_measure: *mut NveSurfaceMeasure,
) -> i32 {
    with_runtime("nve_runtime::measure_surface", handle, |rt| {
        if constraints.is_null() {
            return Err(RuntimeError::InvalidArg("null constraints"));
        }
        let out = required(out_measure, "null out_measure")?;
        // SAFETY: `constraints` checked non-null above and readable for this call.
        let c = unsafe { *constraints };
        let measured = rt.measure_surface_shared(c, flags)?;
        // SAFETY: `out` is a non-null caller out slot.
        unsafe { out.write(measured) };
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_set_intrinsic_size_invalidated_callback(
    handle: NveRuntimeHandle,
    cb: Option<NveRuntimeIntrinsicSizeInvalidatedFn>,
    ctx: *mut c_void,
) -> i32 {
    with_runtime(
        "nve_runtime::set_intrinsic_size_invalidated_callback",
        handle,
        |rt| {
            let callback = cb.map(|func| {
                let send_ctx = SendPtr(ctx);
                Arc::new(move || {
                    // SAFETY: Host guarantees `func(ctx)` stays valid until unregistered with
                    // `NULL` or `destroy`; `IntrinsicSizeNotifier` drains in-flight calls under
                    // its `RwLock`.
                    unsafe { func(send_ctx.as_ptr()) };
                }) as Arc<dyn Fn() + Send + Sync>
            });
            rt.set_intrinsic_size_invalidated_callback_shared(callback)
        },
    )
}

pub(super) unsafe extern "C" fn rt_set_host_callbacks(
    handle: NveRuntimeHandle,
    callbacks: *const NveHostCallbacks,
) -> i32 {
    with_runtime("nve_runtime::set_host_callbacks", handle, |rt| {
        let cb = if callbacks.is_null() {
            None
        } else {
            // SAFETY: Non-null `callbacks` points to a readable `NveHostCallbacks`.
            Some(unsafe { *callbacks })
        };
        rt.set_host_callbacks_shared(cb)
    })
}

pub(super) unsafe extern "C" fn rt_complete_host_request(
    handle: NveRuntimeHandle,
    request_id: HostRequestId,
    status: i32,
    payload: NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::complete_host_request", handle, |rt| {
        // SAFETY: Caller guarantees `payload` is readable for `payload.len` bytes when valid.
        let bytes = unsafe { host_bytes(payload, "invalid host completion payload NveBufferSlice") }?;
        let _ = rt.complete_host_request(request_id, NveHostStatus::from_i32(status), bytes)?;
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_push_host_event(
    handle: NveRuntimeHandle,
    service_id: HostServiceId,
    event_id: u32,
    payload: NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::push_host_event", handle, |rt| {
        // SAFETY: Caller guarantees `payload` is readable for `payload.len` bytes when valid.
        let bytes = unsafe { host_bytes(payload, "invalid host push payload NveBufferSlice") }?;
        rt.push_host_event(service_id, event_id, bytes)
    })
}

pub(super) unsafe extern "C" fn rt_publish_host_channel(
    handle: NveRuntimeHandle,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::publish_host_channel", handle, |rt| {
        // SAFETY: Caller guarantees `payload` is readable for `payload.len` bytes when valid.
        let bytes = unsafe { host_bytes(payload, "invalid host channel payload NveBufferSlice") }?;
        rt.publish_host_channel(channel_id, flags, bytes)
    })
}
