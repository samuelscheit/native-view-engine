//! Extended layout (`flex-basis`, `aspect-ratio`, `position`, CSS Grid, RTL logical layout)
//! and 2D [`Transform2D`] builders on [`TypedStyle`].

use nve_abi::Dimension;
use nve_components::layout_ext::{
    GridTrack, GridTrackPlacement, LayoutDirection, PositionMode,
};
use nve_components::tables::Display;
use nve_components::transform::Transform2D;

use super::TypedStyle;

impl TypedStyle {
    // --- Extended Flexbox & Sizing ---

    #[inline]
    pub fn flex_basis(mut self, dim: Dimension) -> Self {
        self.ext_mut().flex_basis = dim;
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
        }
        self
    }

    // --- Positioning & Edge Insets ---

    #[inline]
    pub fn position(mut self, mode: PositionMode) -> Self {
        self.ext_mut().position = mode;
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
    pub fn inset_top(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.top = dim;
        self
    }

    #[inline]
    pub fn inset_top_px(self, v: f32) -> Self {
        self.inset_top(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_right(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.right = dim;
        self
    }

    #[inline]
    pub fn inset_right_px(self, v: f32) -> Self {
        self.inset_right(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_bottom(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.bottom = dim;
        self
    }

    #[inline]
    pub fn inset_bottom_px(self, v: f32) -> Self {
        self.inset_bottom(Dimension::Points(Self::sanitize(v)))
    }

    #[inline]
    pub fn inset_left(mut self, dim: Dimension) -> Self {
        self.ext_mut().insets.left = dim;
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
        self
    }

    #[inline]
    pub fn display_grid(self) -> Self {
        self.display(Display::Grid)
    }

    #[inline]
    pub fn grid_cols(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.ext_mut().grid_mut().template_columns = tracks.into();
        self
    }

    #[inline]
    pub fn grid_rows(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self {
        self.ext_mut().grid_mut().template_rows = tracks.into();
        self
    }

    #[inline]
    pub fn column_gap(mut self, v: f32) -> Self {
        self.ext_mut().grid_mut().column_gap = Some(Self::sanitize(v).max(0.0));
        self
    }

    #[inline]
    pub fn row_gap(mut self, v: f32) -> Self {
        self.ext_mut().grid_mut().row_gap = Some(Self::sanitize(v).max(0.0));
        self
    }

    #[inline]
    pub fn grid_column(mut self, placement: GridTrackPlacement) -> Self {
        self.ext_mut().grid_item.column = placement;
        self
    }

    #[inline]
    pub fn grid_row(mut self, placement: GridTrackPlacement) -> Self {
        self.ext_mut().grid_item.row = placement;
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
        self
    }

    #[inline]
    pub fn padding_inline_end(mut self, v: f32) -> Self {
        self.ext_mut().padding_inline.end = Some(Self::sanitize(v).max(0.0));
        self
    }

    #[inline]
    pub fn margin_inline_start(mut self, v: f32) -> Self {
        self.ext_mut().margin_inline.start = Some(Self::sanitize(v));
        self
    }

    #[inline]
    pub fn margin_inline_end(mut self, v: f32) -> Self {
        self.ext_mut().margin_inline.end = Some(Self::sanitize(v));
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
