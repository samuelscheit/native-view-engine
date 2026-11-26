//! Pure, testable virtualized list state (`ScrollAlign`, `VirtualSlice`, and `VirtualListState`).

use super::engine::RowHeights;

/// Alignment strategy used by [`VirtualListState::scroll_to_index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollAlign {
    #[default]
    Nearest,
    Start,
    Center,
    End,
}

/// Computed visible window and top/bottom spacer heights for O(viewport) rendering.
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
    pub clamped_scroll_offset: f32,
    /// Exact pixel height of virtual space above `start`.
    pub top_spacer_px: f32,
    /// Exact pixel height of the mounted slice `[start..end)`.
    pub rendered_height_px: f32,
    /// Exact pixel height of virtual space below `end`.
    pub bottom_spacer_px: f32,
    /// Exact total virtual document height.
    pub total_height_px: f32,
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
        top_spacer_px: 0.0,
        rendered_height_px: 0.0,
        bottom_spacer_px: 0.0,
        total_height_px: 0.0,
        visible_count: 0,
    };

    /// Number of rows directly visible inside the viewport (excluding overscan).
    #[inline]
    pub const fn viewport_row_count(&self) -> usize {
        self.end_visible.saturating_sub(self.first_visible)
    }
}

/// Pure, testable virtualized list state handling overscan slicing, `scroll_to_index`, and
/// keyboard navigation (`ArrowUp`/`ArrowDown`/`PageUp`/`PageDown`/`Home`/`End`).
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualListState {
    pub heights: RowHeights,
    pub viewport_height: f32,
    pub scroll_offset: f32,
    pub overscan: usize,
    pub selected_index: Option<usize>,
}

impl VirtualListState {
    pub fn new(heights: RowHeights, viewport_height: f32, overscan: usize) -> Self {
        let vp = if viewport_height.is_finite() && viewport_height > 0.0 {
            viewport_height
        } else {
            0.0
        };
        Self {
            heights,
            viewport_height: vp,
            scroll_offset: 0.0,
            overscan,
            selected_index: None,
        }
    }

    pub fn max_scroll_offset(&self) -> f32 {
        (self.heights.total_height() - self.viewport_height).max(0.0)
    }

    /// Clamps and stores a raw scroll offset (safely handles NaN, negative, and overscrolled inputs).
    pub fn set_scroll_offset(&mut self, raw_offset: f32) -> f32 {
        let clamped = if !raw_offset.is_finite() || raw_offset <= 0.0 {
            0.0
        } else {
            raw_offset.min(self.max_scroll_offset())
        };
        self.scroll_offset = clamped;
        clamped
    }

    /// Computes the O(viewport) visible slice with exact top and bottom spacer accounting.
    #[inline]
    pub fn slice(&self) -> VirtualSlice {
        self.heights
            .slice(self.scroll_offset, self.viewport_height, self.overscan)
    }

    /// Scrolls the viewport so `index` satisfies `align`, returning the new clamped scroll offset.
    pub fn scroll_to_index(&mut self, index: usize, align: ScrollAlign) -> f32 {
        let len = self.heights.len();
        if len == 0 {
            self.scroll_offset = 0.0;
            return 0.0;
        }
        let clamped_idx = index.min(len - 1);
        let row_top = self.heights.offset_of(clamped_idx);
        let row_h = self.heights.row_height(clamped_idx);
        let row_bottom = row_top + row_h;

        let target = match align {
            ScrollAlign::Start => row_top,
            ScrollAlign::Center => row_top - ((self.viewport_height - row_h) * 0.5),
            ScrollAlign::End => row_bottom - self.viewport_height,
            ScrollAlign::Nearest => {
                if row_top < self.scroll_offset {
                    row_top
                } else if row_bottom > self.scroll_offset + self.viewport_height {
                    row_bottom - self.viewport_height
                } else {
                    self.scroll_offset
                }
            }
        };
        self.set_scroll_offset(target)
    }

    /// Processes keyboard list navigation (`ArrowUp`, `ArrowDown`, `PageUp`, `PageDown`,
    /// `Home`, `End`), updating `selected_index` and scrolling it into view.
    pub fn handle_key(&mut self, key: &str) -> Option<usize> {
        let len = self.heights.len();
        if len == 0 {
            self.selected_index = None;
            return None;
        }
        let cur = self.selected_index.unwrap_or(0).min(len - 1);
        let row_h = self.heights.row_height(cur).max(1.0);
        let page_step = (self.viewport_height / row_h).floor().max(1.0) as usize;

        let next = match key {
            "ArrowDown" => {
                if self.selected_index.is_none() {
                    0
                } else {
                    (cur + 1).min(len - 1)
                }
            }
            "ArrowUp" => cur.saturating_sub(1),
            "PageDown" => (cur.saturating_add(page_step)).min(len - 1),
            "PageUp" => cur.saturating_sub(page_step),
            "Home" => 0,
            "End" => len - 1,
            _ => return None,
        };

        self.selected_index = Some(next);
        self.scroll_to_index(next, ScrollAlign::Nearest);
        Some(next)
    }
}
