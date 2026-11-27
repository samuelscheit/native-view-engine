//! [`TextMeasurer`] adapter over a host's C ABI [`NveTextMeasureFn`] (`set_text_measurer`).

use std::ffi::c_void;

use nve_abi::{NveStrSlice, NveTextMeasureFn};
use nve_layout::{HeuristicTextMeasurer, TextMeasurer};

/// Calls the host measurer, falling back to [`HeuristicTextMeasurer`] when it fails or returns a
/// non-finite / negative size.
pub(crate) struct HostTextMeasurer {
    measure: NveTextMeasureFn,
    ctx: *mut c_void,
}

// SAFETY: The host guarantees `ctx` stays valid until the measurer is replaced or the surface is
// destroyed; layout (the only caller) runs on the surface's owner thread, which the host's
// `measure` expects per the `set_text_measurer` contract.
unsafe impl Send for HostTextMeasurer {}
// SAFETY: See `Send`; the runtime never measures from two threads at once (owner-thread layout).
unsafe impl Sync for HostTextMeasurer {}

impl HostTextMeasurer {
    /// # Safety
    /// `measure` and `ctx` must follow the `NveTextMeasureFn` contract until replaced/destroyed.
    pub(crate) unsafe fn new(measure: NveTextMeasureFn, ctx: *mut c_void) -> Self {
        Self { measure, ctx }
    }

    /// `max_width` as the C contract encodes it: `INFINITY` unconstrained, `0` min-content.
    fn encode_max_width(max_width: Option<f32>) -> f32 {
        match max_width {
            None => f32::INFINITY,
            Some(w) if !w.is_finite() || w <= 0.0 => 0.0,
            Some(w) => w,
        }
    }
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
        let (mut w, mut h) = (f32::NAN, f32::NAN);
        // SAFETY: `text` outlives the call; out-pointers are valid locals; `measure`/`ctx` follow
        // the host contract (see `new`).
        let status = unsafe {
            (self.measure)(
                self.ctx,
                NveStrSlice::from_str(text),
                font_size,
                u32::from(font_weight),
                Self::encode_max_width(max_width),
                &mut w,
                &mut h,
            )
        };
        if status == 0 && w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0 {
            (w, h)
        } else {
            HeuristicTextMeasurer.measure_with_weight(text, font_size, font_weight, max_width)
        }
    }
}
