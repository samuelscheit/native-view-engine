//! Floating UI-grade pure-Rust anchored overlay collision-flipping, viewport clamping,
//! auto-sizing, arrow center tracking, and submenu safe-polygon hover corridor solver.
//!
//! Canonical zero-allocation implementation lives in [`nve_layout::overlay`].

pub use nve_layout::overlay::{
    Alignment, ComputedPosition, Placement, Point, PositionOptions, Rect, Side, Size, Viewport,
    compute_overlay_position, point_in_safe_polygon,
};

/// Canonical kebab-case placement attribute string (`"bottom-start"`, `"top"`, etc.).
#[inline]
pub const fn placement_as_str(placement: Placement) -> &'static str {
    match placement {
        Placement::Top => "top",
        Placement::TopStart => "top-start",
        Placement::TopEnd => "top-end",
        Placement::Bottom => "bottom",
        Placement::BottomStart => "bottom-start",
        Placement::BottomEnd => "bottom-end",
        Placement::Left => "left",
        Placement::LeftStart => "left-start",
        Placement::LeftEnd => "left-end",
        Placement::Right => "right",
        Placement::RightStart => "right-start",
        Placement::RightEnd => "right-end",
    }
}
