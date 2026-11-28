//! Extended zero-serialization layout primitives (`LayoutExt`): `flex-basis`, `aspect-ratio`,
//! `PositionMode` (`relative` / `absolute` / `sticky`) + edge insets, CSS Grid containers & item
//! placement, RTL logical layout (`LayoutDirection` + `LogicalInlineEdges`), and deterministic
//! `position: sticky` clamping math shared across native backends.

use nve_abi::{Dimension, EdgeInsets};

use crate::tables::Display;

#[inline]
fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Strictly typed CSS `position` mode (`#[repr(u8)]`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PositionMode {
    #[default]
    Relative = 0,
    Absolute = 1,
    Sticky = 2,
}

/// Optional CSS positioning edge insets (`top`, `right`, `bottom`, `left`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionInsets {
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,
}

impl Default for PositionInsets {
    #[inline]
    fn default() -> Self {
        Self::AUTO
    }
}

impl PositionInsets {
    pub const AUTO: Self = Self {
        top: Dimension::Auto,
        right: Dimension::Auto,
        bottom: Dimension::Auto,
        left: Dimension::Auto,
    };

    #[inline]
    pub fn is_auto(&self) -> bool {
        self.top == Dimension::Auto
            && self.right == Dimension::Auto
            && self.bottom == Dimension::Auto
            && self.left == Dimension::Auto
    }
}

/// Text & layout reading direction (`Ltr` or `Rtl`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LayoutDirection {
    #[default]
    Ltr = 0,
    Rtl = 1,
}

impl LayoutDirection {
    #[inline]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }

}

/// Direction-aware logical inline edges (`inline-start`, `inline-end`), resolving to physical
/// `left`/`right` under [`LayoutDirection::Ltr`] and `right`/`left` under [`LayoutDirection::Rtl`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LogicalInlineEdges {
    pub start: Option<f32>,
    pub end: Option<f32>,
}

impl LogicalInlineEdges {
    pub const EMPTY: Self = Self {
        start: None,
        end: None,
    };

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }

    /// Resolves logical `start`/`end` overrides onto `physical` according to `dir`.
    #[inline]
    pub fn resolve_into(&self, mut physical: EdgeInsets, dir: LayoutDirection) -> EdgeInsets {
        match dir {
            LayoutDirection::Ltr => {
                if let Some(s) = self.start {
                    physical.left = sanitize(s);
                }
                if let Some(e) = self.end {
                    physical.right = sanitize(e);
                }
            }
            LayoutDirection::Rtl => {
                if let Some(s) = self.start {
                    physical.right = sanitize(s);
                }
                if let Some(e) = self.end {
                    physical.left = sanitize(e);
                }
            }
        }
        physical
    }
}

/// Atomic track sizing unit used inside [`GridTrack::MinMax`] and [`GridSingleTrack`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrackUnit {
    Auto,
    Px(f32),
    Percent(f32),
    Fr(f32),
    MinContent,
    MaxContent,
}

/// Non-recursive single CSS Grid track sizing function (valid inside [`GridTrack::Repeat`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridSingleTrack {
    Auto,
    Px(f32),
    Percent(f32),
    Fr(f32),
    MinMax(GridTrackUnit, GridTrackUnit),
}

impl GridSingleTrack {
    #[inline]
    pub fn px(v: f32) -> Self {
        Self::Px(sanitize(v).max(0.0))
    }

    #[inline]
    pub fn percent(v: f32) -> Self {
        Self::Percent(sanitize(v).max(0.0))
    }

    #[inline]
    pub fn fr(v: f32) -> Self {
        Self::Fr(sanitize(v).max(0.0))
    }

    #[inline]
    pub const fn auto() -> Self {
        Self::Auto
    }

    #[inline]
    pub const fn minmax(min: GridTrackUnit, max: GridTrackUnit) -> Self {
        Self::MinMax(min, max)
    }

    #[inline]
    pub fn minmax_zero_fr(fr: f32) -> Self {
        Self::MinMax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(sanitize(fr).max(0.0)))
    }

}

/// Strictly typed CSS Grid template track component (`Copy`, zero per-track heap allocation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    Auto,
    Px(f32),
    Percent(f32),
    Fr(f32),
    MinMax(GridTrackUnit, GridTrackUnit),
    Repeat(u16, GridSingleTrack),
}

impl GridTrack {
    #[inline]
    pub const fn auto() -> Self {
        Self::Auto
    }

    #[inline]
    pub fn px(v: f32) -> Self {
        Self::Px(sanitize(v).max(0.0))
    }

    #[inline]
    pub fn percent(v: f32) -> Self {
        Self::Percent(sanitize(v).max(0.0))
    }

    #[inline]
    pub fn fr(v: f32) -> Self {
        Self::Fr(sanitize(v).max(0.0))
    }

    #[inline]
    pub const fn minmax(min: GridTrackUnit, max: GridTrackUnit) -> Self {
        Self::MinMax(min, max)
    }

    /// Convenience for blowout-safe `minmax(0, <fr>fr)` tracks.
    #[inline]
    pub fn minmax_zero_fr(fr: f32) -> Self {
        Self::MinMax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(sanitize(fr).max(0.0)))
    }

    #[inline]
    pub fn repeat(count: u16, track: GridSingleTrack) -> Self {
        Self::Repeat(count.max(1), track)
    }

    /// Convenience for `repeat(count, <fr>fr)`.
    #[inline]
    pub fn repeat_fr(count: u16, fr: f32) -> Self {
        Self::Repeat(count.max(1), GridSingleTrack::fr(fr))
    }

    /// Convenience for blowout-safe `repeat(count, minmax(0, 1fr))`.
    #[inline]
    pub fn repeat_minmax_zero_fr(count: u16) -> Self {
        Self::Repeat(count.max(1), GridSingleTrack::minmax_zero_fr(1.0))
    }

}

/// Strictly typed CSS Grid line endpoint (`auto`, signed 1-based line index, or `span N`).
pub use nve_store::GridLine;

/// Start/end placement along a single CSS Grid axis (`grid-column` or `grid-row`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GridTrackPlacement {
    pub start: GridLine,
    pub end: GridLine,
}

impl GridTrackPlacement {
    pub const AUTO: Self = Self {
        start: GridLine::Auto,
        end: GridLine::Auto,
    };

    #[inline]
    pub const fn auto() -> Self {
        Self::AUTO
    }

    #[inline]
    pub const fn line(start: i16) -> Self {
        Self {
            start: GridLine::Line(start),
            end: GridLine::Auto,
        }
    }

    #[inline]
    pub const fn range(start: i16, end: i16) -> Self {
        Self {
            start: GridLine::Line(start),
            end: GridLine::Line(end),
        }
    }

    #[inline]
    pub const fn span(count: u16) -> Self {
        Self {
            start: GridLine::Span(if count == 0 { 1 } else { count }),
            end: GridLine::Auto,
        }
    }

    #[inline]
    pub const fn line_span(start: i16, count: u16) -> Self {
        Self {
            start: GridLine::Line(start),
            end: GridLine::Span(if count == 0 { 1 } else { count }),
        }
    }

    #[inline]
    pub const fn is_auto(self) -> bool {
        matches!((self.start, self.end), (GridLine::Auto, GridLine::Auto))
    }

}

/// Grid item placement (`grid-column` + `grid-row`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GridItemPlacement {
    pub column: GridTrackPlacement,
    pub row: GridTrackPlacement,
}

impl GridItemPlacement {
    #[inline]
    pub const fn is_auto(self) -> bool {
        self.column.is_auto() && self.row.is_auto()
    }
}

/// CSS Grid container track templates and independent axis gaps (allocated only on grid nodes).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridLayout {
    pub template_columns: Vec<GridTrack>,
    pub template_rows: Vec<GridTrack>,
    pub column_gap: Option<f32>,
    pub row_gap: Option<f32>,
}

impl GridLayout {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.template_columns.is_empty()
            && self.template_rows.is_empty()
            && self.column_gap.is_none()
            && self.row_gap.is_none()
    }
}

/// Extended zero-serialization layout metadata held lazily (`Option<Box<LayoutExt>>`) on nodes
/// that opt into `flex-basis`, `aspect-ratio`, `position` + insets, CSS Grid, or RTL logical edges.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayoutExt {
    pub display: Display,
    pub flex_basis: Dimension,
    pub aspect_ratio: Option<f32>,
    pub position: PositionMode,
    pub insets: PositionInsets,
    pub direction: Option<LayoutDirection>,
    pub padding_inline: LogicalInlineEdges,
    pub margin_inline: LogicalInlineEdges,
    pub grid: Option<Box<GridLayout>>,
    pub grid_item: GridItemPlacement,
}

impl LayoutExt {
    #[inline]
    pub fn is_default(&self) -> bool {
        self.display == Display::Flex
            && self.flex_basis == Dimension::Auto
            && self.aspect_ratio.is_none()
            && self.position == PositionMode::Relative
            && self.insets.is_auto()
            && self.direction.is_none()
            && self.padding_inline.is_empty()
            && self.margin_inline.is_empty()
            && self.grid.as_deref().is_none_or(GridLayout::is_empty)
            && self.grid_item.is_auto()
    }

    #[inline]
    pub fn grid_mut(&mut self) -> &mut GridLayout {
        self.grid.get_or_insert_with(|| Box::new(GridLayout::default()))
    }
}

/// Resolves a sticky edge threshold [`Dimension`] against `viewport_extent`.
#[inline]
fn resolve_sticky_inset(dim: Dimension, viewport_extent: f32) -> Option<f32> {
    match dim {
        Dimension::Auto => None,
        Dimension::Points(v) if v.is_finite() => Some(v),
        Dimension::Percent(p) if p.is_finite() => Some(viewport_extent * (p / 100.0)),
        _ => None,
    }
}

/// Computes deterministic CSS `position: sticky` offset `(dx, dy)` for a sticky element in
/// content-space coordinates given:
/// - `item_rect`: `(x, y, width, height)` of the sticky element in normal flow (pre-scroll)
/// - `container_rect`: `(x, y, width, height)` of the sticky element's containing block (pre-scroll)
/// - `scroll_viewport`: `(x, y, width, height)` of the nearest scrolling ancestor viewport
/// - `scroll_offset`: `(scroll_x, scroll_y)` of that scrolling ancestor
/// - `insets`: sticky threshold insets (`top`, `right`, `bottom`, `left`)
///
/// The returned `(dx, dy)` is added on top of the normal scroll shift `(item - scroll + delta)`.
pub fn compute_sticky_offset(
    item_rect: (f32, f32, f32, f32),
    container_rect: (f32, f32, f32, f32),
    scroll_viewport: (f32, f32, f32, f32),
    scroll_offset: (f32, f32),
    insets: PositionInsets,
) -> (f32, f32) {
    let (ix, iy, iw, ih) = item_rect;
    let (cx, cy, cw, ch) = container_rect;
    let (vx, vy, vw, vh) = scroll_viewport;
    let (sx, sy) = scroll_offset;

    let vis_top = vy + sy;
    let vis_bottom = vy + vh + sy;
    let vis_left = vx + sx;
    let vis_right = vx + vw + sx;

    let cb_min_y = cy;
    let cb_max_y = (cy + ch - ih).max(cb_min_y);
    let dy = if let Some(top) = resolve_sticky_inset(insets.top, vh) {
        let sticky_y = vis_top + top;
        let clamped_y = sticky_y.clamp(iy, cb_max_y.max(iy));
        clamped_y - iy
    } else if let Some(bottom) = resolve_sticky_inset(insets.bottom, vh) {
        let sticky_y = vis_bottom - bottom - ih;
        let clamped_y = sticky_y.clamp(cb_min_y.min(iy), iy);
        clamped_y - iy
    } else {
        0.0
    };

    let cb_min_x = cx;
    let cb_max_x = (cx + cw - iw).max(cb_min_x);
    let dx = if let Some(left) = resolve_sticky_inset(insets.left, vw) {
        let sticky_x = vis_left + left;
        let clamped_x = sticky_x.clamp(ix, cb_max_x.max(ix));
        clamped_x - ix
    } else if let Some(right) = resolve_sticky_inset(insets.right, vw) {
        let sticky_x = vis_right - right - iw;
        let clamped_x = sticky_x.clamp(cb_min_x.min(ix), ix);
        clamped_x - ix
    } else {
        0.0
    };

    (dx, dy)
}
