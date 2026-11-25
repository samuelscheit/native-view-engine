//! High-performance virtualized list engine (`FenwickHeightTree`, `RowHeights`,
//! `VirtualListState`, and [`VirtualList`]) supporting 100,000+ uniform or variable-height rows
//! in strictly O(log N) prefix-sum/lower-bound time and O(viewport) rendered DOM nodes.

use dioxus_core::{Callback, EventHandler};
use dioxus_core_macro::Props;
use nve_dioxus::prelude::*;

use crate::style_buf::{StyleBuf, SurfaceStyle};
use crate::theme::use_theme;

/// Reusable integer-pixel Fenwick tree (Binary Indexed Tree) storing per-row heights with
/// O(N) construction, O(1) total height, and O(log N) prefix sums, height updates, and
/// bit-lifting `lower_bound` offset queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenwickHeightTree {
    heights: Vec<u32>,
    /// 1-indexed Binary Indexed Tree of `u64` prefix sums (overflow-proof across millions of rows).
    bit: Vec<u64>,
    total: u64,
}

impl FenwickHeightTree {
    /// Builds a Fenwick tree in strictly **O(N)** time. Row heights `< 1` are clamped to `1`.
    pub fn from_heights(heights: Vec<u32>) -> Self {
        let n = heights.len();
        let mut bit = vec![0u64; n + 1];
        let mut total = 0u64;
        let mut normalized = Vec::with_capacity(n);

        for (idx, raw_h) in heights.into_iter().enumerate() {
            let h = raw_h.max(1);
            normalized.push(h);
            let h64 = u64::from(h);
            total = total.saturating_add(h64);
            let i = idx + 1;
            bit[i] = bit[i].saturating_add(h64);
            let parent = i + i.isolate_lowest_one();
            if parent <= n {
                bit[parent] = bit[parent].saturating_add(bit[i]);
            }
        }

        Self {
            heights: normalized,
            bit,
            total,
        }
    }

    /// Builds a Fenwick tree from floating-point row heights, sanitizing NaN/negative/zero values.
    pub fn from_f32_heights(heights: &[f32]) -> Self {
        let normalized: Vec<u32> = heights
            .iter()
            .map(|&h| {
                if h.is_finite() && h >= 1.0 {
                    h.round().min(u32::MAX as f32) as u32
                } else {
                    1
                }
            })
            .collect();
        Self::from_heights(normalized)
    }

    /// Builds a uniform-height Fenwick tree of `len` rows in **O(N)** time.
    pub fn uniform(len: usize, row_height: u32) -> Self {
        Self::from_heights(vec![row_height.max(1); len])
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.heights.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// Total document height in logical pixels in **O(1)** time.
    #[inline]
    pub fn total_height(&self) -> u64 {
        self.total
    }

    /// Height of a single row in **O(1)** time.
    #[inline]
    pub fn height_at(&self, index: usize) -> Option<u32> {
        self.heights.get(index).copied()
    }

    /// Updates the height of `index` in **O(log N)** time. Returns `false` if out of bounds.
    pub fn set_height(&mut self, index: usize, new_height: u32) -> bool {
        let Some(&old_h) = self.heights.get(index) else {
            return false;
        };
        let clamped = new_height.max(1);
        if clamped == old_h {
            return true;
        }
        self.heights[index] = clamped;
        let old64 = u64::from(old_h);
        let new64 = u64::from(clamped);
        let n = self.heights.len();
        let mut i = index + 1;
        if new64 > old64 {
            let diff = new64 - old64;
            self.total = self.total.saturating_add(diff);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_add(diff);
                i += i.isolate_lowest_one();
            }
        } else {
            let diff = old64 - new64;
            self.total = self.total.saturating_sub(diff);
            while i <= n {
                self.bit[i] = self.bit[i].saturating_sub(diff);
                i += i.isolate_lowest_one();
            }
        }
        true
    }

    /// Cumulative pixel height of rows `[0..count)` in **O(log N)** time.
    pub fn prefix_sum(&self, count: usize) -> u64 {
        let mut i = count.min(self.heights.len());
        let mut sum = 0u64;
        while i > 0 {
            sum = sum.saturating_add(self.bit[i]);
            i -= i.isolate_lowest_one();
        }
        sum
    }

    /// Binary-search / bit-lifting **O(log N)** lower bound: returns the smallest row index `r`
    /// whose bottom edge strictly exceeds `offset_px`, or `len()` when `offset_px >= total_height()`.
    pub fn lower_bound(&self, offset_px: u64) -> usize {
        let n = self.heights.len();
        if n == 0 {
            return 0;
        }
        if offset_px >= self.total {
            return n;
        }
        let mut idx = 0usize;
        let mut acc = 0u64;
        let mut step = 1usize << (usize::BITS - 1 - n.leading_zeros());
        while step > 0 {
            let next = idx + step;
            if next <= n {
                let candidate = acc.saturating_add(self.bit[next]);
                if candidate <= offset_px {
                    idx = next;
                    acc = candidate;
                }
            }
            step >>= 1;
        }
        idx
    }
}

/// Row height model supporting both zero-allocation uniform lists and variable-height Fenwick lists.
#[derive(Debug, Clone, PartialEq)]
pub enum RowHeights {
    /// Constant row height in logical pixels — strictly O(1) offset/slice math, zero allocation.
    Uniform { count: usize, row_height: f32 },
    /// Variable per-row heights backed by [`FenwickHeightTree`] — O(log N) prefix sums & lookups.
    Variable(FenwickHeightTree),
}

impl RowHeights {
    pub fn uniform(count: usize, row_height: f32) -> Self {
        let h = if row_height.is_finite() && row_height >= 1.0 {
            row_height
        } else {
            1.0
        };
        Self::Uniform {
            count,
            row_height: h,
        }
    }

    pub fn variable(heights: Vec<u32>) -> Self {
        Self::Variable(FenwickHeightTree::from_heights(heights))
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Uniform { count, .. } => *count,
            Self::Variable(tree) => tree.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_height(&self) -> f32 {
        match self {
            Self::Uniform { count, row_height } => (*count as f32) * *row_height,
            Self::Variable(tree) => tree.total_height() as f32,
        }
    }

    pub fn row_height(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { count, row_height } => {
                if index < *count {
                    *row_height
                } else {
                    0.0
                }
            }
            Self::Variable(tree) => tree.height_at(index).unwrap_or(0) as f32,
        }
    }

    pub fn offset_of(&self, index: usize) -> f32 {
        match self {
            Self::Uniform { count, row_height } => (index.min(*count) as f32) * *row_height,
            Self::Variable(tree) => tree.prefix_sum(index) as f32,
        }
    }

    pub fn index_at_offset(&self, offset_px: f32) -> usize {
        let len = self.len();
        if len == 0 || !offset_px.is_finite() || offset_px <= 0.0 {
            return 0;
        }
        match self {
            Self::Uniform { count, row_height } => {
                ((offset_px / *row_height).floor() as usize).min(*count)
            }
            Self::Variable(tree) => tree.lower_bound(offset_px.floor() as u64),
        }
    }
}

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
    pub start: usize,
    pub end: usize,
    pub top_spacer_px: f32,
    pub bottom_spacer_px: f32,
    pub total_height_px: f32,
    pub visible_count: usize,
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
    pub fn slice(&self) -> VirtualSlice {
        let len = self.heights.len();
        if len == 0 {
            return VirtualSlice {
                start: 0,
                end: 0,
                top_spacer_px: 0.0,
                bottom_spacer_px: 0.0,
                total_height_px: 0.0,
                visible_count: 0,
            };
        }

        let total = self.heights.total_height();
        let vp = if self.viewport_height.is_finite() && self.viewport_height > 0.0 {
            self.viewport_height
        } else {
            0.0
        };
        let max_scroll = (total - vp).max(0.0);
        let offset = if !self.scroll_offset.is_finite() || self.scroll_offset <= 0.0 {
            0.0
        } else {
            self.scroll_offset.min(max_scroll)
        };

        let first_visible = self.heights.index_at_offset(offset).min(len - 1);
        let end_px = offset + vp;
        let raw_end = self.heights.index_at_offset(end_px);
        let end_visible = if raw_end < len {
            let row_top = self.heights.offset_of(raw_end);
            if row_top < end_px || raw_end == first_visible {
                (raw_end + 1).min(len)
            } else {
                raw_end.max(first_visible + 1).min(len)
            }
        } else {
            len
        };

        let start = first_visible.saturating_sub(self.overscan);
        let end = end_visible.saturating_add(self.overscan).min(len);
        let top_spacer_px = self.heights.offset_of(start);
        let end_offset_px = self.heights.offset_of(end);
        let bottom_spacer_px = (total - end_offset_px).max(0.0);

        VirtualSlice {
            start,
            end,
            top_spacer_px,
            bottom_spacer_px,
            total_height_px: total,
            visible_count: end.saturating_sub(start),
        }
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

/// High-performance virtualized scroll list rendering only the visible slice plus overscan,
/// framed by `top_spacer` and `bottom_spacer` views and wired for keyboard navigation.
#[component]
pub fn VirtualList(
    heights: RowHeights,
    viewport_height: f32,
    render_row: Callback<(usize, bool), Element>,
    #[props(default = 3)] overscan: usize,
    #[props(default)] initial_scroll_offset: f32,
    #[props(default)] initial_selected: Option<usize>,
    #[props(default)] on_select: Option<EventHandler<usize>>,
    #[props(default)] test_id: Option<String>,
    #[props(default)] accessibility_label: Option<String>,
    #[props(default)] style: Option<String>,
) -> Element {
    let theme = use_theme();
    let mut state = use_signal(|| {
        let mut st = VirtualListState::new(heights.clone(), viewport_height, overscan);
        st.set_scroll_offset(initial_scroll_offset);
        st.selected_index = initial_selected;
        st
    });

    {
        let mut st = state.write_unchecked();
        if st.heights != heights {
            st.heights = heights;
        }
        if (st.viewport_height - viewport_height).abs() > f32::EPSILON {
            st.viewport_height = viewport_height.max(0.0);
        }
        st.overscan = overscan;
    }

    let snap = state.read().clone();
    let slice = snap.slice();
    let base_id = test_id.as_deref().unwrap_or("nve-virtual-list");
    let top_spacer_id = format!("{base_id}-top-spacer");
    let bottom_spacer_id = format!("{base_id}-bottom-spacer");

    let mut scroll_sb = StyleBuf::new();
    scroll_sb
        .width_pct(100.0)
        .height_px(viewport_height.max(1.0))
        .flex_direction("column")
        .overflow("hidden")
        .surface(SurfaceStyle {
            background: theme.palette.surface.as_ref(),
            border_width: 1.0,
            border_color: theme.palette.border.as_ref(),
            border_radius: theme.radii.md,
            opacity: 1.0,
            box_shadow: "none",
        });
    if let Some(ref extra) = style {
        scroll_sb.raw(extra);
    }
    let scroll_style = scroll_sb.finish();

    let mut top_sb = StyleBuf::new();
    top_sb
        .width_pct(100.0)
        .height_px(slice.top_spacer_px)
        .flex_shrink(0.0);
    let top_style = top_sb.finish();

    let mut bot_sb = StyleBuf::new();
    bot_sb
        .width_pct(100.0)
        .height_px(slice.bottom_spacer_px)
        .flex_shrink(0.0);
    let bottom_style = bot_sb.finish();

    let mut rows = Vec::with_capacity(slice.visible_count);
    for idx in slice.start..slice.end {
        let is_selected = snap.selected_index == Some(idx);
        let row_h = snap.heights.row_height(idx);
        let row_tid = format!("{base_id}-row-{idx}");
        let aria_selected = if is_selected { "true" } else { "false" };

        let mut row_sb = StyleBuf::new();
        row_sb
            .width_pct(100.0)
            .height_px(row_h)
            .flex_shrink(0.0)
            .flex_direction("row")
            .align_items("center")
            .surface(SurfaceStyle {
                background: if is_selected {
                    theme.palette.surface_subtle.as_ref()
                } else {
                    "transparent"
                },
                border_width: if is_selected { 1.0 } else { 0.0 },
                border_color: if is_selected {
                    theme.palette.border_focus.as_ref()
                } else {
                    "transparent"
                },
                border_radius: 0.0,
                opacity: 1.0,
                box_shadow: "none",
            });
        let row_style = row_sb.finish();

        let content = render_row.call((idx, is_selected));
        rows.push(rsx! {
            pressable {
                key: "{idx}",
                test_id: "{row_tid}",
                accessibility_role: "listitem",
                aria_selected: "{aria_selected}",
                tab_index: if is_selected { 0 } else { -1 },
                style: "{row_style}",
                onpress: move |_| {
                    state.write().selected_index = Some(idx);
                    if let Some(handler) = on_select {
                        handler.call(idx);
                    }
                },
                {content}
            }
        });
    }

    rsx! {
        scrollview {
            test_id: "{base_id}",
            accessibility_role: "list",
            accessibility_label: accessibility_label,
            tab_index: 0,
            style: "{scroll_style}",
            onscroll: move |ev: ScrollEvent| {
                let y = ev.data().y;
                state.write().set_scroll_offset(y);
            },
            onkeydown: move |ev: KeyEvent| {
                if let Some(next) = state.write().handle_key(ev.data().key.as_str())
                    && let Some(handler) = on_select
                {
                    handler.call(next);
                }
            },
            view {
                test_id: "{top_spacer_id}",
                style: "{top_style}",
            }
            {rows.into_iter()}
            view {
                test_id: "{bottom_spacer_id}",
                style: "{bottom_style}",
            }
        }
    }
}
