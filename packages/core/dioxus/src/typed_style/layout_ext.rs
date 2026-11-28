//! Extended layout (`flex-basis`, `aspect-ratio`, `position`, CSS Grid, RTL logical layout)
//! and 2D [`Transform2D`] builders on [`TypedStyle`].

use nve_abi::Dimension;
use nve_tables::layout_ext::{
    GridTrack, GridTrackPlacement, LayoutDirection, PositionMode,
};
use nve_tables::tables::Display;
use nve_tables::transform::Transform2D;

use super::merge::{
    E_ASPECT_RATIO, E_COL_GAP, E_DIRECTION, E_DISPLAY, E_FLEX_BASIS, E_GRID_COLS, E_GRID_COL_ITEM,
    E_GRID_ROWS, E_GRID_ROW_ITEM, E_INSET_BOTTOM, E_INSET_LEFT, E_INSET_RIGHT, E_INSET_TOP,
    E_MAR_INLINE_END, E_MAR_INLINE_START, E_PAD_INLINE_END, E_PAD_INLINE_START, E_POSITION,
    E_ROW_GAP,
};
use super::TypedStyle;

impl TypedStyle {
    // --- Extended Flexbox & Sizing ---

    #[inline]
    pub fn flex_basis(mut self, dim: Dimension) -> Self {
        self.ext_mut().flex_basis = dim;
        self.layout_ext_mask |= E_FLEX_BASIS;
        self
    }

    #[inline]
    pub fn flex_basis_px(self, v: f32) -> Self {
        self.flex_basis(Dimension::Points(Self::sanitize(v).max(0.0)))
    }

    #[inline]
    pub fn flex_basis_pct(self, v: f32) -> Self {
        self.flex_basis(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0)))
    }

    #[inline]
    pub fn aspect_ratio(mut self, ratio: f32) -> Self {
        if ratio.is_finite() && ratio > 0.0 {
            self.ext_mut().aspect_ratio = Some(ratio);
            self.layout_ext_mask |= E_ASPECT_RATIO;
        }
        self
    }

    // --- Positioning & Edge Insets ---

    #[inline]
    pub fn position(mut self, mode: PositionMode) -> Self {
        self.ext_mut().position = mode;
        self.layout_ext_mask |= E_POSITION;
        self
    }

    #[inline]
    pub fn position_relative(self) -> Self {
        self.position(PositionMode::Relative)
    }

    #[inline]
    pub fn position_absolute(self) -> Self {
        self.position(PositionMode::Absolute)
    }

    #[inline]
    pub fn position_sticky(self) -> Self {
        self.position(PositionMode::Sticky)
    }

    #[inline]
    pub fn position_fixed(self) -> Self {
        self.position(PositionMode::Fixed)
    }

    #[inline]
    pub fn inset_top(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.top = dim;
        self.layout_ext_mask |= E_INSET_TOP;
        self
    }

    #[inline]
    pub fn inset_top_px(self, v: f32) -> Self {
        self.inset_top(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_right(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.right = dim;
        self.layout_ext_mask |= E_INSET_RIGHT;
        self
    }

    #[inline]
    pub fn inset_right_px(self, v: f32) -> Self {
        self.inset_right(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_bottom(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.bottom = dim;
        self.layout_ext_mask |= E_INSET_BOTTOM;
        self
    }

    #[inline]
    pub fn inset_bottom_px(self, v: f32) -> Self {
        self.inset_bottom(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_left(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.left = dim;
        self.layout_ext_mask |= E_INSET_LEFT;
        self
    }

    #[inline]
    pub fn inset_left_px(self, v: f32) -> Self {
        self.inset_left(Dimension::Points(Self::sanitize(v)))
    }

    // --- CSS Grid ---

    #[inline]
    pub fn display(mut self, display: Display) -> Self {
        self.ext_mut().display = display;
        self.layout_ext_mask |= E_DISPLAY;
        self
    }

    #[inline]
    pub fn display_grid(self) -> Self {
        self.display(Display::Grid)
    }

    #[inline]
    pub fn grid_cols(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.ext_mut().grid_mut().template_columns = tracks.into();
        self.layout_ext_mask |= E_GRID_COLS;
        self
    }

    #[inline]
    pub fn grid_rows(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.ext_mut().grid_mut().template_rows = tracks.into();
        self.layout_ext_mask |= E_GRID_ROWS;
        self
    }

    #[inline]
    pub fn column_gap(mut self, v: f32) -> Self {
        self.ext_mut().grid_mut().column_gap = Some(Self::sanitize(v).max(0.0));
        self.layout_ext_mask |= E_COL_GAP;
        self
    }

    #[inline]
    pub fn row_gap(mut self, v: f32) -> Self {
        self.ext_mut().grid_mut().row_gap = Some(Self::sanitize(v).max(0.0));
        self.layout_ext_mask |= E_ROW_GAP;
        self
    }

    #[inline]
    pub fn grid_column(mut self, placement: GridTrackPlacement) -> Self {
        self.ext_mut().grid_item.column = placement;
        self.layout_ext_mask |= E_GRID_COL_ITEM;
        self
    }

    #[inline]
    pub fn grid_row(mut self, placement: GridTrackPlacement) -> Self {
        self.ext_mut().grid_item.row = placement;
        self.layout_ext_mask |= E_GRID_ROW_ITEM;
        self
    }

    #[inline]
    pub fn grid_column_span(self, span: u16) -> Self {
        self.grid_column(GridTrackPlacement::span(span))
    }

    #[inline]
    pub fn grid_row_span(self, span: u16) -> Self {
        self.grid_row(GridTrackPlacement::span(span))
    }

    // --- RTL Logical Layout ---

    #[inline]
    pub fn direction(mut self, dir: LayoutDirection) -> Self {
        self.ext_mut().direction = Some(dir);
        self.layout_ext_mask |= E_DIRECTION;
        self
    }

    #[inline]
    pub fn rtl(self) -> Self {
        self.direction(LayoutDirection::Rtl)
    }

    #[inline]
    pub fn ltr(self) -> Self {
        self.direction(LayoutDirection::Ltr)
    }

    #[inline]
    pub fn padding_inline_start(mut self, v: f32) -> Self {
        self.ext_mut().padding_inline.start = Some(Self::sanitize(v).max(0.0));
        self.layout_ext_mask |= E_PAD_INLINE_START;
        self
    }

    #[inline]
    pub fn padding_inline_end(mut self, v: f32) -> Self {
        self.ext_mut().padding_inline.end = Some(Self::sanitize(v).max(0.0));
        self.layout_ext_mask |= E_PAD_INLINE_END;
        self
    }

    #[inline]
    pub fn margin_inline_start(mut self, v: f32) -> Self {
        self.ext_mut().margin_inline.start = Some(Self::sanitize(v));
        self.layout_ext_mask |= E_MAR_INLINE_START;
        self
    }

    #[inline]
    pub fn margin_inline_end(mut self, v: f32) -> Self {
        self.ext_mut().margin_inline.end = Some(Self::sanitize(v));
        self.layout_ext_mask |= E_MAR_INLINE_END;
        self
    }

    // --- Typed 2D Transform ---

    #[inline]
    pub fn transform(mut self, tf: Transform2D) -> Self {
        self.transform = Some(tf);
        self
    }

    #[inline]
    pub fn translate(mut self, x: f32, y: f32) -> Self {
        let tf = self
            .transform
            .unwrap_or(Transform2D::IDENTITY)
            .with_translate(x, y);
        self.transform = Some(tf);
        self
    }

    #[inline]
    pub fn scale(mut self, s: f32) -> Self {
        let tf = self
            .transform
            .unwrap_or(Transform2D::IDENTITY)
            .with_scale(s);
        self.transform = Some(tf);
        self
    }

    #[inline]
    pub fn scale_xy(mut self, sx: f32, sy: f32) -> Self {
        let tf = self
            .transform
            .unwrap_or(Transform2D::IDENTITY)
            .with_scale_xy(sx, sy);
        self.transform = Some(tf);
        self
    }

    #[inline]
    pub fn rotate(mut self, deg: f32) -> Self {
        let tf = self
            .transform
            .unwrap_or(Transform2D::IDENTITY)
            .with_rotate(deg);
        self.transform = Some(tf);
        self
    }

    #[inline]
    pub fn transform_origin(mut self, ox: f32, oy: f32) -> Self {
        let tf = self
            .transform
            .unwrap_or(Transform2D::IDENTITY)
            .with_origin(ox, oy);
        self.transform = Some(tf);
        self
    }
}
