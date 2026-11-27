//! The resolved [`Theme`], brand presets, OS-following [`ThemeConfig`], and compile-time preset
//! table (4 polarities × 3 brands, all `const`).

use nve_abi::{ColorRgba, ColorScheme, ContrastLevel};
use nve_interop::ThemeContext;
use nve_interop::traits::UiDensity;

use super::lch::{with_alpha, Oklch};
use super::scale::{Accent, Polarity, Scale, Tone, NEUTRAL_HUE};
use super::tokens::{
    Border, Elevation, Motion, Radius, Spacing, StateLayer, Status, Surface, Syntax, Text,
    Typography,
};

/// Product family member; each brand derives its accent from the shared base scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppBrand {
    #[default]
    Default,
    Docs,
    Studio,
}

impl AppBrand {
    pub const ALL: [Self; 3] = [Self::Default, Self::Docs, Self::Studio];

    pub const fn accent(self) -> Accent {
        match self {
            Self::Default => Accent::BLUE,
            Self::Docs => Accent::INDIGO,
            Self::Studio => Accent::CYAN,
        }
    }
}

/// User color-scheme preference; [`ThemeMode::System`] follows the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const fn resolve(self, os: ColorScheme) -> ColorScheme {
        match self {
            Self::System => os,
            Self::Light => ColorScheme::Light,
            Self::Dark => ColorScheme::Dark,
        }
    }
}

/// The host's current appearance (`nve-system-env` / trait environment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Appearance {
    pub scheme: ColorScheme,
    pub contrast: ContrastLevel,
}

impl Appearance {
    pub const LIGHT: Self = Self { scheme: ColorScheme::Light, contrast: ContrastLevel::Standard };
    pub const DARK: Self = Self { scheme: ColorScheme::Dark, contrast: ContrastLevel::Standard };
}

/// Unresolved theme preferences: resolved against the OS [`Appearance`] by [`Self::resolve`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeConfig {
    pub mode: ThemeMode,
    /// `None` follows the OS contrast preference.
    pub contrast: Option<ContrastLevel>,
    pub brand: AppBrand,
    /// Runtime accent override (e.g. Studio's per-workspace accent).
    pub accent: Option<Accent>,
    pub density: UiDensity,
}

impl ThemeConfig {
    pub const fn new(brand: AppBrand) -> Self {
        Self {
            mode: ThemeMode::System,
            contrast: None,
            brand,
            accent: None,
            density: UiDensity::Comfortable,
        }
    }

    pub const fn resolve(self, os: Appearance) -> Theme {
        let contrast = match self.contrast {
            Some(c) => c,
            None => os.contrast,
        };
        let theme = Theme::preset(Polarity::new(self.mode.resolve(os.scheme), contrast), self.brand)
            .with_density(self.density);
        match self.accent {
            Some(accent) => theme.with_accent(accent),
            None => theme,
        }
    }
}

/// Complete resolved design-token set consumed by every `nve-ui` component, `nve-docs`, and
/// Studio. `Copy`: reading it from context never allocates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub polarity: Polarity,
    pub brand: AppBrand,
    pub accent: Accent,
    pub surface: Surface,
    pub state: StateLayer,
    pub text: Text,
    pub border: Border,
    pub status: Status,
    pub syntax: Syntax,
    pub scrim: ColorRgba,
    pub elevation: Elevation,
    pub typography: Typography,
    pub spacing: Spacing,
    pub radius: Radius,
    pub motion: Motion,
    pub density: UiDensity,
}

impl Default for Theme {
    fn default() -> Self {
        Self::LIGHT
    }
}

const SUCCESS: Accent = Accent::GREEN;
const WARNING: Accent = Accent::AMBER;
const DANGER: Accent = Accent::RED;
const INFO: Accent = Accent::new(245.0, 0.16, 0.53);

const fn syntax_hue(p: Polarity, hue: f32) -> ColorRgba {
    let (l, c) = p.profile().syntax;
    Oklch::new(l, c, hue).to_srgb()
}

impl Theme {
    pub const LIGHT: Self = Self::build(Polarity::Light, AppBrand::Default);
    pub const DARK: Self = Self::build(Polarity::Dark, AppBrand::Default);
    pub const HIGH_CONTRAST_LIGHT: Self = Self::build(Polarity::HighContrastLight, AppBrand::Default);
    pub const HIGH_CONTRAST_DARK: Self = Self::build(Polarity::HighContrastDark, AppBrand::Default);

    const PRESETS: [[Self; 3]; 4] = {
        const fn row(p: Polarity) -> [Theme; 3] {
            [
                Theme::build(p, AppBrand::Default),
                Theme::build(p, AppBrand::Docs),
                Theme::build(p, AppBrand::Studio),
            ]
        }
        [
            row(Polarity::Light),
            row(Polarity::Dark),
            row(Polarity::HighContrastLight),
            row(Polarity::HighContrastDark),
        ]
    };

    /// Compile-time preset lookup (a table index; no color math at runtime).
    pub const fn preset(polarity: Polarity, brand: AppBrand) -> Self {
        let row = match polarity {
            Polarity::Light => 0,
            Polarity::Dark => 1,
            Polarity::HighContrastLight => 2,
            Polarity::HighContrastDark => 3,
        };
        let col = match brand {
            AppBrand::Default => 0,
            AppBrand::Docs => 1,
            AppBrand::Studio => 2,
        };
        Self::PRESETS[row][col]
    }

    /// Generates every token for `polarity` and `brand` from the shared base scales.
    pub const fn build(polarity: Polarity, brand: AppBrand) -> Self {
        let n = Scale::neutral(polarity);
        let p = polarity.profile();
        let dark = polarity.is_dark();
        let hc = polarity.is_high_contrast();
        let ink = n.step::<12>();
        let white = ColorRgba::WHITE;
        let (base, raised, overlay, sunken, subtle) = if dark {
            (n.step::<2>(), n.step::<3>(), n.step::<4>(), n.step::<1>(), n.step::<3>())
        } else {
            (white, white, white, n.step::<2>(), n.step::<2>())
        };
        let tertiary = Oklch::new(p.tertiary_l, p.neutral_c[10], NEUTRAL_HUE).to_srgb();
        let (hairline_subtle, hairline_default) = match polarity {
            Polarity::Light => (with_alpha(ink, 0x12), with_alpha(ink, 0x1f)),
            Polarity::Dark => (with_alpha(ink, 0x10), with_alpha(ink, 0x1a)),
            Polarity::HighContrastLight | Polarity::HighContrastDark => (n.step::<6>(), n.step::<7>()),
        };
        let (hover_a, active_a) = match polarity {
            Polarity::Light => (0x0b, 0x14),
            Polarity::Dark => (0x0e, 0x17),
            Polarity::HighContrastLight => (0x12, 0x20),
            Polarity::HighContrastDark => (0x1a, 0x2e),
        };
        let success = Tone::derive(polarity, SUCCESS);
        let danger = Tone::derive(polarity, DANGER);
        let theme = Self {
            polarity,
            brand,
            accent: brand.accent(),
            surface: Surface { canvas: n.step::<1>(), base, raised, overlay, sunken, subtle },
            state: StateLayer {
                hover: with_alpha(ink, hover_a),
                active: with_alpha(ink, active_a),
                selected: ColorRgba::TRANSPARENT,
            },
            text: Text {
                primary: ink,
                secondary: n.step::<11>(),
                tertiary,
                link: ColorRgba::TRANSPARENT,
                on_accent: ColorRgba::TRANSPARENT,
            },
            border: Border {
                subtle: hairline_subtle,
                default: hairline_default,
                strong: n.step::<8>(),
                focus: ColorRgba::TRANSPARENT,
            },
            status: Status {
                // Derived once by the trailing `with_accent(brand.accent())`.
                accent: Tone::UNSET,
                success,
                warning: Tone::derive(polarity, WARNING),
                danger,
                info: Tone::derive(polarity, INFO),
            },
            syntax: Syntax {
                plain: ink,
                keyword: syntax_hue(polarity, 302.0),
                type_name: syntax_hue(polarity, 222.0),
                string: syntax_hue(polarity, 145.0),
                comment: tertiary,
                number: syntax_hue(polarity, 50.0),
                function: syntax_hue(polarity, 258.0),
                macro_name: syntax_hue(polarity, 350.0),
                attribute: syntax_hue(polarity, 85.0),
                punctuation: n.step::<11>(),
                diff_add: success.text,
                diff_del: danger.text,
            },
            scrim: if dark {
                ColorRgba::rgba(0, 0, 0, 0xb3)
            } else {
                with_alpha(n.step::<12>(), 0x73)
            },
            elevation: if hc {
                Elevation::FLAT
            } else if dark {
                Elevation::DARK
            } else {
                Elevation::LIGHT
            },
            typography: Typography::DEFAULT,
            spacing: Spacing::for_density(UiDensity::Comfortable),
            radius: Radius::DEFAULT,
            motion: Motion::DEFAULT,
            density: UiDensity::Comfortable,
        };
        theme.with_accent(brand.accent())
    }

    /// Re-derives every accent-bound role (tone, link, focus ring, selection layer) for `accent`.
    pub const fn with_accent(mut self, accent: Accent) -> Self {
        let tone = Tone::derive(self.polarity, accent);
        self.accent = accent;
        self.status.accent = tone;
        self.text.link = tone.text;
        self.text.on_accent = tone.on_solid;
        self.border.focus = Tone::focus(self.polarity, accent);
        let selected_a = match self.polarity {
            Polarity::Light => 0x24,
            Polarity::Dark => 0x2e,
            Polarity::HighContrastLight => 0x22,
            Polarity::HighContrastDark => 0x38,
        };
        self.state.selected = with_alpha(tone.solid, selected_a);
        self
    }

    pub const fn with_density(mut self, density: UiDensity) -> Self {
        self.density = density;
        self.spacing = Spacing::for_density(density);
        self
    }

    pub const fn with_radius(mut self, radius: Radius) -> Self {
        self.radius = radius;
        self
    }

    /// Maps an island/host [`ThemeContext`] (scheme with OS fallback, contrast, accent RGBA).
    pub const fn from_theme_context(ctx: &ThemeContext, os: ColorScheme, brand: AppBrand) -> Self {
        let scheme = match ctx.color_scheme {
            Some(s) => s,
            None => os,
        };
        let theme = Self::preset(Polarity::new(scheme, ctx.contrast), brand);
        if ctx.accent_rgba == 0 {
            theme
        } else {
            let [r, g, b, _] = ctx.accent_rgba.to_be_bytes();
            theme.with_accent(Accent::from_srgb(ColorRgba::rgb(r, g, b)))
        }
    }

    pub const fn scheme(&self) -> ColorScheme {
        self.polarity.scheme()
    }

    pub const fn contrast(&self) -> ContrastLevel {
        self.polarity.contrast()
    }

    pub const fn is_dark(&self) -> bool {
        self.polarity.is_dark()
    }

    pub const fn is_high_contrast(&self) -> bool {
        self.polarity.is_high_contrast()
    }
}
