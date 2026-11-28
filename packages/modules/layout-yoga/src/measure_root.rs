//! Pure scratch-tree `measure_root` implementation for [`crate::YogaLayoutEngine`].
//!
//! Runs dry-run sizing on an isolated scratch Yoga tree so committed `nodes`, `computed_rects`,
//! `layout_dirty`, `last_pass`, clean-frame caching, and `measurer_panicked` on the live engine
//! remain completely untouched.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nve_layout::{SizeConstraints, SurfaceMeasure};
use nve_store::{NodeId, NodeStore};

use crate::ffi::{nve_yg_config_set_point_scale_factor, nve_yg_node_measure_layout};
use crate::real::YogaLayoutEngine;

#[inline]
fn valid_constraints(c: SizeConstraints) -> bool {
    c.min_w.is_finite()
        && c.min_w >= 0.0
        && c.min_h.is_finite()
        && c.min_h >= 0.0
        && !c.max_w.is_nan()
        && c.max_w >= c.min_w
        && !c.max_h.is_nan()
        && c.max_h >= c.min_h
}

pub(crate) fn measure_root_dry_run(
    engine: &YogaLayoutEngine,
    store: &NodeStore,
    root: NodeId,
    constraints: SizeConstraints,
    ideal: bool,
) -> SurfaceMeasure {
    let empty = SurfaceMeasure {
        width: 0.0,
        height: 0.0,
        first_baseline: f32::NAN,
        last_baseline: f32::NAN,
    };
    if !store.is_live(root) || !valid_constraints(constraints) {
        return empty;
    }
    let Ok(mut scratch) = YogaLayoutEngine::with_text_measurer(Arc::clone(&engine.text_measurer))
    else {
        return empty;
    };
    let scale = engine.last_pass.map_or(1.0, |(_, _, _, s)| s);
    // SAFETY: `scratch.config` is a live owned `YGConfigRef`; initializes scale factor even when
    // `measure_root` runs before the first `compute_layout`.
    unsafe {
        nve_yg_config_set_point_scale_factor(scratch.config, scale);
    }
    scratch.sync_subtree_if_unseen(store, root);
    scratch.typo_dirty = true;
    scratch.propagate_typography(store, root);
    let Some(root_yg) = scratch.nodes.get(&root).map(|e| e.yg_node) else {
        return empty;
    };
    scratch.measurer_panicked.store(false, Ordering::Release);
    // SAFETY: `root_yg` and all descendants live inside `scratch` for the duration of this call;
    // `scratch` is dropped immediately after so any caught measurer panic never poisons `engine`.
    let raw = unsafe {
        nve_yg_node_measure_layout(
            root_yg,
            constraints.min_w,
            constraints.max_w,
            constraints.min_h,
            constraints.max_h,
            u8::from(ideal),
        )
    };
    let _panicked = scratch.measurer_panicked.swap(false, Ordering::AcqRel);
    let clamp = |v: f32, lo: f32, hi: f32| {
        let f = if v.is_finite() && v >= 0.0 { v } else { 0.0 };
        if ideal {
            f
        } else {
            let capped = if hi.is_finite() { f.min(hi) } else { f };
            capped.max(lo)
        }
    };
    SurfaceMeasure {
        width: clamp(raw.width, constraints.min_w, constraints.max_w),
        height: clamp(raw.height, constraints.min_h, constraints.max_h),
        first_baseline: f32::NAN,
        last_baseline: f32::NAN,
    }
}
