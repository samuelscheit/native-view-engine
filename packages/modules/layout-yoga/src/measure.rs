//! Aliasing-sound FFI measure context (`RawMeasureState`), panic-safe `rust_measure_callback`,
//! and CSS Flexbox §4.5 scroll-container intrinsic-size helper for [`crate::YogaLayoutEngine`].

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{NonNull, addr_of, addr_of_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_components::tables::{FlexDirection, Layout, Overflow, Scroll, ScrollAxes, TextContent, Visual};
use nve_layout::{HeuristicTextMeasurer, TextMeasurer};
use nve_store::{NodeId, NodeStore};

use crate::ffi::NveYogaMeasureHook;

const MEASURE_MODE_EXACTLY: u8 = 1;
const MEASURE_MODE_AT_MOST: u8 = 2;

pub(crate) struct MeasurePayload {
    pub(crate) text: String,
    pub(crate) font_size: f32,
    pub(crate) font_weight: u16,
    pub(crate) measurer: Arc<dyn TextMeasurer>,
    pub(crate) panicked: Arc<AtomicBool>,
}

#[repr(C)]
struct BoxedMeasureState {
    hook: NveYogaMeasureHook,
    payload: MeasurePayload,
}

/// Raw-pointer owned measure context stored beside each text `YGNodeRef`.
///
/// Allocated once via `Box::into_raw` and never re-wrapped in `Box`/`&mut BoxedMeasureState`
/// while Yoga holds `hook_ptr()` in `YGNodeSetContext`. Every Rust mutation derives its pointer
/// from `hook.user_data` (the exact same `SharedReadWrite` tag handed to Yoga), keeping both
/// Stacked Borrows and Tree Borrows sound across FFI callbacks. Reclaimed via `Box::from_raw`
/// strictly in [`Self::free_after_detach`] after Yoga's hook/context has been cleared.
pub(crate) struct RawMeasureState(NonNull<BoxedMeasureState>);

impl RawMeasureState {
    pub(crate) fn new(measurer: Arc<dyn TextMeasurer>, panicked: Arc<AtomicBool>) -> Self {
        let raw = Box::into_raw(Box::new(BoxedMeasureState {
            hook: NveYogaMeasureHook {
                callback: rust_measure_callback,
                user_data: std::ptr::null_mut(),
            },
            payload: MeasurePayload {
                text: String::new(),
                font_size: HeuristicTextMeasurer::DEFAULT_FONT_SIZE,
                font_weight: 400,
                measurer,
                panicked,
            },
        }));
        // SAFETY: `raw` is a fresh unique heap allocation; deriving `payload_ptr` via
        // `addr_of_mut!` after `Box::into_raw` avoids creating any `&mut BoxedMeasureState`.
        unsafe {
            let payload_ptr = addr_of_mut!((*raw).payload).cast::<c_void>();
            addr_of_mut!((*raw).hook.user_data).write(payload_ptr);
            Self(NonNull::new_unchecked(raw))
        }
    }

    #[inline]
    pub(crate) fn hook_ptr(&self) -> *mut NveYogaMeasureHook {
        // SAFETY: `self.0` points to a live allocation; `addr_of_mut!` only projects to `hook`.
        unsafe { addr_of_mut!((*self.0.as_ptr()).hook) }
    }

    /// Runs `f` on the payload while Yoga is NOT inside `calculate_layout`.
    #[inline]
    pub(crate) fn with_payload_mut<R>(&mut self, f: impl FnOnce(&mut MeasurePayload) -> R) -> R {
        // SAFETY: No Yoga layout call is active (`&mut self` on `YogaLayoutEngine`). Reading
        // `hook.user_data` via `addr_of!` and deriving `&mut MeasurePayload` from it preserves
        // `user_data`'s `SharedReadWrite` tag in Stacked Borrows without touching `hook`.
        let payload = unsafe {
            let ptr = addr_of!((*self.0.as_ptr()).hook.user_data).read();
            &mut *(ptr.cast::<MeasurePayload>())
        };
        f(payload)
    }

    /// Reclaims the allocation ONLY after Yoga's measure hook/context has been detached.
    pub(crate) unsafe fn free_after_detach(self) {
        // SAFETY: Caller guarantees Yoga no longer references `hook` or `user_data`.
        unsafe { drop(Box::from_raw(self.0.as_ptr())) };
    }
}

extern "C" fn rust_measure_callback(
    user_data: *mut c_void,
    width: f32,
    width_mode: u8,
    height: f32,
    height_mode: u8,
    out_width: *mut f32,
    out_height: *mut f32,
) {
    if user_data.is_null() || out_width.is_null() || out_height.is_null() {
        return;
    }
    // SAFETY: `user_data` points to `MeasurePayload` inside a live `RawMeasureState` owned by
    // the attached `YGNode`, and no mutable access runs concurrently during `calculate_layout`.
    let payload = unsafe { &*(user_data.cast::<MeasurePayload>()) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let max_width = match width_mode {
            MEASURE_MODE_EXACTLY | MEASURE_MODE_AT_MOST if width.is_finite() => {
                Some(width.max(0.0))
            }
            _ => None,
        };
        let (measured_w, measured_h) = payload.measurer.measure_with_weight(
            &payload.text,
            payload.font_size,
            payload.font_weight,
            max_width,
        );
        let final_w = match width_mode {
            MEASURE_MODE_EXACTLY if width.is_finite() => width.max(0.0),
            _ => measured_w,
        };
        let final_h = match height_mode {
            MEASURE_MODE_EXACTLY if height.is_finite() => height.max(0.0),
            _ => measured_h,
        };
        (final_w, final_h)
    }));
    let (w, h) = match result {
        Ok(dims) => dims,
        Err(_) => {
            payload.panicked.store(true, Ordering::Release);
            (0.0, 0.0)
        }
    };
    // SAFETY: `out_width` and `out_height` are non-null stack pointers passed by
    // `nve_yoga_measure_trampoline` in `yoga_shim.cpp`.
    unsafe {
        *out_width = w;
        *out_height = h;
    }
}

/// Returns `true` when `id` is an auto-sized, visible-overflow child along a scroll container's
/// scrolling main axis. Per CSS Flexbox §4.5, such an item's automatic minimum size equals its
/// wrapped content size so it must not shrink inside the scroll viewport; Yoga's vertical
/// min-content walk omits cross-axis wrapping and gaps, so we preserve CSS semantics via
/// `flex_shrink = 0.0` (matching React Native `<ScrollView>` content containers).
pub(crate) fn preserve_scroll_child_intrinsic_size(store: &NodeStore, id: NodeId) -> bool {
    let Some(parent) = store.parent(id) else {
        return false;
    };
    let p_dir = store
        .try_row::<Layout>(parent)
        .map_or(FlexDirection::Column, |l| l.flex_direction());
    let is_col = matches!(p_dir, FlexDirection::Column | FlexDirection::ColumnReverse);
    // Avoid disabling shrink on wrapping text children inside horizontal `Row` containers.
    if !is_col && store.has::<TextContent>(id) {
        return false;
    }
    let has_scroll_table = store.try_row::<Scroll>(parent).is_some_and(|s| {
        match s.axes() {
            ScrollAxes::Vertical => is_col,
            ScrollAxes::Horizontal => !is_col,
            ScrollAxes::Both => true,
            ScrollAxes::None => false,
        }
    });
    let scroll_main = has_scroll_table
        || store.try_row::<Visual>(parent).is_some_and(|p_vis| {
            if is_col {
                matches!(p_vis.overflow_y(), Overflow::Scroll | Overflow::Auto)
            } else {
                matches!(p_vis.overflow_x(), Overflow::Scroll | Overflow::Auto)
            }
        });
    if !scroll_main {
        return false;
    }
    let child_overflow_visible = store.try_row::<Visual>(id).is_none_or(|v| {
        if is_col {
            v.overflow_y() == Overflow::Visible
        } else {
            v.overflow_x() == Overflow::Visible
        }
    });
    if !child_overflow_visible {
        return false;
    }
    let Some(l) = store.try_row::<Layout>(id) else {
        return true;
    };
    let ar = l.aspect_ratio();
    if ar.is_finite() && ar > 0.0 {
        return false;
    }
    let (main_size, min_main) = if is_col {
        (l.height(), l.min_height())
    } else {
        (l.width(), l.min_width())
    };
    main_size.is_auto() && l.flex_basis().is_auto() && min_main.is_auto()
}
