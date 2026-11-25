//! Unified sparse, strictly typed style & layout patch (`StylePatch`) shared across
//! responsive `@media` rules (`ResponsiveRule`) and interactive pseudo-state rules
//! (`StateStyleRule`) with zero duplication and zero string serialization.

use std::fmt::Write as _;

use nve_abi::{
    ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, LayoutProps,
    StyleProps,
};

use crate::interaction::CursorKind;

/// Strictly typed `display` override for [`StylePatch`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Display {
    Flex = 0,
    None = 1,
}

impl Display {
    #[inline]
    pub const fn as_css(self) -> &'static str {
        match self {
            Self::Flex => "flex",
            Self::None => "none",
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
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StylePatch {
    pub flex_direction: Option<FlexDirection>,
    pub justify_content: Option<FlexJustify>,
    pub align_items: Option<FlexAlign>,
    pub flex_grow: Option<f32>,
    pub flex_shrink: Option<f32>,
    pub gap: Option<f32>,
    pub width: Option<Dimension>,
    pub height: Option<Dimension>,
    pub min_width: Option<Dimension>,
    pub max_width: Option<Dimension>,
    pub min_height: Option<Dimension>,
    pub max_height: Option<Dimension>,
    pub padding: Option<EdgeInsets>,
    pub margin: Option<EdgeInsets>,
    pub background: Option<ColorRgba>,
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
            gap: None,
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            padding: None,
            margin: None,
            background: None,
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
            && self.gap.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.min_width.is_none()
            && self.max_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
            && self.padding.is_none()
            && self.margin.is_none()
            && self.background.is_none()
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
    #[inline] pub fn cursor(mut self, cursor: CursorKind) -> Self { self.cursor = Some(cursor); self }

    /// Merges all populated fields from `other` on top of `self` (last-writer-wins cascade).
    pub fn merge(&mut self, other: &Self) {
        if other.flex_direction.is_some() { self.flex_direction = other.flex_direction; }
        if other.justify_content.is_some() { self.justify_content = other.justify_content; }
        if other.align_items.is_some() { self.align_items = other.align_items; }
        if other.flex_grow.is_some() { self.flex_grow = other.flex_grow; }
        if other.flex_shrink.is_some() { self.flex_shrink = other.flex_shrink; }
        if other.gap.is_some() { self.gap = other.gap; }
        if other.width.is_some() { self.width = other.width; }
        if other.height.is_some() { self.height = other.height; }
        if other.min_width.is_some() { self.min_width = other.min_width; }
        if other.max_width.is_some() { self.max_width = other.max_width; }
        if other.min_height.is_some() { self.min_height = other.min_height; }
        if other.max_height.is_some() { self.max_height = other.max_height; }
        if other.padding.is_some() { self.padding = other.padding; }
        if other.margin.is_some() { self.margin = other.margin; }
        if other.background.is_some() { self.background = other.background; }
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

    /// Applies all present visual style overrides onto `style` (paint-only fast path).
    pub fn apply_style_to(&self, style: &mut StyleProps) {
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
    pub fn apply_layout_to(&self, layout: &mut LayoutProps) {
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

    /// Applies all present layout and visual overrides onto `layout` and `style` (native path).
    pub fn apply_to(&self, layout: &mut LayoutProps, style: &mut StyleProps) {
        self.apply_layout_to(layout);
        self.apply_style_to(style);
    }

    /// Applies layout/style overrides and invokes `on_raw` for non-ABI CSS properties
    /// (`flex-wrap`, `overflow`, `text-align`, `font-weight`, `display`, `cursor`).
    pub fn apply_to_with_raw<F: FnMut(&str, &str)>(
        &self,
        layout: &mut LayoutProps,
        style: &mut StyleProps,
        mut on_raw: F,
    ) {
        self.apply_to(layout, style);
        if let Some(w) = self.flex_wrap { on_raw("flex-wrap", w.as_css()); }
        if let Some(o) = self.overflow { on_raw("overflow", o.as_css()); }
        if let Some(a) = self.text_align { on_raw("text-align", a.as_css()); }
        if let Some(fw) = self.font_weight {
            let mut buf = String::with_capacity(4);
            let _ = write!(&mut buf, "{fw}");
            on_raw("font-weight", &buf);
        }
        if let Some(d) = self.display { on_raw("display", d.as_css()); }
        if let Some(c) = self.cursor { on_raw("cursor", c.to_css_keyword()); }
    }
}
