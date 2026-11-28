//! Pure dry-run `YogaLayoutEngine::measure_root` verification:
//! - Preserves committed rects, `layout_dirty`, and clean-frame caching (`compute_layout` -> `false`)
//! - `ideal = true` measures unconstrained (never clamps `YGUndefined` to `0.0`)
//! - Enforces `min_w`/`min_h` and `max_w`/`max_h` during layout (re-wrapping height accurately)
//! - Works before the first `compute_layout` (`point_scale_factor` initialized, dirty state intact)
//! - Rejects invalid constraints (`-1`, `-Inf`, `NaN`, `min > max`) and isolates panicking
//!   `TextMeasurer` without poisoning the live `YogaLayoutEngine`.

#[path = "support/require_yoga.rs"]
mod require_yoga;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_engine::Viewport;
use nve_layout::{SizeConstraints, StoreLayoutEngine, TextMeasurer, TextStyle};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};
use nve_tables::tables::{FlexDirection, Layout, TextContent};
use require_yoga::require_yoga_or_skip;

const VP: Viewport = Viewport {
    width: 400.0,
    height: 300.0,
    scale_factor: 2.0,
};

struct WrappingCharMeasurer;

impl TextMeasurer for WrappingCharMeasurer {
    fn measure(&self, text: &str, _style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        let natural_w = (text.len() as f32) * 10.0;
        let line_h = 20.0;
        match max_width {
            Some(limit) if limit.is_finite() && limit > 0.0 && natural_w > limit => {
                let chars_per_line = (limit / 10.0).floor().max(1.0);
                let lines = (text.len() as f32 / chars_per_line).ceil().max(1.0);
                (chars_per_line * 10.0, lines * line_h)
            }
            _ => (natural_w, line_h),
        }
    }
}

struct TogglePanicMeasurer(Arc<AtomicBool>);

impl TextMeasurer for TogglePanicMeasurer {
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        if self.0.load(Ordering::Acquire) {
            #[expect(clippy::panic, reason = "synthetic yoga dry-run measurer panic test")]
            {
                panic!("synthetic yoga dry-run text measurer panic");
            }
        }
        WrappingCharMeasurer.measure(text, style, max_width)
    }
}

fn build_wrapping_tree() -> (NodeStore, ChangeSet, NodeId) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let label = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .flex_direction(FlexDirection::Column);
        let label = tx.create();
        // 20 chars -> natural 200x20; at max_w=100 wraps to 2 lines (100x40).
        tx.edit::<TextContent>(label).text("12345678901234567890");
        tx.set_children(NodeId::ROOT, &[label]);
        tx.commit_into(&mut changes);
        label
    };
    (store, changes, label)
}

#[test]
fn yoga_measure_root_preserves_clean_frame_cache_and_handles_ideal_and_min_max() {
    if !require_yoga_or_skip() {
        return;
    }
    let (store, changes, label) = build_wrapping_tree();
    let mut yoga =
        YogaLayoutEngine::with_text_measurer(Arc::new(WrappingCharMeasurer)).expect("real Yoga");
    yoga.sync_changes(&store, &changes);

    // 1. `measure_root` BEFORE first `compute_layout`: accurate size AND keeps live engine dirty!
    let pre = yoga.measure_root(
        &store,
        NodeId::ROOT,
        SizeConstraints::loose(100.0, f32::INFINITY),
        false,
    );
    assert!(
        (pre.width - 100.0).abs() <= 1.0 && (pre.height - 40.0).abs() <= 1.0,
        "pre-layout wrapped measure expected ~100x40, got {pre:?}"
    );
    assert!(
        yoga.compute_layout(&store, NodeId::ROOT, VP),
        "first compute_layout must still run after pre-layout measure_root"
    );
    let committed_root = yoga.layout_of(NodeId::ROOT);
    let committed_label = yoga.layout_of(label);

    // 2. `ideal = true` measures unconstrained (200x20, NEVER collapsed to 0.0!).
    let ideal = yoga.measure_root(
        &store,
        NodeId::ROOT,
        SizeConstraints {
            min_w: 50.0,
            min_h: 10.0,
            max_w: 50.0,
            max_h: 10.0,
        },
        true,
    );
    assert!(
        (ideal.width - 200.0).abs() <= 1.0 && (ideal.height - 20.0).abs() <= 1.0,
        "ideal measure must return unconstrained natural size ~200x20, got {ideal:?}"
    );

    // 3. Constrained wrapping + `0 < min_w < max_w` enforcement during layout.
    let wrapped = yoga.measure_root(
        &store,
        NodeId::ROOT,
        SizeConstraints {
            min_w: 60.0,
            min_h: 30.0,
            max_w: 100.0,
            max_h: 200.0,
        },
        false,
    );
    assert!(
        (60.0..=100.0).contains(&wrapped.width) && (wrapped.height - 40.0).abs() <= 1.0,
        "wrapped measure expected width in 60..=100 and height ~40, got {wrapped:?}"
    );

    // 4. Clean-frame caching purity: committed rects untouched & next `compute_layout` is `false`!
    assert_eq!(yoga.layout_of(NodeId::ROOT), committed_root);
    assert_eq!(yoga.layout_of(label), committed_label);
    assert!(
        !yoga.compute_layout(&store, NodeId::ROOT, VP),
        "measure_root must NOT bust Yoga clean-frame caching"
    );
}

#[test]
fn yoga_measure_root_enforces_min_width_aspect_ratio_and_isolates_invalid_or_panics() {
    if !require_yoga_or_skip() {
        return;
    }
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let child = {
        let mut tx = store.tx();
        let child = tx.create();
        // Child fills root width (`100%`) with `aspect_ratio = 2.0`: at width 120 -> height 60.
        tx.edit::<Layout>(child)
            .width(Length::percent(100.0))
            .aspect_ratio(Some(2.0));
        tx.set_children(NodeId::ROOT, &[child]);
        tx.commit_into(&mut changes);
        child
    };
    let mut yoga = YogaLayoutEngine::new().expect("real Yoga");
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, VP));

    let min_enforced = yoga.measure_root(
        &store,
        NodeId::ROOT,
        SizeConstraints {
            min_w: 120.0,
            min_h: 0.0,
            max_w: 300.0,
            max_h: 300.0,
        },
        false,
    );
    assert!(
        (min_enforced.width - 120.0).abs() <= 1.0 && (min_enforced.height - 60.0).abs() <= 1.0,
        "0 < min_w < max_w must lay out child at min_w=120 -> height 60, got {min_enforced:?}"
    );
    assert!(!yoga.compute_layout(&store, NodeId::ROOT, VP));
    let _ = child;

    // Invalid constraints return zeroed empty measure without touching live engine.
    for bad in [
        SizeConstraints {
            min_w: -1.0,
            min_h: 0.0,
            max_w: 100.0,
            max_h: 100.0,
        },
        SizeConstraints {
            min_w: 0.0,
            min_h: f32::NEG_INFINITY,
            max_w: 100.0,
            max_h: 100.0,
        },
        SizeConstraints {
            min_w: f32::NAN,
            min_h: 0.0,
            max_w: 100.0,
            max_h: 100.0,
        },
        SizeConstraints {
            min_w: 150.0,
            min_h: 0.0,
            max_w: 100.0,
            max_h: 100.0,
        },
    ] {
        let m = yoga.measure_root(&store, NodeId::ROOT, bad, false);
        assert_eq!((m.width, m.height), (0.0, 0.0));
        assert!(m.first_baseline.is_nan() && m.last_baseline.is_nan());
    }

    // Panicking `TextMeasurer` inside `measure_root` must NOT poison the live `YogaLayoutEngine`.
    let (txt_store, txt_changes, txt_label) = build_wrapping_tree();
    let should_panic = Arc::new(AtomicBool::new(false));
    let mut panic_yoga = YogaLayoutEngine::with_text_measurer(Arc::new(TogglePanicMeasurer(
        Arc::clone(&should_panic),
    )))
    .expect("real Yoga");
    panic_yoga.sync_changes(&txt_store, &txt_changes);
    assert!(panic_yoga.compute_layout(&txt_store, NodeId::ROOT, VP));
    let good_rect = panic_yoga.layout_of(txt_label);

    should_panic.store(true, Ordering::Release);
    let _ = panic_yoga.measure_root(
        &txt_store,
        NodeId::ROOT,
        SizeConstraints::loose(100.0, 100.0),
        false,
    );
    should_panic.store(false, Ordering::Release);
    assert_eq!(panic_yoga.layout_of(txt_label), good_rect);
    assert!(
        !panic_yoga.compute_layout(&txt_store, NodeId::ROOT, VP),
        "panicking dry-run measurer must not poison live YogaLayoutEngine"
    );
}
