//! Semantic design tokens, built-in color schemes, `ThemeContext` bridge, and Dioxus
//! `ThemeProvider` / `use_theme()` hooks for `nve-ui`.

use std::borrow::Cow;

use dioxus_core::{provide_context, try_consume_context, use_hook, Element};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_interop::{ColorScheme, ThemeContext};

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

/// Provides a reactive [`UiTheme`] to all descendant `nve-ui` components.
#[component]
pub fn ThemeProvider(
    #[props(default = UiTheme::light())] theme: UiTheme,
    children: Element,
) -> Element {
    let mut ctx = use_hook(|| provide_context(Signal::new(theme.clone())));
    if *ctx.peek() != theme {
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

/// Returns the backing `Signal<UiTheme>` from the nearest [`ThemeProvider`], if mounted.
pub fn use_theme_signal() -> Option<Signal<UiTheme>> {
    try_consume_context::<Signal<UiTheme>>()
}
