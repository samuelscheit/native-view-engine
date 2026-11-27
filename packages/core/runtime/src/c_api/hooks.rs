//! Embedding-host entry points: native text measurement, scene-guarded resource payloads, and
//! the optional app capabilities (props, outbound events, state snapshots, hot patches).

use std::sync::Arc;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveMapEntry, NvePropsRef, NveRuntimeHandle,
    NveRuntimeMeasureTextFn, NveStrSlice,
};
use nve_engine::HotPatchMsg;
use nve_layout::TextMeasurer;
use nve_store::ResourceId;

use super::{host_bytes, required, with_runtime, write_opt, SendPtr};
use crate::builder::RuntimeError;

/// [`TextMeasurer`] calling a host `NveRuntimeMeasureTextFn` (owner thread, during layout).
struct HostTextMeasurer {
    measure: NveRuntimeMeasureTextFn,
    ctx: SendPtr,
}

/// Host sizes that are negative, NaN, or infinite collapse to `0` so layout stays finite.
fn finite_size(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

impl TextMeasurer for HostTextMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_with_weight(text, font_size, 400, max_width)
    }

    fn measure_with_weight(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let (mut width, mut height) = (0.0f32, 0.0f32);
        // SAFETY: The host guarantees `measure(ctx, ..)` stays callable until it is replaced or the
        // runtime is destroyed; `text` and both out slots outlive the synchronous call.
        unsafe {
            (self.measure)(
                self.ctx.as_ptr(),
                NveStrSlice::from_str(text),
                font_size,
                u32::from(font_weight),
                max_width.unwrap_or(f32::INFINITY),
                &raw mut width,
                &raw mut height,
            );
        }
        (finite_size(width), finite_size(height))
    }
}

pub(super) unsafe extern "C" fn rt_set_text_measurer(
    handle: NveRuntimeHandle,
    measure: Option<NveRuntimeMeasureTextFn>,
    ctx: *mut core::ffi::c_void,
) -> i32 {
    with_runtime("nve_runtime::set_text_measurer", handle, |rt| {
        let measurer = measure.map(|measure| {
            Arc::new(HostTextMeasurer {
                measure,
                ctx: SendPtr(ctx),
            }) as Arc<dyn TextMeasurer>
        });
        rt.set_text_measurer_shared(measurer)
    })
}

pub(super) unsafe extern "C" fn rt_set_props(
    handle: NveRuntimeHandle,
    entries: *const NveMapEntry,
    count: u32,
    out_changed: *mut u32,
) -> i32 {
    with_runtime("nve_runtime::set_props", handle, |rt| {
        // SAFETY: Caller guarantees `entries` holds `count` valid entries for this call.
        let props = unsafe { NvePropsRef::try_from_raw_parts(entries, count) }.ok_or(
            RuntimeError::InvalidArg("invalid NveMapEntry array (null/misaligned pointer or len overflow)"),
        )?;
        let changed = rt.set_props_shared(props)?;
        // SAFETY: `out_changed` is null or a valid caller out slot.
        unsafe { write_opt(out_changed, u32::from(changed)) };
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_drain_emitted_events(
    handle: NveRuntimeHandle,
    out_events: *mut NveEventBatch,
) -> i32 {
    with_runtime("nve_runtime::drain_emitted_events", handle, |rt| {
        // Draining is destructive: validate the slot before any events leave the app.
        let out = required(out_events, "null out_events")?;
        let events = rt.drain_emitted_events_shared()?;
        // SAFETY: `out` is a non-null caller out slot.
        unsafe { out.write(events) };
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_snapshot_state(
    handle: NveRuntimeHandle,
    out_state: *mut NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::snapshot_state", handle, |rt| {
        let out = required(out_state, "null out_state")?;
        let state = rt.snapshot_state_shared()?;
        // SAFETY: `out` is a non-null caller out slot.
        unsafe { out.write(state) };
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_restore_state(
    handle: NveRuntimeHandle,
    state: NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::restore_state", handle, |rt| {
        // SAFETY: Caller guarantees `state` is readable for `state.len` bytes when valid.
        let bytes = unsafe { host_bytes(state, "invalid state NveBufferSlice") }?;
        rt.restore_state_shared(bytes)
    })
}

pub(super) unsafe extern "C" fn rt_apply_hot_patch(
    handle: NveRuntimeHandle,
    patch_nvp1: NveBufferSlice,
    out_changed: *mut u32,
) -> i32 {
    with_runtime("nve_runtime::apply_hot_patch", handle, |rt| {
        // SAFETY: Caller guarantees `patch_nvp1` is readable for `len` bytes when valid.
        let bytes = unsafe { host_bytes(patch_nvp1, "invalid patch NveBufferSlice") }?;
        let patch = HotPatchMsg::from_bytes(bytes)
            .map_err(|_| RuntimeError::InvalidArg("malformed NVP1 hot patch frame"))?;
        let changed = rt.apply_hot_patch_shared(&patch)?;
        // SAFETY: `out_changed` is null or a valid caller out slot.
        unsafe { write_opt(out_changed, u32::from(changed)) };
        Ok(())
    })
}

pub(super) unsafe extern "C" fn rt_resource_payload(
    handle: NveRuntimeHandle,
    resource_id: u32,
    out_kind: *mut u64,
    out_payload: *mut NveBufferSlice,
) -> i32 {
    with_runtime("nve_runtime::resource_payload", handle, |rt| {
        let out = required(out_payload, "null out_payload")?;
        rt.check_not_poisoned()?;
        let _reader = rt.lock.scene_bound_read_guard().ok_or(RuntimeError::InvalidArg(
            "resource_payload requires an active scene_acquire",
        ))?;
        // SAFETY: `_reader` (and the host's own `scene_acquire`, which the returned borrow is
        // documented to be tied to) exclude writers.
        let (kind, bytes) = unsafe { rt.published_resource(ResourceId(resource_id)) }
            .ok_or(RuntimeError::NotFound("resource id is not live in the published scene"))?;
        // SAFETY: `out_kind` is null or a valid slot; `out` is a non-null caller out slot.
        unsafe {
            write_opt(out_kind, kind.0);
            out.write(NveBufferSlice::from_slice(bytes));
        }
        Ok(())
    })
}
