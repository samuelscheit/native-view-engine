//! Deterministic zero-allocation 100,000-row dataset synthesis and cached `StreamVirtualEngine`.

use crate::state::{DensityMode, Priority, TaskStatus};

use super::{stream_row_heights, RowHeights, VirtualSlice};

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
    pub height_px: u16,
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
    height_px: u16,
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

/// Cached virtual list engine kept across frames in `StudioWorkspaceApp` so scrolling or jumping
/// across 100,000 rows performs zero index rebuilds and zero dataset allocations.
#[derive(Debug, Clone)]
pub struct StreamVirtualEngine {
    row_count: usize,
    density: DensityMode,
    variable_heights: bool,
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
        let heights = stream_row_heights(row_count, density, variable_heights, expanded_rows);
        Self {
            row_count,
            density,
            variable_heights,
            expanded_rows: expanded_rows.to_vec(),
            heights,
        }
    }

    /// Synchronizes configuration with minimal work:
    /// - Identical config: **O(1)** no-op.
    /// - Only `expanded_rows` changed: **O(k log N)** incremental Fenwick updates (with one-time
    ///   uniform→variable promotion if starting from fixed height)!
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

        if self.density == density && self.variable_heights == variable_heights {
            for &old_row in &self.expanded_rows {
                if !expanded_rows.contains(&old_row) {
                    let base_h = if variable_heights {
                        u32::from(deterministic_row_height(old_row, density))
                    } else {
                        match density {
                            DensityMode::Normal => 44,
                            DensityMode::Compact => 34,
                        }
                    };
                    self.heights.set_row_height(old_row, base_h);
                }
            }
            let exp_h = u32::from(expanded_row_height(density));
            for &new_row in expanded_rows {
                if !self.expanded_rows.contains(&new_row) {
                    self.heights.set_row_height(new_row, exp_h);
                }
            }
            self.expanded_rows.clear();
            self.expanded_rows.extend_from_slice(expanded_rows);
            return;
        }

        self.density = density;
        self.variable_heights = variable_heights;
        self.expanded_rows.clear();
        self.expanded_rows.extend_from_slice(expanded_rows);
        self.heights =
            stream_row_heights(self.row_count, density, variable_heights, expanded_rows);
    }

    #[inline]
    pub fn heights(&self) -> &RowHeights {
        &self.heights
    }

    #[inline]
    pub fn offset_for_row(&self, row_index: usize) -> u32 {
        self.heights.offset_of_u64(row_index).min(u32::MAX as u64) as u32
    }

    #[inline]
    pub fn slice(
        &self,
        scroll_offset_y: f32,
        viewport_height: f32,
        overscan: usize,
    ) -> VirtualSlice {
        self.heights
            .slice(scroll_offset_y, viewport_height, overscan)
    }

    #[inline]
    pub fn row_meta(&self, index: usize) -> VirtualRowMeta {
        let h = self.heights.row_height(index) as u16;
        let is_expanded = self.expanded_rows.contains(&index);
        synthesize_stream_row(index, h, is_expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FenwickHeightTree, DEFAULT_OVERSCAN, DEFAULT_STREAM_ROW_COUNT};
    use super::*;

    #[test]
    fn fenwick_tree_exact_prefix_sums_lower_bound_and_live_updates() {
        let heights = vec![40u32, 50, 30, 60, 20];
        let mut bit = FenwickHeightTree::from_heights(heights);
        assert_eq!(bit.len(), 5);
        assert_eq!(bit.total_height(), 200);

        assert_eq!(bit.prefix_sum(0), 0);
        assert_eq!(bit.prefix_sum(1), 40);
        assert_eq!(bit.prefix_sum(2), 90);
        assert_eq!(bit.prefix_sum(3), 120);
        assert_eq!(bit.prefix_sum(4), 180);
        assert_eq!(bit.prefix_sum(5), 200);

        assert_eq!(bit.lower_bound(0), 0);
        assert_eq!(bit.lower_bound(39), 0);
        assert_eq!(bit.lower_bound(40), 1);
        assert_eq!(bit.lower_bound(89), 1);
        assert_eq!(bit.lower_bound(90), 2);
        assert_eq!(bit.lower_bound(119), 2);
        assert_eq!(bit.lower_bound(120), 3);
        assert_eq!(bit.lower_bound(179), 3);
        assert_eq!(bit.lower_bound(180), 4);
        assert_eq!(bit.lower_bound(199), 4);
        assert_eq!(bit.lower_bound(200), 5);
        assert_eq!(bit.lower_bound(999), 5);

        assert!(bit.set_height(1, 100));
        assert_eq!(bit.total_height(), 250);
        assert_eq!(bit.prefix_sum(2), 140);
        assert_eq!(bit.lower_bound(139), 1);
        assert_eq!(bit.lower_bound(140), 2);

        assert!(bit.set_height(3, 10));
        assert_eq!(bit.total_height(), 200);
        assert_eq!(bit.prefix_sum(4), 180);
        assert_eq!(bit.lower_bound(179), 3);
        assert_eq!(bit.lower_bound(180), 4);
    }

    #[test]
    fn virtual_list_100k_rows_spacer_invariant_and_uniform_variable_parity() {
        let uniform = RowHeights::uniform(DEFAULT_STREAM_ROW_COUNT, 44.0);
        assert_eq!(uniform.total_height_u64(), 4_400_000);
        for &target_row in &[0usize, 1, 50_000, 99_999] {
            let offset = uniform.offset_of(target_row);
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
            var.row_height(50_000) as u16,
            expanded_row_height(DensityMode::Normal)
        );
        for &target_row in &[0usize, 12_345, 50_000, 99_999] {
            let offset = var.offset_of(target_row);
            let w = var.slice(offset, 520.0, DEFAULT_OVERSCAN);
            let sum_slice: u64 = (w.start..w.end)
                .map(|i| var.row_height(i) as u64)
                .sum();
            assert_eq!(w.rendered_height_px as u64, sum_slice);
            assert_eq!(
                (w.top_spacer_px + w.rendered_height_px + w.bottom_spacer_px) as u64,
                var.total_height_u64()
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
        let empty = RowHeights::uniform(0, 44.0);
        let w = empty.slice(100.0, 500.0, 3);
        assert_eq!(w.visible_count, 0);
        assert_eq!(w.total_height_px, 0.0);

        let idx = RowHeights::uniform(100, 40.0);
        let w_nan = idx.slice(f32::NAN, 400.0, 2);
        assert_eq!(w_nan.first_visible, 0);
        assert_eq!(w_nan.clamped_scroll_offset, 0.0);

        let w_neg = idx.slice(-500.0, 400.0, 2);
        assert_eq!(w_neg.first_visible, 0);

        let w_past = idx.slice(9_999_999.0, 400.0, 2);
        assert_eq!(w_past.end_visible, 100);
        assert_eq!(w_past.end, 100);
        assert_eq!(w_past.bottom_spacer_px, 0.0);
    }
}
