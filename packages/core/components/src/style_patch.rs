//! Unified sparse, strictly typed style & layout patch (`StylePatch`) shared across
//! responsive `@media` rules (`ResponsiveRule`) and interactive pseudo-state rules
//! (`StateStyleRule`) with zero duplication and zero string serialization.

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};

/// Resolved value-type snapshot of core visual + typography properties (used by motion
/// interpolation and pseudo-state resolution; derived directly from `NodeStore` word tables).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisualSnapshot {
    pub background: ColorRgba,
    pub text_color: ColorRgba,
    pub border_color: ColorRgba,
    pub border_width: f32,
    pub border_radius: f32,
    pub opacity: f32,
    pub font_size: f32,
    pub z_index: i32,
}

impl VisualSnapshot {
    /// Keeps only inherited text styling: the snapshot of a node that generates no box.
    #[inline]
    pub fn without_box_paint(self) -> Self {
        Self {
            text_color: self.text_color,
            font_size: self.font_size,
            ..Self::default()
        }
    }
}

impl Default for VisualSnapshot {
    fn default() -> Self {
        Self {
            background: ColorRgba::TRANSPARENT,
            text_color: ColorRgba::BLACK,
            border_color: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_radius: 0.0,
            opacity: 1.0,
            font_size: 16.0,
            z_index: 0,
        }
    }
}

/// Resolved value-type snapshot of core box-layout properties (derived from `tables::Layout`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutSnapshot {
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub min_height: Dimension,
    pub max_width: Dimension,
    pub max_height: Dimension,
    pub flex_direction: FlexDirection,
    pub justify_content: FlexJustify,
    pub align_items: FlexAlign,
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub gap: f32,
    pub padding: EdgeInsets,
    pub margin: EdgeInsets,
}

impl Default for LayoutSnapshot {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: Dimension::Auto,
            min_height: Dimension::Auto,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            flex_direction: FlexDirection::Column,
            justify_content: FlexJustify::FlexStart,
            align_items: FlexAlign::Stretch,
            align_self: AlignSelf::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            gap: 0.0,
            padding: EdgeInsets::default(),
            margin: EdgeInsets::default(),
        }
    }
}

use crate::interaction::CursorKind;
use crate::tables::AlignSelf;
use crate::layout_ext::{
    GridTrack, GridTrackPlacement, LayoutDirection, LayoutExt, PositionMode,
};
use crate::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use crate::transform::Transform2D;

/// Strictly typed `display` mode shared by [`LayoutExt`] and [`StylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Display {
    #[default]
    Flex = 0,
    None = 1,
    Grid = 2,
}

impl Display {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Flex => "flex",
            Self::None => "none",
            Self::Grid => "grid",
        }
    }
}

/// Strictly typed `flex-wrap` override for [`StylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexWrap {
    NoWrap = 0,
    Wrap = 1,
    WrapReverse = 2,
}

impl FlexWrap {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::NoWrap => "nowrap",
            Self::Wrap => "wrap",
            Self::WrapReverse => "wrap-reverse",
        }
    }
}

/// Strictly typed `overflow` override for [`StylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Overflow {
    Visible = 0,
    Hidden = 1,
    Scroll = 2,
    Auto = 3,
}

impl Overflow {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Scroll => "scroll",
            Self::Auto => "auto",
        }
    }
}

/// Strictly typed `text-align` override for [`StylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left = 0,
    Center = 1,
    Right = 2,
    Justify = 3,
}

impl TextAlign {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "justify",
        }
    }
}

/// Sparse, strictly typed layout, visual style, and cursor overrides shared by both responsive
/// `@media` rules and interactive pseudo-state rules (`:hover`, `:focus`, `:focus-visible`,
/// `:active`, `:disabled`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StylePatch {
    pub flex_direction: Option<FlexDirection>,
    pub justify_content: Option<FlexJustify>,
    pub align_items: Option<FlexAlign>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub flex_basis: Option<Dimension>,
    pub aspect_ratio: Option<f32>,
    pub position: Option<PositionMode>,
    pub inset_top: Option<Dimension>,
    pub inset_right: Option<Dimension>,
    pub inset_bottom: Option<Dimension>,
    pub inset_left: Option<Dimension>,
    pub gap: Option<f32>,
    pub column_gap: Option<f32>,
    pub row_gap: Option<f32>,
    pub grid_template_columns: Option<Box<[GridTrack]>>,
    pub grid_template_rows: Option<Box<[GridTrack]>>,
    pub grid_column: Option<GridTrackPlacement>,
    pub grid_row: Option<GridTrackPlacement>,
    pub direction: Option<LayoutDirection>,
    pub padding_inline_start: Option<f32>,
    pub padding_inline_end: Option<f32>,
    pub margin_inline_start: Option<f32>,
    pub margin_inline_end: Option<f32>,
    pub transform: Option<Transform2D>,
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    pub min_width: Option<Dimension>,
    pub max_width: Option<Dimension>,
    pub min_height: Option<Dimension>,
    pub max_height: Option<Dimension>,
    pub padding: Option<EdgeInsets>,
    pub margin: Option<EdgeInsets>,
    pub background: Option<ColorRgba>,
    pub background_fill: Option<BackgroundFill>,
    pub text_color: Option<ColorRgba>,
    pub border_width: Option<f32>,
    pub border_color: Option<ColorRgba>,
    pub border_radius: Option<f32>,
    pub opacity: Option<f32>,
    pub font_size: Option<f32>,
    pub z_index: Option<i32>,
    pub flex_wrap: Option<FlexWrap>,
    pub overflow: Option<Overflow>,
    pub text_align: Option<TextAlign>,
    pub font_weight: Option<u16>,
    pub display: Option<Display>,
    pub cursor: Option<CursorKind>,
}

impl StylePatch {
    #[inline]
    pub const fn new() -> Self {
        Self {
            flex_direction: None,
            justify_content: None,
            align_items: None,
            flex_grow: None,
            flex_shrink: None,
            flex_basis: None,
            aspect_ratio: None,
            position: None,
            inset_top: None,
            inset_right: None,
            inset_bottom: None,
            inset_left: None,
            gap: None,
            column_gap: None,
            row_gap: None,
            grid_template_columns: None,
            grid_template_rows: None,
            grid_column: None,
            grid_row: None,
            direction: None,
            padding_inline_start: None,
            padding_inline_end: None,
            margin_inline_start: None,
            margin_inline_end: None,
            transform: None,
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            padding: None,
            margin: None,
            background: None,
            background_fill: None,
            text_color: None,
            border_width: None,
            border_color: None,
            border_radius: None,
            opacity: None,
            font_size: None,
            z_index: None,
            flex_wrap: None,
            overflow: None,
            text_align: None,
            font_weight: None,
            display: None,
            cursor: None,
        }
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.flex_direction.is_none()
            && self.justify_content.is_none()
            && self.align_items.is_none()
            && self.flex_grow.is_none()
            && self.flex_shrink.is_none()
            && self.flex_basis.is_none()
            && self.aspect_ratio.is_none()
            && self.position.is_none()
            && self.inset_top.is_none()
            && self.inset_right.is_none()
            && self.inset_bottom.is_none()
            && self.inset_left.is_none()
            && self.gap.is_none()
            && self.column_gap.is_none()
            && self.row_gap.is_none()
            && self.grid_template_columns.is_none()
            && self.grid_template_rows.is_none()
            && self.grid_column.is_none()
            && self.grid_row.is_none()
            && self.direction.is_none()
            && self.padding_inline_start.is_none()
            && self.padding_inline_end.is_none()
            && self.margin_inline_start.is_none()
            && self.margin_inline_end.is_none()
            && self.transform.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.min_width.is_none()
            && self.max_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
            && self.padding.is_none()
            && self.margin.is_none()
            && self.background.is_none()
            && self.background_fill.is_none()
            && self.text_color.is_none()
            && self.border_width.is_none()
            && self.border_color.is_none()
            && self.border_radius.is_none()
            && self.opacity.is_none()
            && self.font_size.is_none()
            && self.z_index.is_none()
            && self.flex_wrap.is_none()
            && self.overflow.is_none()
            && self.text_align.is_none()
            && self.font_weight.is_none()
            && self.display.is_none()
            && self.cursor.is_none()
    }

    #[inline]
    fn sanitize(v: f32) -> f32 {
        if v.is_finite() { v } else { 0.0 }
    }

    #[inline] pub fn flex_direction(mut self, dir: FlexDirection) -> Self { self.flex_direction = Some(dir); self }
    #[inline] pub fn flex_row(self) -> Self { self.flex_direction(FlexDirection::Row) }
    #[inline] pub fn flex_col(self) -> Self { self.flex_direction(FlexDirection::Column) }
    #[inline] pub fn justify_content(mut self, j: FlexJustify) -> Self { self.justify_content = Some(j); self }
    #[inline] pub fn align_items(mut self, a: FlexAlign) -> Self { self.align_items = Some(a); self }
    #[inline] pub fn flex_grow(mut self, v: f32) -> Self { self.flex_grow = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn flex_shrink(mut self, v: f32) -> Self { self.flex_shrink = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn gap(mut self, v: f32) -> Self { self.gap = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn width(mut self, d: Dimension) -> Self { self.width = Some(d); self }
    #[inline] pub fn width_px(self, v: f32) -> Self { self.width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn width_pct(self, v: f32) -> Self { self.width(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0))) }
    #[inline] pub fn width_auto(self) -> Self { self.width(Dimension::Auto) }
    #[inline] pub fn height(mut self, d: Dimension) -> Self { self.height = Some(d); self }
    #[inline] pub fn height_px(self, v: f32) -> Self { self.height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn height_pct(self, v: f32) -> Self { self.height(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0))) }
    #[inline] pub fn height_auto(self) -> Self { self.height(Dimension::Auto) }
    #[inline] pub fn min_width(mut self, d: Dimension) -> Self { self.min_width = Some(d); self }
    #[inline] pub fn min_width_px(self, v: f32) -> Self { self.min_width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn max_width(mut self, d: Dimension) -> Self { self.max_width = Some(d); self }
    #[inline] pub fn max_width_px(self, v: f32) -> Self { self.max_width(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn min_height(mut self, d: Dimension) -> Self { self.min_height = Some(d); self }
    #[inline] pub fn min_height_px(self, v: f32) -> Self { self.min_height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn max_height(mut self, d: Dimension) -> Self { self.max_height = Some(d); self }
    #[inline] pub fn max_height_px(self, v: f32) -> Self { self.max_height(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn padding(mut self, v: f32) -> Self { self.padding = Some(EdgeInsets::all(Self::sanitize(v).max(0.0))); self }
    #[inline] pub fn padding_xy(mut self, h: f32, v: f32) -> Self { self.padding = Some(EdgeInsets::symmetric(Self::sanitize(v).max(0.0), Self::sanitize(h).max(0.0))); self }
    #[inline] pub fn padding_insets(mut self, insets: EdgeInsets) -> Self { self.padding = Some(insets); self }
    #[inline] pub fn margin(mut self, v: f32) -> Self { self.margin = Some(EdgeInsets::all(Self::sanitize(v))); self }
    #[inline] pub fn margin_xy(mut self, h: f32, v: f32) -> Self { self.margin = Some(EdgeInsets::symmetric(Self::sanitize(v), Self::sanitize(h))); self }
    #[inline] pub fn margin_insets(mut self, insets: EdgeInsets) -> Self { self.margin = Some(insets); self }
    #[inline] pub fn bg(mut self, c: ColorRgba) -> Self { self.background = Some(c); self }
    #[inline] pub fn background(self, c: ColorRgba) -> Self { self.bg(c) }
    #[inline] pub fn background_fill(mut self, fill: BackgroundFill) -> Self { self.background_fill = Some(fill); self }
    #[inline] pub fn linear_gradient(mut self, angle_deg: f32, stops: impl Into<Vec<GradientStop>>) -> Self { self.background_fill = Some(BackgroundFill::Linear(LinearGradient::new(angle_deg, stops))); self }
    #[inline] pub fn radial_gradient(mut self, center_x: f32, center_y: f32, radius: f32, stops: impl Into<Vec<GradientStop>>) -> Self { self.background_fill = Some(BackgroundFill::Radial(RadialGradient::new(center_x, center_y, radius, stops))); self }
    #[inline] pub fn color(mut self, c: ColorRgba) -> Self { self.text_color = Some(c); self }
    #[inline] pub fn text_color(self, c: ColorRgba) -> Self { self.color(c) }
    #[inline] pub fn border(mut self, w: f32, c: ColorRgba) -> Self { self.border_width = Some(Self::sanitize(w).max(0.0)); self.border_color = Some(c); self }
    #[inline] pub fn border_width(mut self, w: f32) -> Self { self.border_width = Some(Self::sanitize(w).max(0.0)); self }
    #[inline] pub fn border_color(mut self, c: ColorRgba) -> Self { self.border_color = Some(c); self }
    #[inline] pub fn radius(mut self, r: f32) -> Self { self.border_radius = Some(Self::sanitize(r).max(0.0)); self }
    #[inline] pub fn border_radius(self, r: f32) -> Self { self.radius(r) }
    #[inline] pub fn opacity(mut self, o: f32) -> Self { self.opacity = Some(Self::sanitize(o).clamp(0.0, 1.0)); self }
    #[inline] pub fn font_size(mut self, px: f32) -> Self { self.font_size = Some(Self::sanitize(px).max(1.0)); self }
    #[inline] pub fn z_index(mut self, z: i32) -> Self { self.z_index = Some(z); self }
    #[inline] pub fn flex_wrap(mut self, wrap: FlexWrap) -> Self { self.flex_wrap = Some(wrap); self }
    #[inline] pub fn overflow(mut self, ov: Overflow) -> Self { self.overflow = Some(ov); self }
    #[inline] pub fn text_align(mut self, align: TextAlign) -> Self { self.text_align = Some(align); self }
    #[inline] pub fn font_weight(mut self, weight: u16) -> Self { self.font_weight = Some(weight.clamp(1, 1000)); self }
    #[inline] pub fn display(mut self, display: Display) -> Self { self.display = Some(display); self }
    #[inline] pub fn display_grid(self) -> Self { self.display(Display::Grid) }
    #[inline] pub fn cursor(mut self, cursor: CursorKind) -> Self { self.cursor = Some(cursor); self }
    #[inline] pub fn flex_basis(mut self, d: Dimension) -> Self { self.flex_basis = Some(d); self }
    #[inline] pub fn flex_basis_px(self, v: f32) -> Self { self.flex_basis(Dimension::Points(Self::sanitize(v).max(0.0))) }
    #[inline] pub fn flex_basis_pct(self, v: f32) -> Self { self.flex_basis(Dimension::Percent(Self::sanitize(v).clamp(0.0, 100.0))) }
    #[inline] pub fn aspect_ratio(mut self, ratio: f32) -> Self { if ratio.is_finite() && ratio > 0.0 { self.aspect_ratio = Some(ratio); } self }
    #[inline] pub fn position(mut self, mode: PositionMode) -> Self { self.position = Some(mode); self }
    #[inline] pub fn inset_top(mut self, d: Dimension) -> Self { self.inset_top = Some(d); self }
    #[inline] pub fn inset_right(mut self, d: Dimension) -> Self { self.inset_right = Some(d); self }
    #[inline] pub fn inset_bottom(mut self, d: Dimension) -> Self { self.inset_bottom = Some(d); self }
    #[inline] pub fn inset_left(mut self, d: Dimension) -> Self { self.inset_left = Some(d); self }
    #[inline] pub fn column_gap(mut self, v: f32) -> Self { self.column_gap = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn row_gap(mut self, v: f32) -> Self { self.row_gap = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn grid_cols(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self { self.grid_template_columns = Some(tracks.into().into_boxed_slice()); self }
    #[inline] pub fn grid_rows(mut self, tracks: impl Into<Vec<GridTrack>>) -> Self { self.grid_template_rows = Some(tracks.into().into_boxed_slice()); self }
    #[inline] pub fn grid_column(mut self, p: GridTrackPlacement) -> Self { self.grid_column = Some(p); self }
    #[inline] pub fn grid_row(mut self, p: GridTrackPlacement) -> Self { self.grid_row = Some(p); self }
    #[inline] pub fn grid_column_span(self, span: u16) -> Self { self.grid_column(GridTrackPlacement::span(span)) }
    #[inline] pub fn grid_row_span(self, span: u16) -> Self { self.grid_row(GridTrackPlacement::span(span)) }
    #[inline] pub fn direction(mut self, dir: LayoutDirection) -> Self { self.direction = Some(dir); self }
    #[inline] pub fn padding_inline_start(mut self, v: f32) -> Self { self.padding_inline_start = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn padding_inline_end(mut self, v: f32) -> Self { self.padding_inline_end = Some(Self::sanitize(v).max(0.0)); self }
    #[inline] pub fn margin_inline_start(mut self, v: f32) -> Self { self.margin_inline_start = Some(Self::sanitize(v)); self }
    #[inline] pub fn margin_inline_end(mut self, v: f32) -> Self { self.margin_inline_end = Some(Self::sanitize(v)); self }
    #[inline] pub fn transform(mut self, tf: Transform2D) -> Self { self.transform = Some(tf); self }
    #[inline] pub fn translate(mut self, x: f32, y: f32) -> Self { let tf = self.transform.unwrap_or(Transform2D::IDENTITY).with_translate(x, y); self.transform = Some(tf); self }
    #[inline] pub fn scale(mut self, s: f32) -> Self { let tf = self.transform.unwrap_or(Transform2D::IDENTITY).with_scale(s); self.transform = Some(tf); self }
    #[inline] pub fn rotate(mut self, deg: f32) -> Self { let tf = self.transform.unwrap_or(Transform2D::IDENTITY).with_rotate(deg); self.transform = Some(tf); self }

    /// Merges all populated fields from `other` on top of `self` (last-writer-wins cascade).
    pub fn merge(&mut self, other: &Self) {
        if other.flex_direction.is_some() { self.flex_direction = other.flex_direction; }
        if other.justify_content.is_some() { self.justify_content = other.justify_content; }
        if other.align_items.is_some() { self.align_items = other.align_items; }
        if other.flex_grow.is_some() { self.flex_grow = other.flex_grow; }
        if other.flex_shrink.is_some() { self.flex_shrink = other.flex_shrink; }
        if other.flex_basis.is_some() { self.flex_basis = other.flex_basis; }
        if other.aspect_ratio.is_some() { self.aspect_ratio = other.aspect_ratio; }
        if other.position.is_some() { self.position = other.position; }
        if other.inset_top.is_some() { self.inset_top = other.inset_top; }
        if other.inset_right.is_some() { self.inset_right = other.inset_right; }
        if other.inset_bottom.is_some() { self.inset_bottom = other.inset_bottom; }
        if other.inset_left.is_some() { self.inset_left = other.inset_left; }
        if other.gap.is_some() { self.gap = other.gap; }
        if other.column_gap.is_some() { self.column_gap = other.column_gap; }
        if other.row_gap.is_some() { self.row_gap = other.row_gap; }
        if let Some(ref cols) = other.grid_template_columns { self.grid_template_columns = Some(cols.clone()); }
        if let Some(ref rows) = other.grid_template_rows { self.grid_template_rows = Some(rows.clone()); }
        if other.grid_column.is_some() { self.grid_column = other.grid_column; }
        if other.grid_row.is_some() { self.grid_row = other.grid_row; }
        if other.direction.is_some() { self.direction = other.direction; }
        if other.padding_inline_start.is_some() { self.padding_inline_start = other.padding_inline_start; }
        if other.padding_inline_end.is_some() { self.padding_inline_end = other.padding_inline_end; }
        if other.margin_inline_start.is_some() { self.margin_inline_start = other.margin_inline_start; }
        if other.margin_inline_end.is_some() { self.margin_inline_end = other.margin_inline_end; }
        if other.transform.is_some() { self.transform = other.transform; }
        if other.width.is_some() { self.width = other.width; }
        if other.height.is_some() { self.height = other.height; }
        if other.min_width.is_some() { self.min_width = other.min_width; }
        if other.max_width.is_some() { self.max_width = other.max_width; }
        if other.min_height.is_some() { self.min_height = other.min_height; }
        if other.max_height.is_some() { self.max_height = other.max_height; }
        if other.padding.is_some() { self.padding = other.padding; }
        if other.margin.is_some() { self.margin = other.margin; }
        if other.background.is_some() { self.background = other.background; }
        if let Some(ref fill) = other.background_fill { self.background_fill = Some(fill.clone()); }
        if other.text_color.is_some() { self.text_color = other.text_color; }
        if other.border_width.is_some() { self.border_width = other.border_width; }
        if other.border_color.is_some() { self.border_color = other.border_color; }
        if other.border_radius.is_some() { self.border_radius = other.border_radius; }
        if other.opacity.is_some() { self.opacity = other.opacity; }
        if other.font_size.is_some() { self.font_size = other.font_size; }
        if other.z_index.is_some() { self.z_index = other.z_index; }
        if other.flex_wrap.is_some() { self.flex_wrap = other.flex_wrap; }
        if other.overflow.is_some() { self.overflow = other.overflow; }
        if other.text_align.is_some() { self.text_align = other.text_align; }
        if other.font_weight.is_some() { self.font_weight = other.font_weight; }
        if other.display.is_some() { self.display = other.display; }
        if other.cursor.is_some() { self.cursor = other.cursor; }
    }

    /// Applies extended layout overrides (grid, flex-basis, aspect-ratio, position, RTL) onto `ext`.
    pub fn apply_layout_ext_to(&self, ext: &mut LayoutExt) {
        if let Some(d) = self.display { ext.display = d; }
        if let Some(fb) = self.flex_basis { ext.flex_basis = fb; }
        if let Some(ar) = self.aspect_ratio { ext.aspect_ratio = Some(ar); }
        if let Some(pos) = self.position { ext.position = pos; }
        if let Some(v) = self.inset_top { ext.insets.top = v; }
        if let Some(v) = self.inset_right { ext.insets.right = v; }
        if let Some(v) = self.inset_bottom { ext.insets.bottom = v; }
        if let Some(v) = self.inset_left { ext.insets.left = v; }
        if let Some(cg) = self.column_gap { ext.grid_mut().column_gap = Some(cg); }
        if let Some(rg) = self.row_gap { ext.grid_mut().row_gap = Some(rg); }
        if let Some(ref cols) = self.grid_template_columns { ext.grid_mut().template_columns = cols.to_vec(); }
        if let Some(ref rows) = self.grid_template_rows { ext.grid_mut().template_rows = rows.to_vec(); }
        if let Some(gc) = self.grid_column { ext.grid_item.column = gc; }
        if let Some(gr) = self.grid_row { ext.grid_item.row = gr; }
        if let Some(dir) = self.direction { ext.direction = Some(dir); }
        if let Some(v) = self.padding_inline_start { ext.padding_inline.start = Some(v); }
        if let Some(v) = self.padding_inline_end { ext.padding_inline.end = Some(v); }
        if let Some(v) = self.margin_inline_start { ext.margin_inline.start = Some(v); }
        if let Some(v) = self.margin_inline_end { ext.margin_inline.end = Some(v); }
    }

    /// Applies all present visual style overrides onto `style` (paint-only fast path).
    pub fn apply_style_to(&self, style: &mut VisualSnapshot) {
        if let Some(v) = self.background { style.background = v; }
        if let Some(v) = self.text_color { style.text_color = v; }
        if let Some(v) = self.border_width { style.border_width = v; }
        if let Some(v) = self.border_color { style.border_color = v; }
        if let Some(v) = self.border_radius { style.border_radius = v; }
        if let Some(v) = self.opacity { style.opacity = v; }
        if let Some(v) = self.font_size { style.font_size = v; }
        if let Some(v) = self.z_index { style.z_index = v; }
    }

    /// Applies all present layout overrides onto `layout`.
    pub fn apply_layout_to(&self, layout: &mut LayoutSnapshot) {
        if let Some(v) = self.flex_direction { layout.flex_direction = v; }
        if let Some(v) = self.justify_content { layout.justify_content = v; }
        if let Some(v) = self.align_items { layout.align_items = v; }
        if let Some(v) = self.flex_grow { layout.flex_grow = v; }
        if let Some(v) = self.flex_shrink { layout.flex_shrink = v; }
        if let Some(v) = self.gap { layout.gap = v; }
        if let Some(v) = self.width { layout.width = v; }
        if let Some(v) = self.height { layout.height = v; }
        if let Some(v) = self.min_width { layout.min_width = v; }
        if let Some(v) = self.max_width { layout.max_width = v; }
        if let Some(v) = self.min_height { layout.min_height = v; }
        if let Some(v) = self.max_height { layout.max_height = v; }
        if let Some(v) = self.padding { layout.padding = v; }
        if let Some(v) = self.margin { layout.margin = v; }
    }

    /// Applies all present layout and visual overrides onto `layout` and `style`.
    pub fn apply_to(&self, layout: &mut LayoutSnapshot, style: &mut VisualSnapshot) {
        self.apply_layout_to(layout);
        self.apply_style_to(style);
    }
}
