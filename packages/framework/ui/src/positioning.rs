//! Floating UI-grade pure-Rust anchored overlay collision-flipping, viewport clamping,
//! auto-sizing, arrow center tracking, and submenu safe-polygon hover corridor solver.
//!
//! Canonical zero-allocation implementation lives in [`nve_layout::overlay`].

pub use nve_layout::overlay::{
    Alignment, ComputedPosition, Placement, Point, PositionOptions, Rect, Side, Size, Viewport,
    compute_overlay_position, point_in_safe_polygon,
};
