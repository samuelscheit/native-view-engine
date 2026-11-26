//! Pure, testable virtualized list state (`ScrollAlign`, `VirtualSlice`, and `VirtualListState`).

use super::engine::{clamp_scroll, sanitize_extent, RowHeights};

/// Alignment strategy used by [`VirtualListState::scroll_to_index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAlign {
    #[default]
    Nearest,
    Start,
    Center,
    End,
}

/// Computed visible window and exact `u64` top/rendered/bottom spacer heights for O(viewport)
/// rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualSlice {
    /// First mounted row index (including top overscan).
    pub start: usize,
    /// Exclusive end of mounted rows (including bottom overscan).
    pub end: usize,
    /// First row index intersecting the viewport (excluding overscan).
    pub first_visible: usize,
    /// Exclusive end of rows intersecting the viewport (excluding overscan).
    pub end_visible: usize,
    /// Clamped scroll offset actually applied to compute this slice.
    pub clamped_scroll_offset: f64,
    /// Exact pixel height of virtual space above `start`.
    pub top_spacer_px: u64,
    /// Exact pixel height of the mounted slice `[start..end)`.
    pub rendered_height_px: u64,
    /// Exact pixel height of virtual space below `end`.
    pub bottom_spacer_px: u64,
    /// Exact total virtual document height.
    pub total_height_px: u64,
    /// Number of mounted rows (`end - start`, including overscan).
    pub visible_count: usize,
}

impl VirtualSlice {
    pub const EMPTY: Self = Self {
        start: 0,
        end: 0,
        first_visible: 0,
        end_visible: 0,
        clamped_scroll_offset: 0.0,
        top_spacer_px: 0,
        rendered_height_px: 0,
        bottom_spacer_px: 0,
        total_height_px: 0,
        visible_count: 0,
    };
}

/// Pure scroll-to-index calculation shared by [`VirtualListState`] and [`super::VirtualList`].
pub(super) fn compute_scroll_to_index(
    heights: &RowHeights,
    current_scroll: f64,
    viewport_height: f64,
    index: usize,
    align: ScrollAlign,
) -> f64 {
    let len = heights.len();
    if len == 0 {
        return 0.0;
    }
    let vp = sanitize_extent(viewport_height);
    let total = heights.total_height();
    let scroll = clamp_scroll(current_scroll, total, vp);
    let clamped_idx = index.min(len - 1);
    let row_top = heights.offset_of(clamped_idx) as f64;
    let row_h = f64::from(heights.row_height(clamped_idx));
    let row_bottom = row_top + row_h;

    let target = match align {
        ScrollAlign::Start => row_top,
        ScrollAlign::Center => row_top - ((vp - row_h) * 0.5),
        ScrollAlign::End => row_bottom - vp,
        ScrollAlign::Nearest => {
            if row_top < scroll {
                row_top
            } else if row_bottom > scroll + vp {
                row_bottom - vp
            } else {
                scroll
            }
        }
    };
    clamp_scroll(target, total, vp)
}

/// Pure keyboard navigation calculation shared by [`VirtualListState`] and [`super::VirtualList`].
///
/// `PageDown` and `PageUp` jump by actual `viewport_height` pixels via `index_at_offset` rather
/// than dividing by the current row's height, keeping page jumps accurate on variable-height lists.
pub(super) fn compute_handle_key(
    heights: &RowHeights,
    current_scroll: f64,
    viewport_height: f64,
    selected_index: Option<usize>,
    key: &str,
) -> Option<(usize, f64)> {
    let len = heights.len();
    if len == 0 {
        return None;
    }
    let vp = sanitize_extent(viewport_height);
    let cur = selected_index.unwrap_or(0).min(len - 1);

    let next = match key {
        "ArrowDown" => {
            if selected_index.is_none() {
                0
            } else {
                (cur + 1).min(len - 1)
            }
        }
        "ArrowUp" => cur.saturating_sub(1),
        "PageDown" => {
            if cur + 1 >= len {
                len - 1
            } else {
                let target_px = (heights.offset_of(cur) as f64) + vp;
                heights.index_at_offset(target_px).max(cur + 1).min(len - 1)
            }
        }
        "PageUp" => {
            if cur == 0 {
                0
            } else {
                let target_px = ((heights.offset_of(cur) as f64) - vp).max(0.0);
                heights.index_at_offset(target_px).min(cur - 1)
            }
        }
        "Home" => 0,
        "End" => len - 1,
        _ => return None,
    };

    let next_scroll =
        compute_scroll_to_index(heights, current_scroll, vp, next, ScrollAlign::Nearest);
    Some((next, next_scroll))
}

/// Pure, testable virtualized list state handling overscan slicing, `scroll_to_index`, and
/// keyboard navigation (`ArrowUp`/`ArrowDown`/`PageUp`/`PageDown`/`Home`/`End`).
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListState {
    pub heights: RowHeights,
    pub viewport_height: f64,
    pub scroll_offset: f64,
    pub overscan: usize,
    pub selected_index: Option<usize>,
}

impl VirtualListState {
    pub fn new(heights: RowHeights, viewport_height: f64, overscan: usize) -> Self {
        Self {
            heights,
            viewport_height: sanitize_extent(viewport_height),
            scroll_offset: 0.0,
            overscan,
            selected_index: None,
        }
    }

    pub fn max_scroll_offset(&self) -> f64 {
        let vp = sanitize_extent(self.viewport_height);
        ((self.heights.total_height() as f64) - vp).max(0.0)
    }

    /// Clamps and stores a raw scroll offset (safely handles NaN, negative, and overscrolled inputs).
    pub fn set_scroll_offset(&mut self, raw_offset: f64) -> f64 {
        let clamped = clamp_scroll(raw_offset, self.heights.total_height(), self.viewport_height);
        self.scroll_offset = clamped;
        clamped
    }

    /// Computes the O(viewport) visible slice with exact `u64` top/rendered/bottom spacer accounting.
    #[inline]
    pub fn slice(&self) -> VirtualSlice {
        self.heights
            .slice(self.scroll_offset, self.viewport_height, self.overscan)
    }

    /// Scrolls the viewport so `index` satisfies `align`, returning the new clamped scroll offset.
    pub fn scroll_to_index(&mut self, index: usize, align: ScrollAlign) -> f64 {
        let next = compute_scroll_to_index(
            &self.heights,
            self.scroll_offset,
            self.viewport_height,
            index,
            align,
        );
        self.scroll_offset = next;
        next
    }

    /// Processes keyboard list navigation (`ArrowUp`, `ArrowDown`, `PageUp`, `PageDown`,
    /// `Home`, `End`), updating `selected_index` and scrolling it into view.
    pub fn handle_key(&mut self, key: &str) -> Option<usize> {
        if self.heights.is_empty() {
            self.selected_index = None;
            return None;
        }
        let (next, next_scroll) = compute_handle_key(
            &self.heights,
            self.scroll_offset,
            self.viewport_height,
            self.selected_index,
            key,
        )?;
        self.selected_index = Some(next);
        self.scroll_offset = next_scroll;
        Some(next)
    }
}
