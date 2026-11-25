//! Semantic design tokens, built-in color schemes, `ThemeContext` bridge, and Dioxus
//! `ThemeProvider` / `use_theme()` hooks for `nve-ui`.

use std::borrow::Cow;

use dioxus_core::{provide_context, try_consume_context, use_hook, Element};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_abi::ColorRgba;
use nve_interop::{ColorScheme, ThemeContext};

use crate::style_buf::parse_color_rgba;

/// Semantic color tokens shared by all `nve-ui` components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPalette {
    pub surface: Cow<'static, str>,
    pub surface_elevated: Cow<'static, str>,
    pub surface_subtle: Cow<'static, str>,
    pub text_primary: Cow<'static, str>,
    pub text_secondary: Cow<'static, str>,
    pub text_muted: Cow<'static, str>,
    pub border: Cow<'static, str>,
    pub border_focus: Cow<'static, str>,
    pub accent: Cow<'static, str>,
    pub accent_fg: Cow<'static, str>,
    pub danger: Cow<'static, str>,
    pub danger_fg: Cow<'static, str>,
    pub success: Cow<'static, str>,
    pub warning: Cow<'static, str>,
}

/// Pre-parsed [`ColorRgba`] companion to [`UiPalette`] so `nve-ui` components emit typed colors
/// directly into `TypedStyle` without re-parsing hex strings on every render frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPaletteRgba {
    pub surface: ColorRgba,
    pub surface_elevated: ColorRgba,
    pub surface_subtle: ColorRgba,
    pub text_primary: ColorRgba,
    pub text_secondary: ColorRgba,
    pub text_muted: ColorRgba,
    pub border: ColorRgba,
    pub border_focus: ColorRgba,
    pub accent: ColorRgba,
    pub accent_fg: ColorRgba,
    pub danger: ColorRgba,
    pub danger_fg: ColorRgba,
    pub success: ColorRgba,
    pub warning: ColorRgba,
}

impl UiPaletteRgba {
    pub const LIGHT: Self = Self {
        surface: ColorRgba::rgb(0xff, 0xff, 0xff),
        surface_elevated: ColorRgba::rgb(0xf8, 0xfa, 0xfc),
        surface_subtle: ColorRgba::rgb(0xf1, 0xf5, 0xf9),
        text_primary: ColorRgba::rgb(0x0f, 0x17, 0x2a),
        text_secondary: ColorRgba::rgb(0x33, 0x41, 0x55),
        text_muted: ColorRgba::rgb(0x64, 0x74, 0x8b),
        border: ColorRgba::rgb(0xcb, 0xd5, 0xe1),
        border_focus: ColorRgba::rgb(0x25, 0x63, 0xeb),
        accent: ColorRgba::rgb(0x25, 0x63, 0xeb),
        accent_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        danger: ColorRgba::rgb(0xdc, 0x26, 0x26),
        danger_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        success: ColorRgba::rgb(0x16, 0xa3, 0x4a),
        warning: ColorRgba::rgb(0xd9, 0x77, 0x06),
    };

    pub const DARK: Self = Self {
        surface: ColorRgba::rgb(0x0f, 0x17, 0x2a),
        surface_elevated: ColorRgba::rgb(0x1e, 0x29, 0x3b),
        surface_subtle: ColorRgba::rgb(0x17, 0x20, 0x33),
        text_primary: ColorRgba::rgb(0xf8, 0xfa, 0xfc),
        text_secondary: ColorRgba::rgb(0xcb, 0xd5, 0xe1),
        text_muted: ColorRgba::rgb(0x94, 0xa3, 0xb8),
        border: ColorRgba::rgb(0x33, 0x41, 0x55),
        border_focus: ColorRgba::rgb(0x60, 0xa5, 0xfa),
        accent: ColorRgba::rgb(0x3b, 0x82, 0xf6),
        accent_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        danger: ColorRgba::rgb(0xef, 0x44, 0x44),
        danger_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        success: ColorRgba::rgb(0x22, 0xc5, 0x5e),
        warning: ColorRgba::rgb(0xf5, 0x9e, 0x0b),
    };

    pub const HIGH_CONTRAST_LIGHT: Self = Self {
        surface: ColorRgba::rgb(0xff, 0xff, 0xff),
        surface_elevated: ColorRgba::rgb(0xff, 0xff, 0xff),
        surface_subtle: ColorRgba::rgb(0xf3, 0xf4, 0xf6),
        text_primary: ColorRgba::rgb(0x00, 0x00, 0x00),
        text_secondary: ColorRgba::rgb(0x11, 0x18, 0x27),
        text_muted: ColorRgba::rgb(0x37, 0x41, 0x51),
        border: ColorRgba::rgb(0x00, 0x00, 0x00),
        border_focus: ColorRgba::rgb(0x00, 0x33, 0xcc),
        accent: ColorRgba::rgb(0x00, 0x33, 0xcc),
        accent_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        danger: ColorRgba::rgb(0x99, 0x1b, 0x1b),
        danger_fg: ColorRgba::rgb(0xff, 0xff, 0xff),
        success: ColorRgba::rgb(0x14, 0x53, 0x2d),
        warning: ColorRgba::rgb(0x92, 0x40, 0x0e),
    };

    pub const HIGH_CONTRAST_DARK: Self = Self {
        surface: ColorRgba::rgb(0x00, 0x00, 0x00),
        surface_elevated: ColorRgba::rgb(0x0a, 0x0a, 0x0a),
        surface_subtle: ColorRgba::rgb(0x14, 0x14, 0x14),
        text_primary: ColorRgba::rgb(0xff, 0xff, 0xff),
        text_secondary: ColorRgba::rgb(0xf3, 0xf4, 0xf6),
        text_muted: ColorRgba::rgb(0xd1, 0xd5, 0xdb),
        border: ColorRgba::rgb(0xff, 0xff, 0xff),
        border_focus: ColorRgba::rgb(0x7d, 0xd3, 0xfc),
        accent: ColorRgba::rgb(0x38, 0xbd, 0xf8),
        accent_fg: ColorRgba::rgb(0x00, 0x00, 0x00),
        danger: ColorRgba::rgb(0xff, 0x6b, 0x6b),
        danger_fg: ColorRgba::rgb(0x00, 0x00, 0x00),
        success: ColorRgba::rgb(0x4a, 0xde, 0x80),
        warning: ColorRgba::rgb(0xfa, 0xcc, 0x15),
    };

    /// Parses every token in `palette` into a `Copy` [`UiPaletteRgba`], falling back to
    /// [`ColorRgba::TRANSPARENT`] if any custom token is malformed.
    pub fn from_palette(palette: &UiPalette) -> Self {
        let p = |raw: &str| parse_color_rgba(raw).unwrap_or(ColorRgba::TRANSPARENT);
        Self {
            surface: p(palette.surface.as_ref()),
            surface_elevated: p(palette.surface_elevated.as_ref()),
            surface_subtle: p(palette.surface_subtle.as_ref()),
            text_primary: p(palette.text_primary.as_ref()),
            text_secondary: p(palette.text_secondary.as_ref()),
            text_muted: p(palette.text_muted.as_ref()),
            border: p(palette.border.as_ref()),
            border_focus: p(palette.border_focus.as_ref()),
            accent: p(palette.accent.as_ref()),
            accent_fg: p(palette.accent_fg.as_ref()),
            danger: p(palette.danger.as_ref()),
            danger_fg: p(palette.danger_fg.as_ref()),
            success: p(palette.success.as_ref()),
            warning: p(palette.warning.as_ref()),
        }
    }
}

impl UiPalette {
    /// Converts this string-backed palette into pre-parsed [`ColorRgba`] tokens.
    pub fn to_rgba(&self) -> UiPaletteRgba {
        UiPaletteRgba::from_palette(self)
    }
}

/// Spacing scale in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingScale {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl SpacingScale {
    pub const NORMAL: Self = Self {
        xs: 4.0,
        sm: 8.0,
        md: 12.0,
        lg: 16.0,
        xl: 24.0,
    };

    pub const COMPACT: Self = Self {
        xs: 2.0,
        sm: 6.0,
        md: 8.0,
        lg: 12.0,
        xl: 18.0,
    };
}

/// Border-radius scale in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadiiScale {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub full: f32,
}

impl Default for RadiiScale {
    fn default() -> Self {
        Self {
            sm: 4.0,
            md: 8.0,
            lg: 12.0,
            full: 999.0,
        }
    }
}

/// Typography size scale in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyScale {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Default for TypographyScale {
    fn default() -> Self {
        Self {
            xs: 11.0,
            sm: 13.0,
            md: 15.0,
            lg: 18.0,
            xl: 24.0,
        }
    }
}

/// Layout density preset controlling control padding and spacing rhythm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiDensity {
    #[default]
    Normal,
    Compact,
}

/// Complete design-token theme consumed by `nve-ui` components.
#[derive(Debug, Clone, PartialEq)]
pub struct UiTheme {
    pub color_scheme: ColorScheme,
    pub palette: UiPalette,
    pub spacing: SpacingScale,
    pub radii: RadiiScale,
    pub typography: TypographyScale,
    pub density: UiDensity,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self::light()
    }
}

impl UiTheme {
    /// Clean light theme with WCAG AA+ contrast.
    pub fn light() -> Self {
        Self {
            color_scheme: ColorScheme::Light,
            palette: UiPalette {
                surface: Cow::Borrowed("#ffffff"),
                surface_elevated: Cow::Borrowed("#f8fafc"),
                surface_subtle: Cow::Borrowed("#f1f5f9"),
                text_primary: Cow::Borrowed("#0f172a"),
                text_secondary: Cow::Borrowed("#334155"),
                text_muted: Cow::Borrowed("#64748b"),
                border: Cow::Borrowed("#cbd5e1"),
                border_focus: Cow::Borrowed("#2563eb"),
                accent: Cow::Borrowed("#2563eb"),
                accent_fg: Cow::Borrowed("#ffffff"),
                danger: Cow::Borrowed("#dc2626"),
                danger_fg: Cow::Borrowed("#ffffff"),
                success: Cow::Borrowed("#16a34a"),
                warning: Cow::Borrowed("#d97706"),
            },
            spacing: SpacingScale::NORMAL,
            radii: RadiiScale::default(),
            typography: TypographyScale::default(),
            density: UiDensity::Normal,
        }
    }

    /// Slate dark theme with legible elevated surfaces and focus ring.
    pub fn dark() -> Self {
        Self {
            color_scheme: ColorScheme::Dark,
            palette: UiPalette {
                surface: Cow::Borrowed("#0f172a"),
                surface_elevated: Cow::Borrowed("#1e293b"),
                surface_subtle: Cow::Borrowed("#172033"),
                text_primary: Cow::Borrowed("#f8fafc"),
                text_secondary: Cow::Borrowed("#cbd5e1"),
                text_muted: Cow::Borrowed("#94a3b8"),
                border: Cow::Borrowed("#334155"),
                border_focus: Cow::Borrowed("#60a5fa"),
                accent: Cow::Borrowed("#3b82f6"),
                accent_fg: Cow::Borrowed("#ffffff"),
                danger: Cow::Borrowed("#ef4444"),
                danger_fg: Cow::Borrowed("#ffffff"),
                success: Cow::Borrowed("#22c55e"),
                warning: Cow::Borrowed("#f59e0b"),
            },
            spacing: SpacingScale::NORMAL,
            radii: RadiiScale::default(),
            typography: TypographyScale::default(),
            density: UiDensity::Normal,
        }
    }

    /// High-contrast light theme meeting WCAG AAA foreground/border ratios.
    pub fn high_contrast_light() -> Self {
        Self {
            color_scheme: ColorScheme::HighContrastLight,
            palette: UiPalette {
                surface: Cow::Borrowed("#ffffff"),
                surface_elevated: Cow::Borrowed("#ffffff"),
                surface_subtle: Cow::Borrowed("#f3f4f6"),
                text_primary: Cow::Borrowed("#000000"),
                text_secondary: Cow::Borrowed("#111827"),
                text_muted: Cow::Borrowed("#374151"),
                border: Cow::Borrowed("#000000"),
                border_focus: Cow::Borrowed("#0033cc"),
                accent: Cow::Borrowed("#0033cc"),
                accent_fg: Cow::Borrowed("#ffffff"),
                danger: Cow::Borrowed("#991b1b"),
                danger_fg: Cow::Borrowed("#ffffff"),
                success: Cow::Borrowed("#14532d"),
                warning: Cow::Borrowed("#92400e"),
            },
            spacing: SpacingScale::NORMAL,
            radii: RadiiScale::default(),
            typography: TypographyScale::default(),
            density: UiDensity::Normal,
        }
    }

    /// High-contrast dark theme meeting WCAG AAA foreground/border ratios.
    pub fn high_contrast_dark() -> Self {
        Self {
            color_scheme: ColorScheme::HighContrastDark,
            palette: UiPalette {
                surface: Cow::Borrowed("#000000"),
                surface_elevated: Cow::Borrowed("#0a0a0a"),
                surface_subtle: Cow::Borrowed("#141414"),
                text_primary: Cow::Borrowed("#ffffff"),
                text_secondary: Cow::Borrowed("#f3f4f6"),
                text_muted: Cow::Borrowed("#d1d5db"),
                border: Cow::Borrowed("#ffffff"),
                border_focus: Cow::Borrowed("#7dd3fc"),
                accent: Cow::Borrowed("#38bdf8"),
                accent_fg: Cow::Borrowed("#000000"),
                danger: Cow::Borrowed("#ff6b6b"),
                danger_fg: Cow::Borrowed("#000000"),
                success: Cow::Borrowed("#4ade80"),
                warning: Cow::Borrowed("#facc15"),
            },
            spacing: SpacingScale::NORMAL,
            radii: RadiiScale::default(),
            typography: TypographyScale::default(),
            density: UiDensity::Normal,
        }
    }

    /// Builds a [`UiTheme`] from an `nve_interop::ThemeContext`, honoring `color_scheme`,
    /// `accent_rgba`, and any semantic token overrides in `ctx.tokens`.
    pub fn from_theme_context(ctx: &ThemeContext) -> Self {
        let mut theme = match ctx.color_scheme {
            ColorScheme::Light | ColorScheme::System => Self::light(),
            ColorScheme::Dark => Self::dark(),
            ColorScheme::HighContrastLight => Self::high_contrast_light(),
            ColorScheme::HighContrastDark => Self::high_contrast_dark(),
        };

        if ctx.accent_rgba != 0 {
            let accent_hex = rgba_to_hex(ctx.accent_rgba);
            theme.palette.accent = Cow::Owned(accent_hex.clone());
            theme.palette.border_focus = Cow::Owned(accent_hex);
            theme.palette.accent_fg = Cow::Borrowed(contrast_fg_for_rgba(ctx.accent_rgba));
        }

        for (k, v) in &ctx.tokens {
            if v.trim().is_empty() {
                continue;
            }
            match k.as_str() {
                "surface" | "surface.bg" => theme.palette.surface = Cow::Owned(v.clone()),
                "surface_elevated" | "surface.elevated" => {
                    theme.palette.surface_elevated = Cow::Owned(v.clone());
                }
                "surface_subtle" | "surface.subtle" => {
                    theme.palette.surface_subtle = Cow::Owned(v.clone());
                }
                "text_primary" | "text.primary" => {
                    theme.palette.text_primary = Cow::Owned(v.clone());
                }
                "text_secondary" | "text.secondary" => {
                    theme.palette.text_secondary = Cow::Owned(v.clone());
                }
                "text_muted" | "text.muted" => theme.palette.text_muted = Cow::Owned(v.clone()),
                "border" | "border.default" => theme.palette.border = Cow::Owned(v.clone()),
                "border_focus" | "border.focus" => {
                    theme.palette.border_focus = Cow::Owned(v.clone());
                }
                "accent" | "accent.bg" => theme.palette.accent = Cow::Owned(v.clone()),
                "accent_fg" | "accent.fg" => theme.palette.accent_fg = Cow::Owned(v.clone()),
                "danger" | "danger.bg" => theme.palette.danger = Cow::Owned(v.clone()),
                "danger_fg" | "danger.fg" => theme.palette.danger_fg = Cow::Owned(v.clone()),
                "success" => theme.palette.success = Cow::Owned(v.clone()),
                "warning" => theme.palette.warning = Cow::Owned(v.clone()),
                _ => {}
            }
        }

        theme
    }

    /// Applies [`UiDensity`] and updates the spacing scale accordingly.
    pub fn with_density(mut self, density: UiDensity) -> Self {
        self.density = density;
        self.spacing = match density {
            UiDensity::Normal => SpacingScale::NORMAL,
            UiDensity::Compact => SpacingScale::COMPACT,
        };
        self
    }

    /// Returns `true` for dark and high-contrast dark color schemes.
    pub fn is_dark(&self) -> bool {
        matches!(
            self.color_scheme,
            ColorScheme::Dark | ColorScheme::HighContrastDark
        )
    }

    /// Returns `true` for high-contrast light or dark color schemes.
    pub fn is_high_contrast(&self) -> bool {
        matches!(
            self.color_scheme,
            ColorScheme::HighContrastLight | ColorScheme::HighContrastDark
        )
    }

    /// Elevation box shadow matching the active theme polarity.
    pub fn elevation_shadow(&self) -> &'static str {
        if self.is_high_contrast() {
            "none"
        } else if self.is_dark() {
            "0 4px 16px rgba(0, 0, 0, 0.45)"
        } else {
            "0 4px 12px rgba(15, 23, 42, 0.10)"
        }
    }

    /// Translucent overlay backdrop scrim matching the active theme polarity.
    pub fn backdrop_scrim(&self) -> &'static str {
        if self.is_dark() {
            "#000000b3"
        } else {
            "#0f172a80"
        }
    }

    /// Pre-parsed translucent overlay backdrop scrim matching the active theme polarity.
    #[inline]
    pub const fn backdrop_scrim_rgba(&self) -> ColorRgba {
        match self.color_scheme {
            ColorScheme::Dark | ColorScheme::HighContrastDark => ColorRgba::rgba(0x00, 0x00, 0x00, 0xb3),
            ColorScheme::Light | ColorScheme::System | ColorScheme::HighContrastLight => {
                ColorRgba::rgba(0x0f, 0x17, 0x2a, 0x80)
            }
        }
    }

    /// Returns pre-parsed [`UiPaletteRgba`] colors for this theme's palette.
    pub fn palette_rgba(&self) -> UiPaletteRgba {
        self.palette.to_rgba()
    }
}

/// Formats a packed `0xRRGGBBAA` color into `#rrggbb` (opaque) or `#rrggbbaa`.
pub fn rgba_to_hex(rgba: u32) -> String {
    let r = (rgba >> 24) & 0xff;
    let g = (rgba >> 16) & 0xff;
    let b = (rgba >> 8) & 0xff;
    let a = rgba & 0xff;
    if a == 0xff {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Picks a high-contrast foreground color (`#ffffff` or `#0f172a`) for a packed `0xRRGGBBAA` fill.
fn contrast_fg_for_rgba(rgba: u32) -> &'static str {
    let r = ((rgba >> 24) & 0xff) as f32 / 255.0;
    let g = ((rgba >> 16) & 0xff) as f32 / 255.0;
    let b = ((rgba >> 8) & 0xff) as f32 / 255.0;
    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luma > 0.58 {
        "#0f172a"
    } else {
        "#ffffff"
    }
}

/// Provides a reactive [`UiTheme`] and pre-parsed [`UiPaletteRgba`] to all descendant `nve-ui`
/// components so themed renders never re-parse hex strings per frame.
#[component]
pub fn ThemeProvider(
    #[props(default = UiTheme::light())] theme: UiTheme,
    children: Element,
) -> Element {
    let mut ctx = use_hook(|| provide_context(Signal::new(theme.clone())));
    let mut rgba_ctx = use_hook(|| {
        provide_context(Signal::new(UiPaletteRgba::from_palette(&theme.palette)))
    });
    if *ctx.peek() != theme {
        rgba_ctx.set(UiPaletteRgba::from_palette(&theme.palette));
        ctx.set(theme);
    }
    rsx! { {children} }
}

/// Reads the nearest [`UiTheme`] from context, subscribing the component to theme switches.
///
/// Falls back to [`UiTheme::light()`] when rendered outside a [`ThemeProvider`].
pub fn use_theme() -> UiTheme {
    try_consume_context::<Signal<UiTheme>>()
        .map(|sig| sig.read().clone())
        .unwrap_or_else(UiTheme::light)
}

/// Reads both the active [`UiTheme`] and its pre-parsed [`UiPaletteRgba`] from context without
/// re-parsing hex color strings on every render frame.
pub fn use_theme_rgba() -> (UiTheme, UiPaletteRgba) {
    let theme = use_theme();
    let colors = try_consume_context::<Signal<UiPaletteRgba>>()
        .map(|sig| *sig.read())
        .unwrap_or_else(|| {
            if theme == UiTheme::light() {
                UiPaletteRgba::LIGHT
            } else {
                theme.palette_rgba()
            }
        });
    (theme, colors)
}

/// Returns the backing `Signal<UiTheme>` from the nearest [`ThemeProvider`], if mounted.
pub fn use_theme_signal() -> Option<Signal<UiTheme>> {
    try_consume_context::<Signal<UiTheme>>()
}
