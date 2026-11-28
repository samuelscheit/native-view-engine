//! Taffy-vs-Yoga rect comparison within [`PARITY_TOLERANCE_PX`].

use nve_engine::LayoutRect;
use nve_store::NodeId;

pub const PARITY_TOLERANCE_PX: f32 = 1.0;

pub fn assert_rect_parity(label: &str, id: NodeId, taffy: LayoutRect, yoga: LayoutRect) {
    assert!(
        (taffy.x - yoga.x).abs() <= PARITY_TOLERANCE_PX
            && (taffy.y - yoga.y).abs() <= PARITY_TOLERANCE_PX
            && (taffy.width - yoga.width).abs() <= PARITY_TOLERANCE_PX
            && (taffy.height - yoga.height).abs() <= PARITY_TOLERANCE_PX,
        "[{label}] node #{} diverged beyond {PARITY_TOLERANCE_PX}px:\n  taffy: {taffy:?}\n  yoga : {yoga:?}",
        id.raw()
    );
}
