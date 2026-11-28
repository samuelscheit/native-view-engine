//! Aliasing-sound FFI measure context (`RawMeasureState`), panic-safe `rust_measure_callback`,
//! and CSS Flexbox §4.5 scroll-container intrinsic-size helper for [`crate::YogaLayoutEngine`].

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{NonNull, addr_of, addr_of_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_tables::tables::{
    FlexDirection, Layout, Overflow, Scroll, ScrollAxes, TextContent, Typography, Visual,
};
use nve_layout::{MAX_TREE_DEPTH, TextMeasurer, TextStyle};
use nve_store::{NodeId, NodeStore, NveTable};

use crate::ffi::{
    NveYogaMeasureHook, YGNodeRef, nve_yg_node_detach_and_clear, nve_yg_node_free,
    nve_yg_node_set_measure_hook,
};

pub(crate) struct YogaNodeEntry {
    pub(crate) yg_node: YGNodeRef,
    pub(crate) is_text: bool,
    pub(crate) measure_attached: bool,
    pub(crate) shrink_preserved: bool,
    pub(crate) synced_epoch: u32,
    pub(crate) children_synced_epoch: u32,
    pub(crate) measure_state: Option<RawMeasureState>,
}

impl YogaNodeEntry {
    pub(crate) fn new(yg_node: YGNodeRef) -> Self {
        Self {
            yg_node,
            is_text: false,
            measure_attached: false,
            shrink_preserved: false,
            synced_epoch: 0,
            children_synced_epoch: 0,
            measure_state: None,
        }
    }

    pub(crate) fn attach_measure_hook(
        &mut self,
        measurer: &Arc<dyn TextMeasurer>,
        panicked: &Arc<AtomicBool>,
    ) {
        let state = self
            .measure_state
            .get_or_insert_with(|| RawMeasureState::new(Arc::clone(measurer), Arc::clone(panicked)));
        let hook_ptr = state.hook_ptr();
        // SAFETY: `self.yg_node` is a live owned Yoga node and `hook_ptr` points into the
        // heap-allocated `RawMeasureState` owned by `self` (detached before free).
        self.measure_attached = unsafe { nve_yg_node_set_measure_hook(self.yg_node, hook_ptr) };
    }

    pub(crate) fn detach_and_free_measure_state(&mut self) {
        // SAFETY: `self.yg_node` is a live owned Yoga node; clearing hook before freeing
        // `measure_state` ensures Yoga never dereferences freed user_data.
        unsafe {
            let _ = nve_yg_node_set_measure_hook(self.yg_node, std::ptr::null_mut());
        }
        self.measure_attached = false;
        if let Some(state) = self.measure_state.take() {
            // SAFETY: Hook and context were cleared above.
            unsafe { state.free_after_detach() };
        }
    }

    pub(crate) fn detach_and_free_node(mut self) {
        // SAFETY: `self.yg_node` is exclusively owned, detached before free, and freed before
        // `self.measure_state` is reclaimed.
        unsafe {
            nve_yg_node_detach_and_clear(self.yg_node);
            nve_yg_node_free(self.yg_node);
        }
        if let Some(state) = self.measure_state.take() {
            // SAFETY: `yg_node` was detached and freed above.
            unsafe { state.free_after_detach() };
        }
    }
}

/// Computes `id`'s depth from the root (`None` if depth exceeds [`MAX_TREE_DEPTH`]).
#[inline]
pub(crate) fn node_depth_within_limit(store: &NodeStore, id: NodeId) -> Option<usize> {
    let mut depth = 0usize;
    let mut cur = store.parent(id);
    while let Some(p) = cur {
        depth += 1;
        if depth > MAX_TREE_DEPTH {
            return None;
        }
        cur = store.parent(p);
    }
    Some(depth)
}

/// Bitmask of `Typography` table words whose effects include `LAYOUT` or `MEASURE`.
#[inline]
pub(crate) fn typography_measure_word_mask(layout_effect: u8) -> u32 {
    Typography::DESC
        .word_effects
        .iter()
        .enumerate()
        .fold(0u32, |acc, (w, &fx)| {
            if fx & layout_effect != 0 {
                acc | (1u32 << w)
            } else {
                acc
            }
        })
}

const MEASURE_MODE_EXACTLY: u8 = 1;
const MEASURE_MODE_AT_MOST: u8 = 2;

pub(crate) struct MeasurePayload {
    pub(crate) text: String,
    /// The node's resolved typography (owned family: the store may change between passes).
    pub(crate) style: TextStyle<String>,
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
                style: TextStyle::default(),
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
        let (measured_w, measured_h) =
            payload
                .measurer
                .measure(&payload.text, &payload.style.borrowed(), max_width);
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
    if l.aspect_ratio().is_some_and(|ar| ar.is_finite() && ar > 0.0) {
        return false;
    }
    let (main_size, min_main) = if is_col {
        (l.height(), l.min_height())
    } else {
        (l.width(), l.min_width())
    };
    main_size.is_auto() && l.flex_basis().is_auto() && min_main.is_auto()
}
