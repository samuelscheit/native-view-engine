//! Zero-external-CSS style builders guaranteeing **explicit full-property emission**.
//!
//! Provides both:
//! - [`TypedStyleBuilder`] / [`TypedSurfaceStyle`] / [`TypedTextStyle`] for **zero-string-format,
//!   zero-CSS-parse** emission straight into `nve_dioxus::TypedStyle` (`nve_style` attribute).
//! - [`StyleBuf`] / [`SurfaceStyle`] / [`TextStyle`] for string-based callers (with
//!   [`StyleBuf::to_typed`] / [`StyleBuf::into_typed`] bridge helpers).
//!
//! By always emitting the full property set for visual surfaces (`background-color`,
//! `border-width`, `border-color`, `border-radius`, `opacity`, `box-shadow`) and text
//! (`color`, `font-size`, `font-weight`, `text-align`, `opacity`), state transitions such as
//! `disabled -> enabled`, `focused -> blurred`, `checked -> unchecked`, or `error -> valid`
//! deterministically reset every prior property with zero stale styles.

mod typed;

use std::fmt::Write as _;

use nve_dioxus::TypedStyle;

pub use typed::{parse_color_rgba, TypedStyleBuilder, TypedSurfaceStyle, TypedTextStyle};

/// Full visual surface specification emitted atomically by [`StyleBuf::surface`].
#[derive(Debug, Clone, Copy)]
pub struct SurfaceStyle<'a> {
    pub background: &'a str,
    pub border_width: f32,
    pub border_color: &'a str,
    pub border_radius: f32,
    pub opacity: f32,
    pub box_shadow: &'a str,
}

impl<'a> Default for SurfaceStyle<'a> {
    fn default() -> Self {
        Self {
            background: "transparent",
            border_width: 0.0,
            border_color: "transparent",
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: "none",
        }
    }
}

/// Full text visual specification emitted atomically by [`StyleBuf::text_style`].
#[derive(Debug, Clone, Copy)]
pub struct TextStyle<'a> {
    pub color: &'a str,
    pub font_size: f32,
    pub font_weight: &'a str,
    pub text_align: &'a str,
    pub opacity: f32,
}

impl<'a> Default for TextStyle<'a> {
    fn default() -> Self {
        Self {
            color: "#0f172a",
            font_size: 15.0,
            font_weight: "400",
            text_align: "left",
            opacity: 1.0,
        }
    }
}

/// Pre-allocated inline style builder producing canonical CSS/RN declarations accepted directly
/// by `nve-components::apply_typed_attr_with_raw`, or convertible to [`TypedStyle`] via
/// [`StyleBuf::into_typed`].
#[derive(Debug, Clone, Default)]
pub struct StyleBuf {
    buf: String,
}

impl StyleBuf {
    /// Creates an empty style buffer with default capacity suitable for typical UI controls.
    pub fn new() -> Self {
        Self {
            buf: String::with_capacity(256),
        }
    }

    /// Creates an empty style buffer with explicit byte capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }

    #[inline]
    fn sanitize(v: f32) -> f32 {
        if v.is_finite() { v } else { 0.0 }
    }

    #[inline]
    fn write_px(&mut self, key: &str, value: f32) -> &mut Self {
        let v = Self::sanitize(value);
        let _ = write!(self.buf, "{key}: {v}px; ");
        self
    }

    pub fn width_px(&mut self, v: f32) -> &mut Self {
        self.write_px("width", v.max(0.0))
    }

    pub fn width_pct(&mut self, v: f32) -> &mut Self {
        let v = Self::sanitize(v).clamp(0.0, 100.0);
        let _ = write!(self.buf, "width: {v}%; ");
        self
    }

    pub fn width_auto(&mut self) -> &mut Self {
        self.buf.push_str("width: auto; ");
        self
    }

    pub fn height_px(&mut self, v: f32) -> &mut Self {
        self.write_px("height", v.max(0.0))
    }

    pub fn height_pct(&mut self, v: f32) -> &mut Self {
        let v = Self::sanitize(v).clamp(0.0, 100.0);
        let _ = write!(self.buf, "height: {v}%; ");
        self
    }

    pub fn height_auto(&mut self) -> &mut Self {
        self.buf.push_str("height: auto; ");
        self
    }

    pub fn min_width_px(&mut self, v: f32) -> &mut Self {
        self.write_px("min-width", v.max(0.0))
    }

    pub fn min_height_px(&mut self, v: f32) -> &mut Self {
        self.write_px("min-height", v.max(0.0))
    }

    pub fn max_width_px(&mut self, v: f32) -> &mut Self {
        self.write_px("max-width", v.max(0.0))
    }

    pub fn max_height_px(&mut self, v: f32) -> &mut Self {
        self.write_px("max-height", v.max(0.0))
    }

    pub fn flex_direction(&mut self, dir: &str) -> &mut Self {
        let _ = write!(self.buf, "flex-direction: {dir}; ");
        self
    }

    pub fn justify_content(&mut self, justify: &str) -> &mut Self {
        let _ = write!(self.buf, "justify-content: {justify}; ");
        self
    }

    pub fn align_items(&mut self, align: &str) -> &mut Self {
        let _ = write!(self.buf, "align-items: {align}; ");
        self
    }

    pub fn flex_grow(&mut self, v: f32) -> &mut Self {
        let v = Self::sanitize(v).max(0.0);
        let _ = write!(self.buf, "flex-grow: {v}; ");
        self
    }

    pub fn flex_shrink(&mut self, v: f32) -> &mut Self {
        let v = Self::sanitize(v).max(0.0);
        let _ = write!(self.buf, "flex-shrink: {v}; ");
        self
    }

    pub fn gap(&mut self, v: f32) -> &mut Self {
        self.write_px("gap", v.max(0.0))
    }

    pub fn padding(&mut self, v: f32) -> &mut Self {
        self.write_px("padding", v.max(0.0))
    }

    pub fn padding_xy(&mut self, horizontal: f32, vertical: f32) -> &mut Self {
        self.write_px("padding-horizontal", horizontal.max(0.0));
        self.write_px("padding-vertical", vertical.max(0.0))
    }

    pub fn margin(&mut self, v: f32) -> &mut Self {
        self.write_px("margin", v)
    }

    pub fn background(&mut self, color: &str) -> &mut Self {
        let _ = write!(self.buf, "background-color: {color}; ");
        self
    }

    pub fn color(&mut self, color: &str) -> &mut Self {
        let _ = write!(self.buf, "color: {color}; ");
        self
    }

    pub fn border(&mut self, width: f32, color: &str) -> &mut Self {
        self.write_px("border-width", width.max(0.0));
        let _ = write!(self.buf, "border-color: {color}; ");
        self
    }

    pub fn border_radius(&mut self, radius: f32) -> &mut Self {
        self.write_px("border-radius", radius.max(0.0))
    }

    pub fn opacity(&mut self, opacity: f32) -> &mut Self {
        let v = Self::sanitize(opacity).clamp(0.0, 1.0);
        let _ = write!(self.buf, "opacity: {v}; ");
        self
    }

    pub fn font_size(&mut self, px: f32) -> &mut Self {
        self.write_px("font-size", px.max(1.0))
    }

    pub fn font_weight(&mut self, weight: &str) -> &mut Self {
        let _ = write!(self.buf, "font-weight: {weight}; ");
        self
    }

    pub fn text_align(&mut self, align: &str) -> &mut Self {
        let _ = write!(self.buf, "text-align: {align}; ");
        self
    }

    pub fn box_shadow(&mut self, shadow: &str) -> &mut Self {
        let _ = write!(self.buf, "box-shadow: {shadow}; ");
        self
    }

    pub fn z_index(&mut self, z: i32) -> &mut Self {
        let _ = write!(self.buf, "z-index: {z}; ");
        self
    }

    pub fn overflow(&mut self, mode: &str) -> &mut Self {
        let _ = write!(self.buf, "overflow: {mode}; ");
        self
    }

    /// Appends arbitrary user style declarations (trailing `;` added automatically if needed).
    pub fn raw(&mut self, extra: &str) -> &mut Self {
        let trimmed = extra.trim();
        if !trimmed.is_empty() {
            self.buf.push_str(trimmed);
            if !trimmed.ends_with(';') {
                self.buf.push(';');
            }
            self.buf.push(' ');
        }
        self
    }

    /// Emits EVERY surface visual property explicitly so incremental diffs never leave stale
    /// borders, backgrounds, opacity, radii, or shadows across state transitions.
    pub fn surface(&mut self, s: SurfaceStyle<'_>) -> &mut Self {
        self.background(s.background)
            .border(s.border_width, s.border_color)
            .border_radius(s.border_radius)
            .opacity(s.opacity)
            .box_shadow(s.box_shadow)
    }

    /// Emits EVERY text visual property explicitly so state transitions never leave stale text
    /// color, weight, size, alignment, or opacity.
    pub fn text_style(&mut self, t: TextStyle<'_>) -> &mut Self {
        self.color(t.color)
            .font_size(t.font_size)
            .font_weight(t.font_weight)
            .text_align(t.text_align)
            .opacity(t.opacity)
    }

    pub fn as_str(&self) -> &str {
        self.buf.trim_end()
    }

    pub fn finish(mut self) -> String {
        while self.buf.ends_with(' ') {
            self.buf.pop();
        }
        self.buf
    }

    /// Converts accumulated declarations into a [`TypedStyle`] for direct `nve_style` binding.
    pub fn to_typed(&self) -> TypedStyle {
        typed::parse_inline_css_to_typed(&self.buf)
    }

    /// Consumes this buffer and converts its declarations into a [`TypedStyle`].
    pub fn into_typed(self) -> TypedStyle {
        self.to_typed()
    }
}
