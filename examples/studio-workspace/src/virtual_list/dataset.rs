//! Deterministic zero-allocation 100,000-row dataset synthesis and cached `StreamVirtualEngine`.

use crate::state::{DensityMode, Priority, TaskStatus};

use super::{stream_row_heights, uniform_row_height, RowHeights, VirtualSlice};

/// Zero-allocation deterministic metadata for a single row in the 100,000-row Aether stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualRowMeta {
    pub index: usize,
    pub issue_number: u32,
    pub author: &'static str,
    pub tag: &'static str,
    pub verb: &'static str,
    pub subject: &'static str,
    pub detail: &'static str,
    pub priority: Priority,
    pub status: TaskStatus,
    pub latency_us: u16,
    pub height_px: u32,
    pub is_expanded: bool,
}

const STREAM_AUTHORS: &[&str] = &["@maya", "@liam", "@sara", "@aria", "@noah", "@dev"];
const STREAM_TAGS: &[&str] = &[
    "ABI", "Layout", "Fabric", "Skia", "WASM", "Sync", "HotSwap", "Interop",
];
const STREAM_VERBS: &[&str] = &[
    "Optimize",
    "Verify",
    "Stream",
    "Patch",
    "Bench",
    "Align",
    "Compact",
    "Audit",
];
const STREAM_SUBJECTS: &[&str] = &[
    "CommandBuffer delta frame",
    "Yoga/Taffy flex parity",
    "120fps Fenwick scroll slice",
    "Zero-copy EventRing batch",
    "Tier-2 cdylib plugin slot",
    "Skia GPU surface handoff",
    "WASM DOM flexbox commit",
    "QUIC live pairing ack window",
    "Mmap snapshot frame-0 boot",
];
const STREAM_DETAILS: &[&str] = &[
    "Zero per-frame heap allocation; deterministic O(log N) prefix sum slice.",
    "Verified < 256B binary delta payload with FNV-1a integrity check.",
    "Consistent cross-backend rect geometry across Taffy and Yoga engines.",
    "Hot-swap preserves full time-travel cursor and active virtual scroll offset.",
];

/// Deterministic variable height per row (compact 34px, standard 44px, tall thread 58px in Normal;
/// scaled cleanly in Compact density).
#[inline]
pub const fn deterministic_row_height(index: usize, density: DensityMode) -> u16 {
    let bucket = (index.wrapping_mul(37) ^ (index >> 3)) % 7;
    match density {
        DensityMode::Normal => match bucket {
            0 => 58,
            1 | 2 => 36,
            _ => 44,
        },
        DensityMode::Compact => match bucket {
            0 => 46,
            1 | 2 => 28,
            _ => 34,
        },
    }
}

/// Height of an explicitly expanded detail row.
#[inline]
pub const fn expanded_row_height(density: DensityMode) -> u16 {
    match density {
        DensityMode::Normal => 76,
        DensityMode::Compact => 62,
    }
}

/// Synthesizes deterministic Linear-style issue/stream row metadata in **O(1)** with **zero**
/// heap allocations.
#[inline]
pub fn synthesize_stream_row(
    index: usize,
    height_px: u32,
    is_expanded: bool,
) -> VirtualRowMeta {
    let mix = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((index as u64) >> 7);
    let author = STREAM_AUTHORS[(mix as usize) % STREAM_AUTHORS.len()];
    let tag = STREAM_TAGS[((mix >> 5) as usize) % STREAM_TAGS.len()];
    let verb = STREAM_VERBS[((mix >> 11) as usize) % STREAM_VERBS.len()];
    let subject = STREAM_SUBJECTS[((mix >> 17) as usize) % STREAM_SUBJECTS.len()];
    let detail = STREAM_DETAILS[((mix >> 23) as usize) % STREAM_DETAILS.len()];
    let priority = match (mix >> 29) % 5 {
        0 => Priority::P0,
        1 | 2 => Priority::P1,
        _ => Priority::P2,
    };
    let status = match (mix >> 33) % 3 {
        0 => TaskStatus::Todo,
        1 => TaskStatus::InProgress,
        _ => TaskStatus::Done,
    };
    let latency_us = 110 + ((mix >> 37) % 240) as u16;

    VirtualRowMeta {
        index,
        issue_number: (index as u32).saturating_add(1),
        author,
        tag,
        verb,
        subject,
        detail,
        priority,
        status,
        latency_us,
        height_px,
        is_expanded,
    }
}

fn normalize_expanded_rows(expanded_rows: &[usize], row_count: usize, out: &mut Vec<usize>) {
    out.clear();
    out.extend(expanded_rows.iter().copied().filter(|&idx| idx < row_count));
    out.sort_unstable();
    out.dedup();
}

/// Cached virtual list engine kept across frames in `StudioWorkspaceApp` so scrolling or jumping
/// across 100,000 rows performs zero index rebuilds and zero dataset allocations.
#[derive(Debug, Clone)]
pub struct StreamVirtualEngine {
    row_count: usize,
    density: DensityMode,
    variable_heights: bool,
    /// Sorted, deduplicated in-bounds expanded row indices for O(log k) membership checks.
    expanded_rows: Vec<usize>,
    heights: RowHeights,
}

impl StreamVirtualEngine {
    pub fn new(
        row_count: usize,
        density: DensityMode,
        variable_heights: bool,
        expanded_rows: &[usize],
    ) -> Self {
        let mut normalized = Vec::new();
        normalize_expanded_rows(expanded_rows, row_count, &mut normalized);
        let heights = stream_row_heights(row_count, density, variable_heights, &normalized);
        Self {
            row_count,
            density,
            variable_heights,
            expanded_rows: normalized,
            heights,
        }
    }

    /// Synchronizes configuration with minimal work:
    /// - Identical config: **O(1)** no-op.
    /// - All expanded rows collapsed in fixed-height mode: demotes back to O(1) zero-allocation
    ///   [`RowHeights::uniform`], immediately freeing the Fenwick tree.
    /// - Only `expanded_rows` changed: **O(k log N)** incremental Fenwick updates (with one-time
    ///   uniform→variable promotion if starting from fixed height) via sorted `binary_search`.
    /// - `density` or `variable_heights` toggled: rebuilds heights in one O(N) pass.
    pub fn sync_config(
        &mut self,
        density: DensityMode,
        variable_heights: bool,
        expanded_rows: &[usize],
    ) {
        if self.density == density
            && self.variable_heights == variable_heights
            && self.expanded_rows.as_slice() == expanded_rows
        {
            return;
        }

        let mut next_expanded = Vec::with_capacity(expanded_rows.len());
        normalize_expanded_rows(expanded_rows, self.row_count, &mut next_expanded);

        if self.density == density
            && self.variable_heights == variable_heights
            && self.expanded_rows == next_expanded
        {
            return;
        }

        if !variable_heights && next_expanded.is_empty() {
            self.density = density;
            self.variable_heights = false;
            self.expanded_rows.clear();
            self.heights = RowHeights::uniform(self.row_count, uniform_row_height(density));
            return;
        }

        if self.density == density && self.variable_heights == variable_heights {
            let uniform_h = uniform_row_height(density);
            for &old_row in &self.expanded_rows {
                if next_expanded.binary_search(&old_row).is_err() {
                    let base_h = if variable_heights {
                        u32::from(deterministic_row_height(old_row, density))
                    } else {
                        uniform_h
                    };
                    self.heights.set_row_height(old_row, base_h);
                }
            }
            let exp_h = u32::from(expanded_row_height(density));
            for &new_row in &next_expanded {
                if self.expanded_rows.binary_search(&new_row).is_err() {
                    self.heights.set_row_height(new_row, exp_h);
                }
            }
            self.expanded_rows = next_expanded;
            return;
        }

        self.density = density;
        self.variable_heights = variable_heights;
        self.heights = stream_row_heights(self.row_count, density, variable_heights, &next_expanded);
        self.expanded_rows = next_expanded;
    }

    #[inline]
    pub fn offset_for_row(&self, row_index: usize) -> u32 {
        self.heights.offset_of(row_index).min(u32::MAX as u64) as u32
    }

    #[inline]
    pub fn slice(
        &self,
        scroll_offset_y: f64,
        viewport_height: f64,
        overscan: usize,
    ) -> VirtualSlice {
        self.heights
            .slice(scroll_offset_y, viewport_height, overscan)
    }

    #[inline]
    pub fn row_meta(&self, index: usize) -> VirtualRowMeta {
        let h = self.heights.row_height(index);
        let is_expanded = self.expanded_rows.binary_search(&index).is_ok();
        synthesize_stream_row(index, h, is_expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DEFAULT_OVERSCAN, DEFAULT_STREAM_ROW_COUNT};
    use super::*;

    #[test]
    fn stream_engine_sync_config_promotes_demotes_ignores_oob_and_handles_toggles() {
        let n = 1_000usize;
        let mut eng = StreamVirtualEngine::new(n, DensityMode::Normal, false, &[n, n + 5]);
        // OOB expanded indices must not promote fixed-height mode.
        assert!(!eng.heights.is_variable());
        assert_eq!(eng.heights.total_height(), (n as u64) * 44);
        assert!(!eng.row_meta(0).is_expanded);

        // Expanding rows promotes Uniform -> Variable and updates offsets + metadata.
        eng.sync_config(DensityMode::Normal, false, &[500, 10, 500, n + 1]);
        assert!(eng.heights.is_variable());
        assert_eq!(eng.expanded_rows, vec![10, 500]);
        assert!(eng.row_meta(10).is_expanded);
        assert!(eng.row_meta(500).is_expanded);
        assert_eq!(
            eng.row_meta(500).height_px,
            u32::from(expanded_row_height(DensityMode::Normal))
        );
        let exp_delta = u64::from(expanded_row_height(DensityMode::Normal)) - 44;
        assert_eq!(eng.heights.total_height(), (n as u64) * 44 + 2 * exp_delta);

        // Partial collapse keeps Variable mode and restores exact collapsed row height.
        eng.sync_config(DensityMode::Normal, false, &[500]);
        assert!(eng.heights.is_variable());
        assert!(!eng.row_meta(10).is_expanded);
        assert_eq!(eng.row_meta(10).height_px, 44);
        assert_eq!(eng.heights.total_height(), (n as u64) * 44 + exp_delta);

        // Collapsing all rows in fixed-height mode demotes back to O(1) Uniform!
        eng.sync_config(DensityMode::Normal, false, &[]);
        assert!(!eng.heights.is_variable());
        assert_eq!(eng.heights.total_height(), (n as u64) * 44);

        // Toggling density and variable heights rebuilds cleanly and preserves expanded rows.
        eng.sync_config(DensityMode::Compact, true, &[25]);
        assert!(eng.heights.is_variable());
        assert!(eng.row_meta(25).is_expanded);
        assert_eq!(
            eng.row_meta(25).height_px,
            u32::from(expanded_row_height(DensityMode::Compact))
        );
        assert_eq!(
            eng.row_meta(26).height_px,
            u32::from(deterministic_row_height(26, DensityMode::Compact))
        );

        // Switching back to Compact fixed-height with no expanded rows demotes to Uniform(34).
        eng.sync_config(DensityMode::Compact, false, &[]);
        assert!(!eng.heights.is_variable());
        assert_eq!(eng.heights.total_height(), (n as u64) * 34);
    }

    #[test]
    fn virtual_list_100k_rows_spacer_invariant_and_uniform_variable_parity() {
        let uniform = RowHeights::uniform(DEFAULT_STREAM_ROW_COUNT, 44);
        assert_eq!(uniform.total_height(), 4_400_000);
        for &target_row in &[0usize, 1, 50_000, 99_999] {
            let offset = uniform.offset_of(target_row) as f64;
            let w = uniform.slice(offset, 520.0, DEFAULT_OVERSCAN);
            assert!(w.visible_count <= 22, "too many rows rendered: {w:?}");
            assert!(w.visible_count >= 10, "too few rows rendered: {w:?}");
            assert_eq!(
                w.top_spacer_px + w.rendered_height_px + w.bottom_spacer_px,
                w.total_height_px,
                "spacer sum must equal exact total height"
            );
            assert!(target_row >= w.start && target_row < w.end);
        }

        let var = stream_row_heights(
            DEFAULT_STREAM_ROW_COUNT,
            DensityMode::Normal,
            true,
            &[50_000],
        );
        assert_eq!(
            var.row_height(50_000),
            u32::from(expanded_row_height(DensityMode::Normal))
        );
        for &target_row in &[0usize, 12_345, 50_000, 99_999] {
            let offset = var.offset_of(target_row) as f64;
            let w = var.slice(offset, 520.0, DEFAULT_OVERSCAN);
            let sum_slice: u64 = (w.start..w.end)
                .map(|i| u64::from(var.row_height(i)))
                .sum();
            assert_eq!(w.rendered_height_px, sum_slice);
            assert_eq!(
                w.top_spacer_px + w.rendered_height_px + w.bottom_spacer_px,
                var.total_height()
            );
            assert!(
                w.visible_count <= 26,
                "window should stay O(viewport): {w:?}"
            );
            assert!(target_row >= w.start && target_row < w.end);
        }
    }

    #[test]
    fn edge_cases_empty_nan_negative_and_overscrolled_offsets() {
        let empty = RowHeights::uniform(0, 44);
        let w = empty.slice(100.0, 500.0, 3);
        assert_eq!(w.visible_count, 0);
        assert_eq!(w.total_height_px, 0);

        let idx = RowHeights::uniform(100, 40);
        let w_nan = idx.slice(f64::NAN, 400.0, 2);
        assert_eq!(w_nan.first_visible, 0);
        assert_eq!(w_nan.clamped_scroll_offset, 0.0);

        let w_neg = idx.slice(-500.0, 400.0, 2);
        assert_eq!(w_neg.first_visible, 0);

        let w_past = idx.slice(9_999_999.0, 400.0, 2);
        assert_eq!(w_past.end_visible, 100);
        assert_eq!(w_past.end, 100);
        assert_eq!(w_past.bottom_spacer_px, 0);
    }
}
