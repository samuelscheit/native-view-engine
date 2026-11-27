//! `NveRuntimeApi` slots forwarding embedded-component props/events, intrinsic measurement,
//! dev-loop hot patches and state snapshots, host text measurement, and resource reads.

use std::cell::RefCell;
use std::ffi::c_void;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveIntrinsicSize, NveMapEntry, NveMapRef, NveRuntimeHandle,
    NveRuntimeStatus, NveSizeConstraints, NveTextMeasureFn,
};
use nve_engine::HotPatchMsg;

use super::{borrow_runtime, runtime_catch_unwind, set_last_error};
use crate::builder::RuntimeError;
use crate::surface::SurfaceRuntime;

thread_local! {
    static COPY_OUT_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Shared slot body: panic containment, handle validation, and error-to-status mapping.
fn runtime_call(
    op: &'static str,
    handle: NveRuntimeHandle,
    f: impl FnOnce(&SurfaceRuntime) -> Result<(), RuntimeError>,
) -> i32 {
    runtime_catch_unwind(op, NveRuntimeStatus::Panic.as_i32(), || {
        // SAFETY: `handle` checked non-null inside `borrow_runtime`.
        let Some(rt) = (unsafe { borrow_runtime(handle) }) else {
            return NveRuntimeStatus::InvalidArg.as_i32();
        };
        match f(rt) {
            Ok(()) => NveRuntimeStatus::Ok.as_i32(),
            Err(e) => {
                set_last_error(e.to_string());
                e.status().as_i32()
            }
        }
    })
}

/// Writes `value` through an optional (nullable) out pointer.
fn write_opt<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: The caller provided a non-null, writable output slot.
        unsafe { out.write(value) };
    }
}

/// Writes `value` through a required out pointer.
fn write_required<T>(out: *mut T, value: T) -> Result<(), RuntimeError> {
    if out.is_null() {
        return Err(RuntimeError::InvalidArg("null output pointer"));
    }
    write_opt(out, value);
    Ok(())
}

pub(super) unsafe extern "C" fn rt_set_props(
    handle: NveRuntimeHandle,
    entries: *const NveMapEntry,
    count: u32,
    out_changed: *mut u32,
) -> i32 {
    runtime_call("nve_runtime::set_props", handle, |rt| {
        // SAFETY: Caller guarantees `entries..entries+count` is readable for this call.
        let props: NveMapRef<'_> = unsafe { NveMapRef::try_from_raw_parts(entries, count) }
            .ok_or(RuntimeError::InvalidArg("invalid props (null/misaligned entries or count overflow)"))?;
        let changed = rt.set_props_shared(props)?;
        write_opt(out_changed, u32::from(changed));
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_drain_emitted_events(
    handle: NveRuntimeHandle,
    out_batch: *mut NveEventBatch,
) -> i32 {
    runtime_call("nve_runtime::drain_emitted_events", handle, |rt| {
        write_required(out_batch, rt.drain_emitted_events_shared()?)
    })
}

pub(super) unsafe extern "C" fn rt_measure(
    handle: NveRuntimeHandle,
    constraints: NveSizeConstraints,
    out_size: *mut NveIntrinsicSize,
) -> i32 {
    runtime_call("nve_runtime::measure", handle, |rt| {
        write_required(out_size, rt.measure_shared(constraints)?)
    })
}

pub(super) unsafe extern "C" fn rt_apply_hot_patch(
    handle: NveRuntimeHandle,
    patch_nvp1: NveBufferSlice,
    out_changed: *mut u32,
) -> i32 {
    runtime_call("nve_runtime::apply_hot_patch", handle, |rt| {
        // SAFETY: Caller guarantees `patch_nvp1` is readable for `len` bytes when structurally valid.
        let bytes = unsafe { patch_nvp1.try_as_slice() }
            .ok_or(RuntimeError::InvalidArg("invalid NveBufferSlice (null pointer with len > 0)"))?;
        let patch = HotPatchMsg::from_bytes(bytes)
            .map_err(|_| RuntimeError::InvalidArg("malformed NVP1 hot-patch frame"))?;
        let changed = rt.apply_hot_patch_shared(&patch)?;
        write_opt(out_changed, u32::from(changed));
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_snapshot_state(
    handle: NveRuntimeHandle,
    out_state: *mut NveBufferSlice,
) -> i32 {
    runtime_call("nve_runtime::snapshot_state", handle, |rt| {
        write_required(out_state, rt.snapshot_state_shared()?)
    })
}

pub(super) unsafe extern "C" fn rt_restore_state(handle: NveRuntimeHandle, state: NveBufferSlice) -> i32 {
    runtime_call("nve_runtime::restore_state", handle, |rt| {
        // SAFETY: Caller guarantees `state` is readable for `len` bytes when structurally valid.
        let bytes = unsafe { state.try_as_slice() }
            .ok_or(RuntimeError::InvalidArg("invalid NveBufferSlice (null pointer with len > 0)"))?;
        rt.restore_state_shared(bytes)
    })
}

pub(super) unsafe extern "C" fn rt_set_text_measurer(
    handle: NveRuntimeHandle,
    cb: Option<NveTextMeasureFn>,
    ctx: *mut c_void,
) -> i32 {
    runtime_call("nve_runtime::set_text_measurer", handle, |rt| {
        // SAFETY: The ABI contract makes the host keep `ctx` valid for `cb` until replaced/destroyed.
        unsafe { rt.set_host_text_measurer(cb, ctx) }
    })
}

/// Copies the encoding of `resource_id` into `out_buf[..cap]` and reports its full length in
/// `out_len`; pass `cap = 0` to size the buffer. Returns `NVE_RUNTIME_INVALID_ARG` (with
/// `out_len` set) when `cap` is too small.
pub(super) unsafe extern "C" fn rt_resource_bytes(
    handle: NveRuntimeHandle,
    resource_id: u32,
    out_buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    runtime_call("nve_runtime::resource_bytes", handle, |rt| {
        copy_out(out_buf, cap, out_len, |scratch| rt.resource_bytes(resource_id, scratch))
    })
}

/// Copies a full `NVE2` keyframe of the current scene (instant-boot Frame-0) into
/// `out_buf[..cap]`; same sizing contract as [`rt_resource_bytes`].
pub(super) unsafe extern "C" fn rt_encode_scene(
    handle: NveRuntimeHandle,
    out_buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    runtime_call("nve_runtime::encode_scene", handle, |rt| {
        copy_out(out_buf, cap, out_len, |scratch| rt.encode_scene(scratch))
    })
}

/// Shared copy-out contract: `fill` encodes into a reused thread-local buffer, `out_len` always
/// receives the full length, `cap = 0` only sizes, a too-small buffer is `InvalidArg`.
fn copy_out(
    out_buf: *mut u8,
    cap: usize,
    out_len: *mut usize,
    fill: impl FnOnce(&mut Vec<u8>) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    COPY_OUT_SCRATCH.with_borrow_mut(|scratch| {
        scratch.clear();
        fill(scratch)?;
        write_required(out_len, scratch.len())?;
        if cap == 0 {
            return Ok(());
        }
        if out_buf.is_null() || cap < scratch.len() {
            return Err(RuntimeError::InvalidArg("output buffer too small"));
        }
        // SAFETY: `out_buf` is writable for `cap >= scratch.len()` bytes per the checks above.
        unsafe { std::ptr::copy_nonoverlapping(scratch.as_ptr(), out_buf, scratch.len()) };
        Ok(())
    })
}