//! Zero-string-format theme surface & text style helpers directly extending [`TypedStyle`].
//!
//! By always emitting the full visual property set for surfaces (`background`, `border_width`,
//! `border_color`, `border_radius`, `opacity`, `box_shadows`) and text (`color`, `font_size`,
//! `font_weight`, `text_align`, `opacity`), state transitions such as `disabled -> enabled`,
//! `focused -> blurred`, `checked -> unchecked`, or `error -> valid` deterministically reset every
//! prior property with zero stale styles.

use nve_abi::ColorRgba;
use nve_dioxus::{TextAlign, TypedStyle};
use nve_tables::tables::BoxShadow;

/// Typed full visual surface specification emitted atomically by [`SurfaceStyleExt::surface`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedSurfaceStyle {
    pub background: ColorRgba,
    pub border_width: f32,
    pub border_color: ColorRgba,
    pub border_radius: f32,
    pub opacity: f32,
    pub box_shadow: Option<BoxShadow>,
}

impl Default for TypedSurfaceStyle {
    #[inline]
    fn default() -> Self {
        Self {
            background: ColorRgba::TRANSPARENT,
            border_width: 0.0,
            border_color: ColorRgba::TRANSPARENT,
            border_radius: 0.0,
            opacity: 1.0,
            box_shadow: None,
        }
    }
}

/// Typed full text visual specification emitted atomically by [`SurfaceStyleExt::text_style`].
/// No `Default`: text color always comes from the active theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedTextStyle {
    pub color: ColorRgba,
    pub font_size: f32,
    pub font_weight: u16,
    pub text_align: TextAlign,
    pub opacity: f32,
}

/// Extension trait adding deterministic full-surface and full-text helpers directly on [`TypedStyle`].
pub trait SurfaceStyleExt: Sized {
    /// Emits EVERY surface visual property explicitly with zero string formatting.
    fn surface(self, s: TypedSurfaceStyle) -> Self;

    /// Emits EVERY text visual property explicitly with zero string formatting.
    fn text_style(self, t: TypedTextStyle) -> Self;
}

impl SurfaceStyleExt for TypedStyle {
    #[inline]
    fn surface(self, s: TypedSurfaceStyle) -> Self {
        self.bg(s.background)
            .border(s.border_width, s.border_color)
            .radius(s.border_radius)
            .opacity(s.opacity)
            .box_shadows(s.box_shadow)
    }

    #[inline]
    fn text_style(self, t: TypedTextStyle) -> Self {
        self.color(t.color)
            .font_size(t.font_size)
            .font_weight(t.font_weight)
            .text_align(t.text_align)
            .opacity(t.opacity)
    }
}
