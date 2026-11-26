//! O(viewport) 100,000-row virtualized 120fps list engine for **Aether Workspace**.
//!
//! Combines:
//! - Deterministic zero-allocation 100,000-row dataset synthesis (Linear-style issues & stream
//!   events generated on demand for only the visible window).
//! - Unified `nve_ui::{FenwickHeightTree, RowHeights, VirtualSlice}` geometry engine providing
//!   O(1) uniform-height slicing, 2-bit-lift O(log N) variable-height slicing, and O(log N)
//!   in-place row-height updates (with one-time uniform→variable promotion on first expansion).

mod dataset;

pub use dataset::{
    deterministic_row_height, expanded_row_height, synthesize_stream_row, StreamVirtualEngine,
    VirtualRowMeta,
};
pub use nve_ui::{FenwickHeightTree, HeightsRevision, RowHeights, VirtualSlice};

use crate::state::DensityMode;

/// Default virtual dataset size for the 100k Stream view.
pub const DEFAULT_STREAM_ROW_COUNT: usize = 100_000;

/// Default overscan row count rendered above and below the visible viewport.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Builds the deterministic 100,000-row Aether stream [`RowHeights`] table for the given density,
/// variable-height mode, and expanded row indices.
///
/// Uses O(1) zero-allocation [`RowHeights::uniform`] when fixed-height with no expanded rows, and
/// single-pass O(N) [`RowHeights::variable_from_fn`] (zero intermediate `Vec`) plus O(k log N)
/// in-place expanded-row updates otherwise.
pub fn stream_row_heights(
    count: usize,
    density: DensityMode,
    variable_heights: bool,
    expanded_rows: &[usize],
) -> RowHeights {
    let uniform_h: u32 = match density {
        DensityMode::Normal => 44,
        DensityMode::Compact => 34,
    };
    if !variable_heights && expanded_rows.is_empty() {
        return RowHeights::uniform(count, uniform_h as f32);
    }

    let exp_h = u32::from(expanded_row_height(density));
    let mut heights = RowHeights::variable_from_fn(count, |idx| {
        if variable_heights {
            u32::from(deterministic_row_height(idx, density))
        } else {
            uniform_h
        }
    });
    for &row_idx in expanded_rows {
        heights.set_row_height(row_idx, exp_h);
    }
    heights
}
