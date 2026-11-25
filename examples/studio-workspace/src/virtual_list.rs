//! O(viewport) 100,000-row virtualized 120fps list engine for **Aether Workspace**.
//!
//! Combines:
//! - Deterministic zero-allocation 100,000-row dataset synthesis (Linear-style issues & stream
//!   events generated on demand for only the visible window).
//! - O(1) uniform-height window slicing AND O(log N) Fenwick-tree (Binary Indexed Tree)
//!   variable-height prefix sums with bit-lifting lookup (`~17` iterations for 100,000 rows)
//!   and O(log N) live row-height updates (e.g. expanding/collapsing rows mid-scroll).
//! - Exact `top_spacer_px` / `bottom_spacer_px` accounting so native and web scrollbars reflect
//!   the true virtual document height while only rendering `~15–25` visible rows per frame.

use crate::state::{DensityMode, Priority, TaskStatus};

/// Default virtual dataset size for the 100k Stream view.
pub const DEFAULT_STREAM_ROW_COUNT: usize = 100_000;

/// Default overscan row count rendered above and below the visible viewport.
pub const DEFAULT_OVERSCAN: usize = 3;

/// Integer-precision Fenwick tree (Binary Indexed Tree) storing per-row heights in logical pixels.
///
/// Using integer `u16` row heights and `u32` prefix sums avoids cumulative floating-point drift
/// across 100,000 rows and keeps the entire 100k index compact (~600 KB, built once in ~0.2 ms).
#[derive(Debug, Clone)]
pub struct FenwickHeightTree {
    heights: Vec<u16>,
    /// 1-indexed Binary Indexed Tree of prefix sums.
    bit: Vec<u32>,
    total: u32,
}

impl FenwickHeightTree {
    /// Builds a Fenwick tree in strictly **O(N)** time.
    pub fn from_heights(heights: Vec<u16>) -> Self {
        let n = heights.len();
        let mut bit = vec![0u32; n + 1];
        let mut total = 0u32;

        for (idx, &raw_h) in heights.iter().enumerate() {
            let h = u32::from(raw_h.max(1));
            total = total.saturating_add(h);
            let i = idx + 1;
            bit[i] = bit[i].saturating_add(h);
            let parent = i + (i & i.wrapping_neg());
            if parent <= n {
                bit[parent] = bit[parent].saturating_add(bit[i]);
            }
        }

        let normalized = heights.into_iter().map(|h| h.max(1)).collect();
        Self {
            heights: normalized,
            bit,
            total,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Total document height in logical pixels — **O(1)**.
    #[inline]
    pub fn total_height(&self) -> u32 {
        self.total
    }

    /// Height of a single row in logical pixels — **O(1)**.
    #[inline]
    pub fn height_of(&self, index: usize) -> u16 {
        self.heights.get(index).copied().unwrap_or(0)
    }

    /// Updates the height of `index` in **O(log N)** time.
    pub fn set_height(&mut self, index: usize, new_height: u16) {
        let clamped = new_height.max(1);
        let Some(&old_h) = self.heights.get(index) else {
            return;
        };
        if old_h == clamped {
            return;
        }
        self.heights[index] = clamped;

        let n = self.heights.len();
        let mut i = index + 1;
        if clamped > old_h {
            let delta = u32::from(clamped - old_h);
            self.total = self.total.saturating_add(delta);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_add(delta);
                i += i & i.wrapping_neg();
            }
        } else {
            let delta = u32::from(old_h - clamped);
            self.total = self.total.saturating_sub(delta);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_sub(delta);
                i += i & i.wrapping_neg();
            }
        }
    }

    /// Pixel Y offset of the top edge of `row_index` (sum of `heights[0..row_index]`) — **O(log N)**.
    pub fn prefix_sum(&self, row_index: usize) -> u32 {
        let mut i = row_index.min(self.heights.len());
        let mut sum = 0u32;
        while i > 0 {
            sum = sum.saturating_add(self.bit[i]);
            i &= i - 1;
        }
        sum
    }

    /// Finds the row index containing pixel offset `target_y` via Fenwick bit-lifting — **O(log N)**.
    ///
    /// Returns the largest `pos <= len` such that `prefix_sum(pos) <= target_y`.
    pub fn find_row_at_offset(&self, target_y: u32) -> usize {
        let n = self.heights.len();
        if n == 0 || target_y == 0 {
            return 0;
        }
        if target_y >= self.total {
            return n;
        }

        let mut pos = 0usize;
        let mut acc = 0u32;
        let mut bit = n.next_power_of_two();
        while bit > 0 {
            let next = pos + bit;
            if next <= n {
                let candidate = acc.saturating_add(self.bit[next]);
                if candidate <= target_y {
                    pos = next;
                    acc = candidate;
                }
            }
            bit >>= 1;
        }
        pos
    }
}

/// Height storage mode for [`VirtualListIndex`].
#[derive(Debug, Clone)]
pub enum HeightLayout {
    /// Fixed row height: **O(1)** offset and window math with zero auxiliary memory.
    Uniform { count: usize, row_height: u16 },
    /// Per-row variable heights backed by a Fenwick tree: **O(log N)** lookup and live updates.
    Variable(FenwickHeightTree),
}

/// Computed visible + overscan slice returned by [`VirtualListIndex::slice_window`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualWindow {
    /// Total rows in the underlying dataset.
    pub total_rows: usize,
    /// First row intersecting `[scroll_offset_y, scroll_offset_y + viewport_height)`.
    pub first_visible: usize,
    /// Exclusive end of rows intersecting the viewport.
    pub last_visible: usize,
    /// First row to actually mount (including top overscan).
    pub render_start: usize,
    /// Exclusive end of rows to actually mount (including bottom overscan).
    pub render_end: usize,
    /// Clamped scroll offset actually applied.
    pub clamped_scroll_y: f32,
    /// Exact pixel height of virtual space above `render_start`.
    pub top_spacer_px: f32,
    /// Exact pixel height of the rendered slice `[render_start..render_end)`.
    pub rendered_height_px: f32,
    /// Exact pixel height of virtual space below `render_end`.
    pub bottom_spacer_px: f32,
    /// Exact total virtual document height.
    pub total_height_px: f32,
}

impl VirtualWindow {
    /// Number of rows actually mounted in this frame (always O(viewport)).
    #[inline]
    pub const fn rendered_count(&self) -> usize {
        self.render_end.saturating_sub(self.render_start)
    }

    /// Number of rows directly visible inside the viewport (excluding overscan).
    #[inline]
    pub const fn visible_count(&self) -> usize {
        self.last_visible.saturating_sub(self.first_visible)
    }
}

/// High-performance virtual list geometry index supporting both uniform O(1) and
/// variable-height O(log N) Fenwick-tree layouts.
#[derive(Debug, Clone)]
pub struct VirtualListIndex {
    layout: HeightLayout,
}

impl VirtualListIndex {
    /// Creates an O(1) uniform-height index for `count` rows.
    pub fn uniform(count: usize, row_height: u16) -> Self {
        Self {
            layout: HeightLayout::Uniform {
                count,
                row_height: row_height.max(1),
            },
        }
    }

    /// Creates an O(log N) variable-height index backed by a Fenwick tree.
    pub fn variable(heights: Vec<u16>) -> Self {
        Self {
            layout: HeightLayout::Variable(FenwickHeightTree::from_heights(heights)),
        }
    }

    /// Builds the deterministic 100,000-row Aether stream height index for the given density
    /// and variable-height mode.
    pub fn for_stream(
        count: usize,
        density: DensityMode,
        variable_heights: bool,
        expanded_rows: &[usize],
    ) -> Self {
        if !variable_heights && expanded_rows.is_empty() {
            let uniform_h = match density {
                DensityMode::Normal => 44,
                DensityMode::Compact => 34,
            };
            return Self::uniform(count, uniform_h);
        }

        let mut heights = Vec::with_capacity(count);
        for idx in 0..count {
            let h = if variable_heights {
                deterministic_row_height(idx, density)
            } else {
                match density {
                    DensityMode::Normal => 44,
                    DensityMode::Compact => 34,
                }
            };
            heights.push(h);
        }
        for &row_idx in expanded_rows {
            if let Some(slot) = heights.get_mut(row_idx) {
                *slot = expanded_row_height(density);
            }
        }
        Self::variable(heights)
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.layout {
            HeightLayout::Uniform { count, .. } => *count,
            HeightLayout::Variable(tree) => tree.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn is_variable(&self) -> bool {
        matches!(self.layout, HeightLayout::Variable(_))
    }

    /// Total virtual scrollable height in logical pixels — **O(1)**.
    #[inline]
    pub fn total_height(&self) -> u32 {
        match &self.layout {
            HeightLayout::Uniform { count, row_height } => {
                (*count as u64).saturating_mul(u64::from(*row_height)).min(u32::MAX as u64) as u32
            }
            HeightLayout::Variable(tree) => tree.total_height(),
        }
    }

    /// Height of row `index` in logical pixels — **O(1)**.
    #[inline]
    pub fn row_height(&self, index: usize) -> u16 {
        match &self.layout {
            HeightLayout::Uniform { count, row_height } => {
                if index < *count {
                    *row_height
                } else {
                    0
                }
            }
            HeightLayout::Variable(tree) => tree.height_of(index),
        }
    }

    /// Y pixel offset of row `index` — **O(1)** in uniform mode, **O(log N)** in variable mode.
    #[inline]
    pub fn row_offset(&self, index: usize) -> u32 {
        match &self.layout {
            HeightLayout::Uniform { count, row_height } => {
                let clamped = index.min(*count);
                (clamped as u64)
                    .saturating_mul(u64::from(*row_height))
                    .min(u32::MAX as u64) as u32
            }
            HeightLayout::Variable(tree) => tree.prefix_sum(index),
        }
    }

    /// Updates a single row's height in **O(log N)** time (transparently promoting uniform mode
    /// to variable mode on first heterogeneous update).
    pub fn set_row_height(&mut self, index: usize, new_height: u16) {
        let clamped = new_height.max(1);
        match &mut self.layout {
            HeightLayout::Uniform { count, row_height } => {
                if index >= *count || *row_height == clamped {
                    return;
                }
                let mut heights = vec![*row_height; *count];
                heights[index] = clamped;
                self.layout = HeightLayout::Variable(FenwickHeightTree::from_heights(heights));
            }
            HeightLayout::Variable(tree) => {
                tree.set_height(index, clamped);
            }
        }
    }

    /// Finds the row index covering vertical offset `y` — **O(1)** uniform / **O(log N)** variable.
    #[inline]
    pub fn row_at_offset(&self, y: u32) -> usize {
        match &self.layout {
            HeightLayout::Uniform { count, row_height } => {
                if *count == 0 {
                    return 0;
                }
                ((y as usize) / usize::from(*row_height)).min(*count)
            }
            HeightLayout::Variable(tree) => tree.find_row_at_offset(y),
        }
    }

    /// Computes the exact `[first_visible..last_visible)` and overscanned `[render_start..render_end)`
    /// slice plus spacer heights for a given `(scroll_offset_y, viewport_height, overscan)`.
    ///
    /// Runs in **O(1)** time for uniform row heights and **O(log N)** time for variable row heights.
    pub fn slice_window(
        &self,
        scroll_offset_y: f32,
        viewport_height: f32,
        overscan: usize,
    ) -> VirtualWindow {
        let total_rows = self.len();
        let total_h = self.total_height();
        if total_rows == 0 || total_h == 0 {
            return VirtualWindow {
                total_rows: 0,
                first_visible: 0,
                last_visible: 0,
                render_start: 0,
                render_end: 0,
                clamped_scroll_y: 0.0,
                top_spacer_px: 0.0,
                rendered_height_px: 0.0,
                bottom_spacer_px: 0.0,
                total_height_px: 0.0,
            };
        }

        let vp_h = if viewport_height.is_finite() && viewport_height > 0.0 {
            viewport_height.ceil() as u32
        } else {
            1
        };
        let max_scroll = total_h.saturating_sub(vp_h);
        let raw_scroll = if scroll_offset_y.is_finite() && scroll_offset_y > 0.0 {
            scroll_offset_y.floor() as u32
        } else {
            0
        };
        let scroll_y = raw_scroll.min(max_scroll);

        let first_visible = self.row_at_offset(scroll_y).min(total_rows - 1);
        let end_y = scroll_y.saturating_add(vp_h).min(total_h);

        let last_visible = if end_y >= total_h {
            total_rows
        } else {
            // Find the last row overlapping `[scroll_y, end_y)`.
            let probe = self.row_at_offset(end_y.saturating_sub(1));
            (probe + 1).min(total_rows).max(first_visible + 1)
        };

        let render_start = first_visible.saturating_sub(overscan);
        let render_end = last_visible.saturating_add(overscan).min(total_rows);

        let top_px = self.row_offset(render_start);
        let end_px = self.row_offset(render_end);
        let rendered_px = end_px.saturating_sub(top_px);
        let bottom_px = total_h.saturating_sub(end_px);

        VirtualWindow {
            total_rows,
            first_visible,
            last_visible,
            render_start,
            render_end,
            clamped_scroll_y: scroll_y as f32,
            top_spacer_px: top_px as f32,
            rendered_height_px: rendered_px as f32,
            bottom_spacer_px: bottom_px as f32,
            total_height_px: total_h as f32,
        }
    }
}

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
    // Cheap bijective-ish deterministic mixing (SplitMix-style) for realistic distribution.
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
    index: VirtualListIndex,
}

impl StreamVirtualEngine {
    pub fn new(
        row_count: usize,
        density: DensityMode,
        variable_heights: bool,
        expanded_rows: &[usize],
    ) -> Self {
        let index = VirtualListIndex::for_stream(row_count, density, variable_heights, expanded_rows);
        Self {
            row_count,
            density,
            variable_heights,
            expanded_rows: expanded_rows.to_vec(),
            index,
        }
    }

    /// Synchronizes configuration with minimal work:
    /// - Identical config: **O(1)** no-op.
    /// - Only `expanded_rows` changed: **O(k log N)** incremental Fenwick updates!
    /// - `density` or `variable_heights` toggled: rebuilds index in O(N) once.
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
            // Incremental O(log N) patch for just the rows whose expanded state changed!
            for &old_row in &self.expanded_rows {
                if !expanded_rows.contains(&old_row) {
                    let base_h = if variable_heights {
                        deterministic_row_height(old_row, density)
                    } else {
                        match density {
                            DensityMode::Normal => 44,
                            DensityMode::Compact => 34,
                        }
                    };
                    self.index.set_row_height(old_row, base_h);
                }
            }
            let exp_h = expanded_row_height(density);
            for &new_row in expanded_rows {
                if !self.expanded_rows.contains(&new_row) {
                    self.index.set_row_height(new_row, exp_h);
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
        self.index = VirtualListIndex::for_stream(
            self.row_count,
            density,
            variable_heights,
            expanded_rows,
        );
    }

    #[inline]
    pub fn index(&self) -> &VirtualListIndex {
        &self.index
    }

    #[inline]
    pub fn offset_for_row(&self, row_index: usize) -> u32 {
        self.index.row_offset(row_index)
    }

    #[inline]
    pub fn slice_window(
        &self,
        scroll_offset_y: f32,
        viewport_height: f32,
        overscan: usize,
    ) -> VirtualWindow {
        self.index
            .slice_window(scroll_offset_y, viewport_height, overscan)
    }

    #[inline]
    pub fn row_meta(&self, index: usize) -> VirtualRowMeta {
        let h = self.index.row_height(index);
        let is_expanded = self.expanded_rows.contains(&index);
        synthesize_stream_row(index, h, is_expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenwick_tree_exact_prefix_sums_lower_bound_and_live_updates() {
        let heights = vec![40u16, 50, 30, 60, 20];
        let mut bit = FenwickHeightTree::from_heights(heights.clone());
        assert_eq!(bit.len(), 5);
        assert_eq!(bit.total_height(), 200);

        // Exact prefix sums: [0, 40, 90, 120, 180, 200]
        assert_eq!(bit.prefix_sum(0), 0);
        assert_eq!(bit.prefix_sum(1), 40);
        assert_eq!(bit.prefix_sum(2), 90);
        assert_eq!(bit.prefix_sum(3), 120);
        assert_eq!(bit.prefix_sum(4), 180);
        assert_eq!(bit.prefix_sum(5), 200);

        // Boundary lower-bound probes
        assert_eq!(bit.find_row_at_offset(0), 0);
        assert_eq!(bit.find_row_at_offset(39), 0);
        assert_eq!(bit.find_row_at_offset(40), 1);
        assert_eq!(bit.find_row_at_offset(89), 1);
        assert_eq!(bit.find_row_at_offset(90), 2);
        assert_eq!(bit.find_row_at_offset(119), 2);
        assert_eq!(bit.find_row_at_offset(120), 3);
        assert_eq!(bit.find_row_at_offset(179), 3);
        assert_eq!(bit.find_row_at_offset(180), 4);
        assert_eq!(bit.find_row_at_offset(199), 4);
        assert_eq!(bit.find_row_at_offset(200), 5);
        assert_eq!(bit.find_row_at_offset(999), 5);

        // Live O(log N) height update on row 1: 50 -> 100 (+50)
        bit.set_height(1, 100);
        assert_eq!(bit.total_height(), 250);
        assert_eq!(bit.prefix_sum(2), 140);
        assert_eq!(bit.find_row_at_offset(139), 1);
        assert_eq!(bit.find_row_at_offset(140), 2);

        // Shrink row 3: 60 -> 10 (-50) -> total back to 200
        bit.set_height(3, 10);
        assert_eq!(bit.total_height(), 200);
        assert_eq!(bit.prefix_sum(4), 180);
        assert_eq!(bit.find_row_at_offset(179), 3);
        assert_eq!(bit.find_row_at_offset(180), 4);
    }

    #[test]
    fn virtual_list_100k_rows_spacer_invariant_and_uniform_variable_parity() {
        // 1. Uniform 100,000 rows @ 44px
        let uniform = VirtualListIndex::uniform(DEFAULT_STREAM_ROW_COUNT, 44);
        assert_eq!(uniform.total_height(), 4_400_000);
        for &target_row in &[0usize, 1, 50_000, 99_999] {
            let offset = uniform.row_offset(target_row) as f32;
            let w = uniform.slice_window(offset, 520.0, DEFAULT_OVERSCAN);
            assert!(w.rendered_count() <= 22, "too many rows rendered: {w:?}");
            assert!(w.rendered_count() >= 10, "too few rows rendered: {w:?}");
            assert_eq!(
                w.top_spacer_px + w.rendered_height_px + w.bottom_spacer_px,
                w.total_height_px,
                "spacer sum must equal exact total height"
            );
            assert!(target_row >= w.render_start && target_row < w.render_end);
        }

        // 2. Variable 100,000 rows via Fenwick tree
        let var = VirtualListIndex::for_stream(
            DEFAULT_STREAM_ROW_COUNT,
            DensityMode::Normal,
            true,
            &[50_000],
        );
        assert_eq!(var.row_height(50_000), expanded_row_height(DensityMode::Normal));
        for &target_row in &[0usize, 12_345, 50_000, 99_999] {
            let offset = var.row_offset(target_row) as f32;
            let w = var.slice_window(offset, 520.0, DEFAULT_OVERSCAN);
            let sum_slice: u32 = (w.render_start..w.render_end)
                .map(|i| u32::from(var.row_height(i)))
                .sum();
            assert_eq!(w.rendered_height_px as u32, sum_slice);
            assert_eq!(
                (w.top_spacer_px + w.rendered_height_px + w.bottom_spacer_px) as u32,
                var.total_height()
            );
            assert!(w.rendered_count() <= 26, "window should stay O(viewport): {w:?}");
            assert!(target_row >= w.render_start && target_row < w.render_end);
        }
    }

    #[test]
    fn edge_cases_empty_nan_negative_and_overscrolled_offsets() {
        let empty = VirtualListIndex::uniform(0, 44);
        let w = empty.slice_window(100.0, 500.0, 3);
        assert_eq!(w.rendered_count(), 0);
        assert_eq!(w.total_height_px, 0.0);

        let idx = VirtualListIndex::uniform(100, 40);
        let w_nan = idx.slice_window(f32::NAN, 400.0, 2);
        assert_eq!(w_nan.first_visible, 0);
        assert_eq!(w_nan.clamped_scroll_y, 0.0);

        let w_neg = idx.slice_window(-500.0, 400.0, 2);
        assert_eq!(w_neg.first_visible, 0);

        let w_past = idx.slice_window(9_999_999.0, 400.0, 2);
        assert_eq!(w_past.last_visible, 100);
        assert_eq!(w_past.render_end, 100);
        assert_eq!(w_past.bottom_spacer_px, 0.0);
    }
}
