//! Unified compile-time OKLCH design-token system for the whole NVE product family.
//!
//! One neutral graphite base scale and per-brand accent hues generate every preset at compile
//! time ([`Theme::preset`]): Light / Dark / High-Contrast Light / High-Contrast Dark ×
//! [`AppBrand::Default`] / [`AppBrand::Docs`] / [`AppBrand::Studio`]. [`ThemeConfig`] follows the
//! OS [`Appearance`] by default ([`ThemeMode::System`]) and accepts a runtime accent override.
//! Contrast of every role pair is verified by `tests/theme_contrast_tests.rs` (WCAG 2.1 + APCA).

mod lch;
mod presets;
mod scale;
mod tokens;

use core::fmt;

use dioxus_core::{provide_context, try_consume_context, use_hook, Element};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_abi::ColorRgba;

pub use lch::{apca_contrast, composite, relative_luminance, wcag_contrast, with_alpha, Oklch};
pub use presets::{AppBrand, Appearance, Theme, ThemeConfig, ThemeMode};
pub use scale::{Accent, Polarity, Scale, Tone};
pub use tokens::{
    Border, Breakpoint, CubicBezier, Elevation, FontStack, Layer, Motion, Radius, Spacing, Spring,
    StateLayer, Status, Surface, Syntax, Text, TypeStyle, Typography,
};

/// Zero-allocation `#rrggbb` / `#rrggbbaa` formatter for the remaining Phase-0 `style:` string
/// bridge (deleted together with `style:` strings in Phase 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hex(pub ColorRgba);

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ColorRgba { r, g, b, a } = self.0;
        if a == 0xff {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else {
            write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }
}

/// `color.hex()` in `style:` format strings.
pub trait ColorHex {
    fn hex(self) -> Hex;
}

impl ColorHex for ColorRgba {
    #[inline]
    fn hex(self) -> Hex {
        Hex(self)
    }
}

/// Provides a reactive [`Theme`] to all descendants; updates propagate only when it changes.
#[component]
pub fn ThemeProvider(#[props(default)] theme: Theme, children: Element) -> Element {
    let mut ctx = use_hook(|| provide_context(Signal::new(theme)));
    if *ctx.peek() != theme {
        ctx.set(theme);
    }
    rsx! { {children} }
}

/// Reads the nearest [`Theme`] (subscribing to switches); [`Theme::LIGHT`] outside a provider.
pub fn use_theme() -> Theme {
    try_consume_context::<Signal<Theme>>().map_or(Theme::LIGHT, |sig| *sig.read())
}

/// The backing `Signal<Theme>` of the nearest [`ThemeProvider`], if mounted.
pub fn use_theme_signal() -> Option<Signal<Theme>> {
    try_consume_context::<Signal<Theme>>()
}
