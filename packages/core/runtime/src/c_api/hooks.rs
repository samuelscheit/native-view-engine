//! Embedding-host entry points: native text measurement, scene-guarded resource payloads, and
//! the optional app capabilities (props, outbound events, state snapshots, hot patches).

use std::sync::Arc;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveMapEntry, NvePropsRef, NveRuntimeHandle,
    NveRuntimeMeasureTextFn, NveRuntimeTextStyle, NveStrSlice, NVE_LINE_HEIGHT_AUTO,
    NVE_LINE_HEIGHT_PERCENT, NVE_LINE_HEIGHT_POINTS,
};
use nve_engine::HotPatchMsg;
use nve_layout::{TextMeasurer, TextStyle};
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

/// The C view of a resolved style: weights inside CSS `100..=900`, a line height that is
/// finite and positive or `auto`, finite letter spacing.
fn abi_text_style(style: &TextStyle<&str>) -> NveRuntimeTextStyle {
    let lh = style.line_height;
    let (line_height, line_height_unit) = match lh.value() {
        v if lh.is_auto() || !v.is_finite() || v <= 0.0 => (0.0, NVE_LINE_HEIGHT_AUTO),
        v if lh.is_percent() => (v, NVE_LINE_HEIGHT_PERCENT),
        v => (v, NVE_LINE_HEIGHT_POINTS),
    };
    NveRuntimeTextStyle {
        font_size: style.font_size,
        letter_spacing: if style.letter_spacing.is_finite() { style.letter_spacing } else { 0.0 },
        line_height,
        max_lines: style.max_lines,
        font_weight: style.font_weight.clamp(100, 900),
        line_height_unit,
        font_style: style.font_style as u8,
        white_space: style.white_space as u8,
        word_break: style.word_break as u8,
        overflow_wrap: style.overflow_wrap as u8,
        _reserved: 0,
        font_family: NveStrSlice::from_str(style.font_family),
    }
}

impl TextMeasurer for HostTextMeasurer {
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        // Degenerate sizes measure as empty without a host round trip.
        if !style.font_size.is_finite() || style.font_size <= 0.0 {
            return (0.0, 0.0);
        }
        let abi_style = abi_text_style(style);
        // `nowrap` / `pre` never wrap: the host gets an unconstrained width (also for the
        // min-content probe) whatever its own `white_space` handling.
        let max_width = max_width.filter(|_| style.wraps()).unwrap_or(f32::INFINITY);
        let (mut width, mut height) = (0.0f32, 0.0f32);
        // SAFETY: The host guarantees `measure(ctx, ..)` stays callable until it is replaced or the
        // runtime is destroyed; `text`, `abi_style` (and its family), and both out slots outlive
        // the synchronous call.
        unsafe {
            (self.measure)(
                self.ctx.as_ptr(),
                NveStrSlice::from_str(text),
                &raw const abi_style,
                max_width,
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// What the host saw: size, weight, line height (value, unit), letter spacing, family,
    /// max width.
    type Seen = (f32, u16, f32, u8, f32, String, f32);
    static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

    unsafe extern "C" fn record(
        _ctx: *mut core::ffi::c_void,
        _text: NveStrSlice,
        style: *const NveRuntimeTextStyle,
        max_width: f32,
        out_width: *mut f32,
        out_height: *mut f32,
    ) {
        // SAFETY: The runtime passes a live style whose family is borrowed for the call.
        let s = unsafe { &*style };
        // SAFETY: As above.
        let family = unsafe { s.font_family.as_str() }.unwrap().to_owned();
        SEEN.lock().unwrap().push((
            s.font_size,
            s.font_weight,
            s.line_height,
            s.line_height_unit,
            s.letter_spacing,
            family,
            max_width,
        ));
        // SAFETY: Runtime-owned out slots.
        unsafe {
            *out_width = 5.0;
            *out_height = 6.0;
        }
    }

    #[test]
    fn host_measurer_sanitizes_the_style_and_skips_degenerate_sizes() {
        use nve_components::tables::WhiteSpace;
        use nve_store::Length;

        let m = HostTextMeasurer {
            measure: record,
            ctx: SendPtr(core::ptr::null_mut()),
        };
        for size in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(m.measure("x", &TextStyle::sized(size), None), (0.0, 0.0));
        }
        assert!(SEEN.lock().unwrap().is_empty(), "degenerate sizes never reach the host");
        let base = TextStyle { font_family: "Mono", ..TextStyle::sized(12.0) };
        let cases = [
            TextStyle { font_weight: 0, letter_spacing: f32::NAN, ..base },
            TextStyle { font_weight: 1000, line_height: Length::points(-3.0), ..base },
            TextStyle { font_weight: 650, line_height: Length::percent(150.0), ..base },
            TextStyle { line_height: Length::points(20.0), letter_spacing: 1.5, ..base },
            TextStyle { white_space: WhiteSpace::NoWrap, ..base },
        ];
        for style in &cases {
            assert_eq!(m.measure("x", style, Some(10.0)), (5.0, 6.0));
        }
        let inf = f32::INFINITY;
        let mono = || "Mono".to_owned();
        assert_eq!(
            *SEEN.lock().unwrap(),
            [
                (12.0, 100, 0.0, NVE_LINE_HEIGHT_AUTO, 0.0, mono(), 10.0),
                (12.0, 900, 0.0, NVE_LINE_HEIGHT_AUTO, 0.0, mono(), 10.0),
                (12.0, 650, 150.0, NVE_LINE_HEIGHT_PERCENT, 0.0, mono(), 10.0),
                (12.0, 400, 20.0, NVE_LINE_HEIGHT_POINTS, 1.5, mono(), 10.0),
                (12.0, 400, 0.0, NVE_LINE_HEIGHT_AUTO, 0.0, mono(), inf), // nowrap: unconstrained
            ]
        );
    }
}
