//! Zero-string-format [`TypedStyleBuilder`], [`TypedSurfaceStyle`], [`TypedTextStyle`], and
//! fast zero-allocation [`parse_color_rgba`] helper.

use std::borrow::Cow;

use dioxus_core::{AttributeValue, IntoAttributeValue};
use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_dioxus::{parse_flex_align, parse_flex_direction, parse_flex_justify, TypedStyle};

/// Zero-allocation parser for `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, and CSS named colors.
#[inline]
pub fn parse_color_rgba(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let b = hex.as_bytes();
        return match b.len() {
            3 => Some(ColorRgba::rgb(
                hex_nibble(b[0])? * 17,
                hex_nibble(b[1])? * 17,
                hex_nibble(b[2])? * 17,
            )),
            4 => Some(ColorRgba::rgba(
                hex_nibble(b[0])? * 17,
                hex_nibble(b[1])? * 17,
                hex_nibble(b[2])? * 17,
                hex_nibble(b[3])? * 17,
            )),
            6 => Some(ColorRgba::rgb(
                hex_byte(b[0], b[1])?,
                hex_byte(b[2], b[3])?,
                hex_byte(b[4], b[5])?,
            )),
            8 => Some(ColorRgba::rgba(
                hex_byte(b[0], b[1])?,
                hex_byte(b[2], b[3])?,
                hex_byte(b[4], b[5])?,
                hex_byte(b[6], b[7])?,
            )),
            _ => None,
        };
    }
    if s.eq_ignore_ascii_case("transparent") {
        Some(ColorRgba::TRANSPARENT)
    } else if s.eq_ignore_ascii_case("white") {
        Some(ColorRgba::WHITE)
    } else if s.eq_ignore_ascii_case("black") {
        Some(ColorRgba::BLACK)
    } else if s.eq_ignore_ascii_case("red") {
        Some(ColorRgba::rgb(239, 68, 68))
    } else if s.eq_ignore_ascii_case("green") {
        Some(ColorRgba::rgb(34, 197, 94))
    } else if s.eq_ignore_ascii_case("blue") {
        Some(ColorRgba::rgb(59, 130, 246))
    } else {
        None
    }
}

#[inline]
const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[inline]
const fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    match (hex_nibble(hi), hex_nibble(lo)) {
        (Some(h), Some(l)) => Some((h << 4) | l),
        _ => None,
    }
}

/// Typed full visual surface specification emitted atomically by [`TypedStyleBuilder::surface`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedSurfaceStyle {
    pub background: ColorRgba,
    pub border_width: f32,
    pub border_color: ColorRgba,
    pub border_radius: f32,
    pub opacity: f32,
    pub box_shadow: &'static str,
}

impl Default for TypedSurfaceStyle {
    fn default() -> Self {
        Self {
            background: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        }
    }
}

/// Typed full text visual specification emitted atomically by [`TypedStyleBuilder::text_style`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedTextStyle {
    pub color: ColorRgba,
    pub font_size: f32,
    pub font_weight: &'static str,
    pub text_align: &'static str,
    pub opacity: f32,
}

impl Default for TypedTextStyle {
    fn default() -> Self {
        Self {
            color: ColorRgba::rgb(0x0f, 0x17, 0x2a),
            font_size: 15.0,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        }
    }
}

/// Zero-string-format builder producing [`nve_dioxus::TypedStyle`] directly for `nve_style`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypedStyleBuilder {
    inner: TypedStyle,
}

impl TypedStyleBuilder {
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: TypedStyle::new(),
        }
    }

    #[inline]
    pub fn width_px(mut self, v: f32) -> Self {
        self.inner = self.inner.width_px(v);
        self
    }

    #[inline]
    pub fn width_pct(mut self, v: f32) -> Self {
        self.inner = self.inner.width_pct(v);
        self
    }

    #[inline]
    pub fn width_auto(mut self) -> Self {
        self.inner = self.inner.width_auto();
        self
    }

    #[inline]
    pub fn height_px(mut self, v: f32) -> Self {
        self.inner = self.inner.height_px(v);
        self
    }

    #[inline]
    pub fn height_pct(mut self, v: f32) -> Self {
        self.inner = self.inner.height_pct(v);
        self
    }

    #[inline]
    pub fn height_auto(mut self) -> Self {
        self.inner = self.inner.height_auto();
        self
    }

    #[inline]
    pub fn min_width_px(mut self, v: f32) -> Self {
        self.inner = self.inner.min_width_px(v);
        self
    }

    #[inline]
    pub fn min_height_px(mut self, v: f32) -> Self {
        self.inner = self.inner.min_height_px(v);
        self
    }

    #[inline]
    pub fn max_width_px(mut self, v: f32) -> Self {
        self.inner = self.inner.max_width_px(v);
        self
    }

    #[inline]
    pub fn max_height_px(mut self, v: f32) -> Self {
        self.inner = self.inner.max_height_px(v);
        self
    }

    #[inline]
    pub fn flex_direction(mut self, dir: FlexDirection) -> Self {
        self.inner = self.inner.flex_direction(dir);
        self
    }

    #[inline]
    pub fn flex_row(mut self) -> Self {
        self.inner = self.inner.flex_row();
        self
    }

    #[inline]
    pub fn flex_col(mut self) -> Self {
        self.inner = self.inner.flex_col();
        self
    }

    #[inline]
    pub fn justify_content(mut self, justify: FlexJustify) -> Self {
        self.inner = self.inner.justify_content(justify);
        self
    }

    #[inline]
    pub fn justify_content_str(mut self, justify: &str) -> Self {
        self.inner = self.inner.justify_content_str(justify);
        self
    }

    #[inline]
    pub fn align_items(mut self, align: FlexAlign) -> Self {
        self.inner = self.inner.align_items(align);
        self
    }

    #[inline]
    pub fn align_items_str(mut self, align: &str) -> Self {
        self.inner = self.inner.align_items_str(align);
        self
    }

    #[inline]
    pub fn flex_grow(mut self, v: f32) -> Self {
        self.inner = self.inner.flex_grow(v);
        self
    }

    #[inline]
    pub fn flex_shrink(mut self, v: f32) -> Self {
        self.inner = self.inner.flex_shrink(v);
        self
    }

    #[inline]
    pub fn gap(mut self, v: f32) -> Self {
        self.inner = self.inner.gap(v);
        self
    }

    #[inline]
    pub fn padding(mut self, v: f32) -> Self {
        self.inner = self.inner.padding(v);
        self
    }

    #[inline]
    pub fn padding_xy(mut self, horizontal: f32, vertical: f32) -> Self {
        self.inner = self.inner.padding_xy(horizontal, vertical);
        self
    }

    #[inline]
    pub fn margin(mut self, v: f32) -> Self {
        self.inner = self.inner.margin(v);
        self
    }

    #[inline]
    pub fn z_index(mut self, z: i32) -> Self {
        self.inner = self.inner.z_index(z);
        self
    }

    #[inline]
    pub fn overflow(mut self, mode: impl Into<Cow<'static, str>>) -> Self {
        self.inner = self.inner.overflow(mode);
        self
    }

    #[inline]
    pub fn object_fit(mut self, mode: impl Into<Cow<'static, str>>) -> Self {
        self.inner = self.inner.object_fit(mode);
        self
    }

    /// Emits EVERY surface visual property explicitly with zero string formatting.
    #[inline]
    pub fn surface(mut self, s: TypedSurfaceStyle) -> Self {
        self.inner = self
            .inner
            .bg(s.background)
            .border(s.border_width, s.border_color)
            .radius(s.border_radius)
            .opacity(s.opacity)
            .box_shadow(s.box_shadow);
        self
    }

    /// Emits EVERY text visual property explicitly with zero string formatting.
    #[inline]
    pub fn text_style(mut self, t: TypedTextStyle) -> Self {
        self.inner = self
            .inner
            .color(t.color)
            .font_size(t.font_size)
            .font_weight(t.font_weight)
            .text_align(t.text_align)
            .opacity(t.opacity);
        self
    }

    #[inline]
    pub fn finish(self) -> TypedStyle {
        self.inner
    }
}

impl From<TypedStyleBuilder> for TypedStyle {
    #[inline]
    fn from(builder: TypedStyleBuilder) -> Self {
        builder.inner
    }
}

impl IntoAttributeValue for TypedStyleBuilder {
    #[inline]
    fn into_value(self) -> AttributeValue {
        self.inner.into_value()
    }
}

fn parse_px(raw: &str) -> Option<f32> {
    let s = raw
        .trim()
        .strip_suffix("px")
        .unwrap_or(raw.trim());
    nve_abi::num::parse_f32(s)
}

fn parse_dim(raw: &str) -> Dimension {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("auto") {
        Dimension::Auto
    } else if let Some(pct) = s.strip_suffix('%') {
        nve_abi::num::parse_f32(pct.trim())
            .map(Dimension::Percent)
            .unwrap_or(Dimension::Auto)
    } else {
        parse_px(s).map(Dimension::Points).unwrap_or(Dimension::Auto)
    }
}

pub(super) fn parse_inline_css_to_typed(css: &str) -> TypedStyle {
    let mut ts = TypedStyle::new();
    for decl in css.split(';') {
        let Some((raw_k, raw_v)) = decl.split_once(':') else {
            continue;
        };
        let k = raw_k.trim();
        let v = raw_v.trim();
        if k.is_empty() || v.is_empty() {
            continue;
        }
        match k {
            "width" => ts = ts.width(parse_dim(v)),
            "height" => ts = ts.height(parse_dim(v)),
            "min-width" | "minWidth" => ts = ts.min_width(parse_dim(v)),
            "min-height" | "minHeight" => ts = ts.min_height(parse_dim(v)),
            "max-width" | "maxWidth" => ts = ts.max_width(parse_dim(v)),
            "max-height" | "maxHeight" => ts = ts.max_height(parse_dim(v)),
            "flex-direction" | "flexDirection" => ts = ts.flex_direction(parse_flex_direction(v)),
            "justify-content" | "justifyContent" => ts = ts.justify_content(parse_flex_justify(v)),
            "align-items" | "alignItems" => ts = ts.align_items(parse_flex_align(v)),
            "flex" | "flex-grow" | "flexGrow" => {
                if let Some(f) = nve_abi::num::parse_f32(v) {
                    ts = ts.flex_grow(f);
                }
            }
            "flex-shrink" | "flexShrink" => {
                if let Some(f) = nve_abi::num::parse_f32(v) {
                    ts = ts.flex_shrink(f);
                }
            }
            "gap" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.gap(f);
                }
            }
            "padding" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.padding(f);
                }
            }
            "padding-horizontal" | "paddingHorizontal" => {
                if let Some(f) = parse_px(v) {
                    let mut insets = ts.layout.padding;
                    insets.left = f;
                    insets.right = f;
                    ts = ts.padding_insets(insets);
                }
            }
            "padding-vertical" | "paddingVertical" => {
                if let Some(f) = parse_px(v) {
                    let mut insets = ts.layout.padding;
                    insets.top = f;
                    insets.bottom = f;
                    ts = ts.padding_insets(insets);
                }
            }
            "margin" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.margin_insets(EdgeInsets::all(f));
                }
            }
            "background" | "background-color" | "backgroundColor" => {
                if let Some(c) = parse_color_rgba(v) {
                    ts = ts.bg(c);
                }
            }
            "color" => {
                if let Some(c) = parse_color_rgba(v) {
                    ts = ts.color(c);
                }
            }
            "border-color" | "borderColor" => {
                if let Some(c) = parse_color_rgba(v) {
                    ts = ts.border_color(c);
                }
            }
            "border-width" | "borderWidth" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.border_width(f);
                }
            }
            "border-radius" | "borderRadius" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.radius(f);
                }
            }
            "opacity" => {
                if let Some(f) = nve_abi::num::parse_f32(v) {
                    ts = ts.opacity(f);
                }
            }
            "font-size" | "fontSize" => {
                if let Some(f) = parse_px(v) {
                    ts = ts.font_size(f);
                }
            }
            "z-index" | "zIndex" => {
                if let Ok(z) = v.parse::<i32>() {
                    ts = ts.z_index(z);
                }
            }
            "font-weight" | "fontWeight" => ts = ts.font_weight(v.to_owned()),
            "text-align" | "textAlign" => ts = ts.text_align(v.to_owned()),
            "box-shadow" | "boxShadow" => ts = ts.box_shadow(v.to_owned()),
            "overflow" => ts = ts.overflow(v.to_owned()),
            "object-fit" | "objectFit" => ts = ts.object_fit(v.to_owned()),
            _ => {}
        }
    }
    ts
}
