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

mod dataset;

pub use dataset::{
    deterministic_row_height, expanded_row_height, synthesize_stream_row, StreamVirtualEngine,
    VirtualRowMeta,
};

use crate::state::DensityMode;

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
            let parent = i + i.isolate_lowest_one();
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
                i += i.isolate_lowest_one();
            }
        } else {
            let delta = u32::from(old_h - clamped);
            self.total = self.total.saturating_sub(delta);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_sub(delta);
                i += i.isolate_lowest_one();
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
