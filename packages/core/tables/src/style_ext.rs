//! Ergonomic style & layout builder extension ([`StylePatchExt`]) directly on
//! [`nve_store::RowPatch`] (and any wrapper exposing [`StylePatchExt::row_patch_mut`]).
//!
//! Every method writes typed masked word ops directly targeting core `tables::*` rows (`Layout`,
//! `Visual`, `Typography`, `Transform`, `Interaction`, `GridLayout`, `GridItem`) with zero
//! intermediate patch structs or codecs.

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_store::{Length, ResourceId, Rgba, RowPatch};

use crate::css_values::dim_to_length;
use crate::interaction::CursorKind;
use crate::layout_ext::{GridTrackPlacement, LayoutDirection, LayoutExt, PositionMode};
use crate::snapshots::{
    apply_patch_to_layout, apply_patch_to_layout_ext, apply_patch_to_visual, LayoutSnapshot,
    VisualSnapshot,
};
use crate::tables::{
    AlignItems, Direction, Display, FlexWrap, GridItem, GridLayout, Interaction, JustifyContent,
    Layout, Overflow, Position, TextAlign, Transform, Typography, Visual,
};
use crate::transform::Transform2D;

#[inline]
fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

/// Fluent core-table style & layout builders implemented directly on [`RowPatch`].
pub trait StylePatchExt: Sized {
    fn row_patch(&self) -> &RowPatch;
    fn row_patch_mut(&mut self) -> &mut RowPatch;

    #[inline]
    fn flex_direction(mut self, dir: FlexDirection) -> Self {
        self.row_patch_mut().set(Layout::FLEX_DIRECTION, dir);
        self
    }
    #[inline]
    fn flex_row(self) -> Self { self.flex_direction(FlexDirection::Row) }
    #[inline]
    fn flex_col(self) -> Self { self.flex_direction(FlexDirection::Column) }

    #[inline]
    fn justify_content(mut self, j: FlexJustify) -> Self {
        self.row_patch_mut()
            .set(Layout::JUSTIFY_CONTENT, JustifyContent::from(j));
        self
    }

    #[inline]
    fn align_items(mut self, a: FlexAlign) -> Self {
        self.row_patch_mut()
            .set(Layout::ALIGN_ITEMS, AlignItems::from(a));
        self
    }

    #[inline]
    fn flex_grow(mut self, v: f32) -> Self {
        self.row_patch_mut().set(Layout::FLEX_GROW, sanitize(v).max(0.0));
        self
    }
    #[inline]
    fn flex_shrink(mut self, v: f32) -> Self {
        self.row_patch_mut().set(Layout::FLEX_SHRINK, sanitize(v).max(0.0));
        self
    }
    #[inline]
    fn gap(mut self, v: f32) -> Self {
        let len = Length::points(sanitize(v).max(0.0));
        self.row_patch_mut()
            .set(Layout::ROW_GAP, len)
            .set(Layout::COLUMN_GAP, len);
        self
    }
    #[inline]
    fn column_gap(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::COLUMN_GAP, Length::points(sanitize(v).max(0.0)));
        self
    }
    #[inline]
    fn row_gap(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::ROW_GAP, Length::points(sanitize(v).max(0.0)));
        self
    }

    #[inline]
    fn width(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::WIDTH, dim_to_length(d));
        self
    }
    #[inline]
    fn width_px(self, v: f32) -> Self { self.width(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn width_pct(self, v: f32) -> Self { self.width(Dimension::Percent(sanitize(v).clamp(0.0, 100.0))) }
    #[inline]
    fn width_auto(self) -> Self { self.width(Dimension::Auto) }

    #[inline]
    fn height(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::HEIGHT, dim_to_length(d));
        self
    }
    #[inline]
    fn height_px(self, v: f32) -> Self { self.height(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn height_pct(self, v: f32) -> Self { self.height(Dimension::Percent(sanitize(v).clamp(0.0, 100.0))) }
    #[inline]
    fn height_auto(self) -> Self { self.height(Dimension::Auto) }

    #[inline]
    fn min_width(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::MIN_WIDTH, dim_to_length(d));
        self
    }
    #[inline]
    fn min_width_px(self, v: f32) -> Self { self.min_width(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn max_width(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::MAX_WIDTH, dim_to_length(d));
        self
    }
    #[inline]
    fn max_width_px(self, v: f32) -> Self { self.max_width(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn min_height(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::MIN_HEIGHT, dim_to_length(d));
        self
    }
    #[inline]
    fn min_height_px(self, v: f32) -> Self { self.min_height(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn max_height(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::MAX_HEIGHT, dim_to_length(d));
        self
    }
    #[inline]
    fn max_height_px(self, v: f32) -> Self { self.max_height(Dimension::Points(sanitize(v).max(0.0))) }

    #[inline]
    fn padding(self, v: f32) -> Self { self.padding_insets(EdgeInsets::all(sanitize(v).max(0.0))) }
    #[inline]
    fn padding_xy(self, h: f32, v: f32) -> Self {
        self.padding_insets(EdgeInsets::symmetric(sanitize(v).max(0.0), sanitize(h).max(0.0)))
    }
    #[inline]
    fn padding_insets(mut self, insets: EdgeInsets) -> Self {
        self.row_patch_mut()
            .set(Layout::PADDING_TOP, Length::points(sanitize(insets.top).max(0.0)))
            .set(Layout::PADDING_RIGHT, Length::points(sanitize(insets.right).max(0.0)))
            .set(Layout::PADDING_BOTTOM, Length::points(sanitize(insets.bottom).max(0.0)))
            .set(Layout::PADDING_LEFT, Length::points(sanitize(insets.left).max(0.0)))
            .set(Layout::PADDING_INLINE_LOGICAL, false);
        self
    }

    #[inline]
    fn margin(self, v: f32) -> Self { self.margin_insets(EdgeInsets::all(sanitize(v))) }
    #[inline]
    fn margin_xy(self, h: f32, v: f32) -> Self {
        self.margin_insets(EdgeInsets::symmetric(sanitize(v), sanitize(h)))
    }
    #[inline]
    fn margin_insets(mut self, insets: EdgeInsets) -> Self {
        self.row_patch_mut()
            .set(Layout::MARGIN_TOP, Length::points(sanitize(insets.top)))
            .set(Layout::MARGIN_RIGHT, Length::points(sanitize(insets.right)))
            .set(Layout::MARGIN_BOTTOM, Length::points(sanitize(insets.bottom)))
            .set(Layout::MARGIN_LEFT, Length::points(sanitize(insets.left)))
            .set(Layout::MARGIN_INLINE_LOGICAL, false);
        self
    }

    #[inline]
    fn bg(mut self, c: ColorRgba) -> Self {
        self.row_patch_mut().set(Visual::BACKGROUND, Rgba(c.to_u32()));
        self
    }
    #[inline]
    fn background(self, c: ColorRgba) -> Self { self.bg(c) }
    #[inline]
    fn background_fill_id(mut self, id: ResourceId) -> Self {
        self.row_patch_mut().set_res_id(Visual::BACKGROUND_FILL, id);
        self
    }

    #[inline]
    fn color(mut self, c: ColorRgba) -> Self {
        self.row_patch_mut().set(Typography::COLOR, Rgba(c.to_u32()));
        self
    }
    #[inline]
    fn text_color(self, c: ColorRgba) -> Self { self.color(c) }

    #[inline]
    fn border(mut self, w: f32, c: ColorRgba) -> Self {
        self.row_patch_mut()
            .set(Visual::BORDER_WIDTH, sanitize(w).max(0.0))
            .set(Visual::BORDER_COLOR, Rgba(c.to_u32()));
        self
    }
    #[inline]
    fn border_width(mut self, w: f32) -> Self {
        self.row_patch_mut().set(Visual::BORDER_WIDTH, sanitize(w).max(0.0));
        self
    }
    #[inline]
    fn border_color(mut self, c: ColorRgba) -> Self {
        self.row_patch_mut().set(Visual::BORDER_COLOR, Rgba(c.to_u32()));
        self
    }
    #[inline]
    fn radius(mut self, r: f32) -> Self {
        self.row_patch_mut().set(Visual::BORDER_RADIUS, sanitize(r).max(0.0));
        self
    }
    #[inline]
    fn border_radius(self, r: f32) -> Self { self.radius(r) }
    #[inline]
    fn opacity(mut self, o: f32) -> Self {
        self.row_patch_mut().set(Visual::OPACITY, sanitize(o).clamp(0.0, 1.0));
        self
    }
    #[inline]
    fn font_size(mut self, px: f32) -> Self {
        self.row_patch_mut().set(Typography::FONT_SIZE, sanitize(px).max(1.0));
        self
    }
    #[inline]
    fn z_index(mut self, z: i32) -> Self {
        self.row_patch_mut().set(Visual::Z_INDEX, z);
        self
    }
    #[inline]
    fn flex_wrap(mut self, wrap: FlexWrap) -> Self {
        self.row_patch_mut().set(Layout::FLEX_WRAP, wrap);
        self
    }
    #[inline]
    fn overflow(mut self, ov: Overflow) -> Self {
        self.row_patch_mut()
            .set(Visual::OVERFLOW_X, ov)
            .set(Visual::OVERFLOW_Y, ov);
        self
    }
    #[inline]
    fn text_align(mut self, align: TextAlign) -> Self {
        self.row_patch_mut().set(Typography::TEXT_ALIGN, align);
        self
    }
    #[inline]
    fn font_weight(mut self, weight: u16) -> Self {
        self.row_patch_mut().set(Typography::FONT_WEIGHT, weight.clamp(1, 1000));
        self
    }
    #[inline]
    fn display(mut self, display: Display) -> Self {
        self.row_patch_mut().set(Layout::DISPLAY, display);
        self
    }
    #[inline]
    fn display_grid(self) -> Self { self.display(Display::Grid) }
    #[inline]
    fn cursor(mut self, cursor: CursorKind) -> Self {
        self.row_patch_mut().set(Interaction::CURSOR, cursor);
        self
    }
    #[inline]
    fn flex_basis(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::FLEX_BASIS, dim_to_length(d));
        self
    }
    #[inline]
    fn flex_basis_px(self, v: f32) -> Self { self.flex_basis(Dimension::Points(sanitize(v).max(0.0))) }
    #[inline]
    fn flex_basis_pct(self, v: f32) -> Self {
        self.flex_basis(Dimension::Percent(sanitize(v).clamp(0.0, 100.0)))
    }
    #[inline]
    fn aspect_ratio(mut self, ratio: f32) -> Self {
        if ratio.is_finite() && ratio > 0.0 {
            self.row_patch_mut().set(Layout::ASPECT_RATIO, Some(ratio));
        }
        self
    }
    #[inline]
    fn position(mut self, mode: PositionMode) -> Self {
        let p = match mode {
            PositionMode::Relative => Position::Relative,
            PositionMode::Absolute => Position::Absolute,
            PositionMode::Sticky => Position::Sticky,
        };
        self.row_patch_mut().set(Layout::POSITION, p);
        self
    }
    #[inline]
    fn inset_top(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::INSET_TOP, dim_to_length(d));
        self
    }
    #[inline]
    fn inset_right(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::INSET_RIGHT, dim_to_length(d));
        self
    }
    #[inline]
    fn inset_bottom(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::INSET_BOTTOM, dim_to_length(d));
        self
    }
    #[inline]
    fn inset_left(mut self, d: Dimension) -> Self {
        self.row_patch_mut().set(Layout::INSET_LEFT, dim_to_length(d));
        self
    }
    #[inline]
    fn grid_cols_id(mut self, id: ResourceId) -> Self {
        self.row_patch_mut().set_res_id(GridLayout::TEMPLATE_COLUMNS, id);
        self
    }
    #[inline]
    fn grid_rows_id(mut self, id: ResourceId) -> Self {
        self.row_patch_mut().set_res_id(GridLayout::TEMPLATE_ROWS, id);
        self
    }
    #[inline]
    fn grid_column(mut self, p: GridTrackPlacement) -> Self {
        self.row_patch_mut()
            .set(GridItem::COLUMN_START, p.start)
            .set(GridItem::COLUMN_END, p.end);
        self
    }
    #[inline]
    fn grid_row(mut self, p: GridTrackPlacement) -> Self {
        self.row_patch_mut()
            .set(GridItem::ROW_START, p.start)
            .set(GridItem::ROW_END, p.end);
        self
    }
    #[inline]
    fn grid_column_span(self, span: u16) -> Self { self.grid_column(GridTrackPlacement::span(span)) }
    #[inline]
    fn grid_row_span(self, span: u16) -> Self { self.grid_row(GridTrackPlacement::span(span)) }
    #[inline]
    fn direction(mut self, dir: LayoutDirection) -> Self {
        let d = match dir {
            LayoutDirection::Ltr => Direction::Ltr,
            LayoutDirection::Rtl => Direction::Rtl,
        };
        self.row_patch_mut().set(Layout::DIRECTION, d);
        self
    }
    #[inline]
    fn padding_inline_start(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::PADDING_INLINE_LOGICAL, true)
            .set(Layout::PADDING_LEFT, Length::points(sanitize(v).max(0.0)));
        self
    }
    #[inline]
    fn padding_inline_end(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::PADDING_INLINE_LOGICAL, true)
            .set(Layout::PADDING_RIGHT, Length::points(sanitize(v).max(0.0)));
        self
    }
    #[inline]
    fn margin_inline_start(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::MARGIN_INLINE_LOGICAL, true)
            .set(Layout::MARGIN_LEFT, Length::points(sanitize(v)));
        self
    }
    #[inline]
    fn margin_inline_end(mut self, v: f32) -> Self {
        self.row_patch_mut()
            .set(Layout::MARGIN_INLINE_LOGICAL, true)
            .set(Layout::MARGIN_RIGHT, Length::points(sanitize(v)));
        self
    }
    #[inline]
    fn transform(mut self, tf: Transform2D) -> Self {
        self.row_patch_mut()
            .set(Transform::TRANSLATE_X, tf.translate_x)
            .set(Transform::TRANSLATE_Y, tf.translate_y)
            .set(Transform::SCALE_X, tf.scale_x)
            .set(Transform::SCALE_Y, tf.scale_y)
            .set(Transform::ROTATE_DEG, tf.rotate_deg)
            .set(Transform::ORIGIN_X, tf.origin_x)
            .set(Transform::ORIGIN_Y, tf.origin_y);
        self
    }
    #[inline]
    fn translate(mut self, x: f32, y: f32) -> Self {
        self.row_patch_mut()
            .set(Transform::TRANSLATE_X, sanitize(x))
            .set(Transform::TRANSLATE_Y, sanitize(y));
        self
    }
    #[inline]
    fn scale(mut self, s: f32) -> Self {
        let s = if s.is_finite() { s } else { 1.0 };
        self.row_patch_mut()
            .set(Transform::SCALE_X, s)
            .set(Transform::SCALE_Y, s);
        self
    }
    #[inline]
    fn rotate(mut self, deg: f32) -> Self {
        self.row_patch_mut().set(Transform::ROTATE_DEG, sanitize(deg));
        self
    }

    #[inline]
    fn apply_style_to(&self, style: &mut VisualSnapshot) {
        apply_patch_to_visual(self.row_patch(), style);
    }
    #[inline]
    fn apply_layout_to(&self, layout: &mut LayoutSnapshot) {
        apply_patch_to_layout(self.row_patch(), layout);
    }
    #[inline]
    fn apply_layout_ext_to(&self, ext: &mut LayoutExt) {
        apply_patch_to_layout_ext(self.row_patch(), ext);
    }
    #[inline]
    fn apply_to(&self, layout: &mut LayoutSnapshot, style: &mut VisualSnapshot) {
        self.apply_layout_to(layout);
        self.apply_style_to(style);
    }
}

impl StylePatchExt for RowPatch {
    #[inline(always)]
    fn row_patch(&self) -> &RowPatch { self }
    #[inline(always)]
    fn row_patch_mut(&mut self) -> &mut RowPatch { self }
}
