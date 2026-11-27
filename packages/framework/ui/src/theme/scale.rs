//! Compile-time 12-step Radix-role scale generator driven by OKLCH lightness profiles.
//!
//! Step roles follow Radix Colors: 1–2 backgrounds, 3–5 component fills (rest / hover / active),
//! 6–8 borders (subtle / default / strong), 9–10 solid fills, 11–12 text (low / high contrast).
//! One neutral hue plus per-[`Accent`] hue/chroma feed the same per-mode profile, so every
//! preset (Light / Dark / High-Contrast × brand) derives from one base scale.

use nve_abi::{ColorRgba, ColorScheme, ContrastLevel};

use super::lch::{best_foreground, Oklch};

/// One of the four supported appearance modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Polarity {
    Light,
    Dark,
    HighContrastLight,
    HighContrastDark,
}

impl Polarity {
    pub const ALL: [Self; 4] = [
        Self::Light,
        Self::Dark,
        Self::HighContrastLight,
        Self::HighContrastDark,
    ];

    pub const fn new(scheme: ColorScheme, contrast: ContrastLevel) -> Self {
        match (scheme, contrast) {
            (ColorScheme::Light, ContrastLevel::Standard) => Self::Light,
            (ColorScheme::Dark, ContrastLevel::Standard) => Self::Dark,
            (ColorScheme::Light, ContrastLevel::High) => Self::HighContrastLight,
            (ColorScheme::Dark, ContrastLevel::High) => Self::HighContrastDark,
        }
    }

    pub const fn scheme(self) -> ColorScheme {
        match self {
            Self::Light | Self::HighContrastLight => ColorScheme::Light,
            Self::Dark | Self::HighContrastDark => ColorScheme::Dark,
        }
    }

    pub const fn contrast(self) -> ContrastLevel {
        match self {
            Self::Light | Self::Dark => ContrastLevel::Standard,
            Self::HighContrastLight | Self::HighContrastDark => ContrastLevel::High,
        }
    }

    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark | Self::HighContrastDark)
    }

    pub const fn is_high_contrast(self) -> bool {
        matches!(self, Self::HighContrastLight | Self::HighContrastDark)
    }

    pub(crate) const fn profile(self) -> &'static ModeProfile {
        match self {
            Self::Light => &LIGHT,
            Self::Dark => &DARK,
            Self::HighContrastLight => &HC_LIGHT,
            Self::HighContrastDark => &HC_DARK,
        }
    }
}

/// A brand / status hue: OKLCH hue + peak chroma + lightness of its solid (step 9) fill.
///
/// `solid_l` must sit outside the `0.54..0.71` band where neither white nor near-black text
/// reaches WCAG AA; [`Accent::new`] snaps it out of that band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    pub hue: f32,
    pub chroma: f32,
    pub solid_l: f32,
}

impl Accent {
    pub const BLUE: Self = Self::new(259.0, 0.21, 0.53);
    pub const INDIGO: Self = Self::new(277.0, 0.2, 0.51);
    pub const CYAN: Self = Self::new(225.0, 0.14, 0.75);
    pub const VIOLET: Self = Self::new(293.0, 0.17, 0.76);
    pub const GREEN: Self = Self::new(152.0, 0.17, 0.75);
    pub const AMBER: Self = Self::new(75.0, 0.16, 0.8);
    pub const RED: Self = Self::new(25.0, 0.2, 0.53);

    pub const fn new(hue: f32, chroma: f32, solid_l: f32) -> Self {
        let solid_l = if solid_l > 0.54 && solid_l < 0.625 {
            0.53
        } else if solid_l >= 0.625 && solid_l < 0.71 {
            0.71
        } else {
            solid_l
        };
        Self { hue, chroma, solid_l }
    }

    /// Derives an accent from an arbitrary sRGB brand color (e.g. `ThemeContext::accent_rgba`).
    pub const fn from_srgb(color: ColorRgba) -> Self {
        let lch = Oklch::from_srgb(color);
        Self::new(lch.h, lch.c, lch.l)
    }
}

/// Per-mode lightness profile shared by every hue.
pub(crate) struct ModeProfile {
    /// Neutral 12-step lightness.
    pub neutral_l: [f32; 12],
    /// Neutral chroma per step (graphite with a faint cool tint).
    pub neutral_c: [f32; 12],
    /// Tertiary text lightness (between neutral steps 11 and 12 in contrast).
    pub tertiary_l: f32,
    /// Tone roles: `(lightness, chroma factor)` for subtle fill, border, text, and focus ring.
    pub tone_subtle: (f32, f32),
    pub tone_border: (f32, f32),
    pub tone_text: (f32, f32),
    pub tone_focus: (f32, f32),
    /// Solid lightness override (high-contrast modes force maximum-contrast solids).
    pub solid_l: Option<f32>,
    /// Syntax token `(lightness, chroma)`.
    pub syntax: (f32, f32),
}

pub(crate) const NEUTRAL_HUE: f32 = 286.0;

/// Near-black graphite ink for light solids (dark neutral step 1, `#09090b`).
const ON_SOLID_INK: ColorRgba = Oklch::new(0.141, 0.005, NEUTRAL_HUE).to_srgb();

const LIGHT: ModeProfile = ModeProfile {
    neutral_l: [0.975, 0.962, 0.945, 0.928, 0.91, 0.89, 0.85, 0.62, 0.552, 0.51, 0.43, 0.21],
    neutral_c: [0.002, 0.003, 0.004, 0.004, 0.005, 0.005, 0.006, 0.012, 0.014, 0.014, 0.016, 0.006],
    tertiary_l: 0.52,
    tone_subtle: (0.955, 0.16),
    tone_border: (0.82, 0.45),
    tone_text: (0.47, 0.85),
    tone_focus: (0.55, 1.0),
    solid_l: None,
    syntax: (0.5, 0.15),
};

const DARK: ModeProfile = ModeProfile {
    neutral_l: [0.141, 0.17, 0.21, 0.235, 0.262, 0.29, 0.34, 0.53, 0.552, 0.6, 0.845, 0.97],
    neutral_c: [0.005, 0.005, 0.006, 0.006, 0.007, 0.008, 0.009, 0.014, 0.014, 0.014, 0.01, 0.003],
    tertiary_l: 0.745,
    tone_subtle: (0.25, 0.3),
    tone_border: (0.42, 0.55),
    tone_text: (0.81, 0.7),
    tone_focus: (0.74, 1.0),
    solid_l: None,
    syntax: (0.8, 0.13),
};

const HC_LIGHT: ModeProfile = ModeProfile {
    neutral_l: [1.0, 0.955, 0.93, 0.9, 0.87, 0.6, 0.45, 0.3, 0.4, 0.33, 0.25, 0.0],
    neutral_c: [0.0, 0.002, 0.003, 0.004, 0.005, 0.008, 0.01, 0.01, 0.012, 0.012, 0.01, 0.0],
    tertiary_l: 0.36,
    tone_subtle: (0.93, 0.22),
    tone_border: (0.42, 0.8),
    tone_text: (0.36, 0.9),
    tone_focus: (0.4, 1.0),
    solid_l: Some(0.42),
    syntax: (0.38, 0.16),
};

const HC_DARK: ModeProfile = ModeProfile {
    neutral_l: [0.0, 0.13, 0.17, 0.2, 0.23, 0.6, 0.76, 0.88, 0.8, 0.86, 0.93, 1.0],
    neutral_c: [0.0, 0.003, 0.004, 0.005, 0.006, 0.008, 0.008, 0.006, 0.01, 0.01, 0.004, 0.0],
    tertiary_l: 0.86,
    tone_subtle: (0.22, 0.38),
    tone_border: (0.8, 0.7),
    tone_text: (0.88, 0.6),
    tone_focus: (0.85, 0.8),
    solid_l: Some(0.84),
    syntax: (0.88, 0.12),
};

/// A generated 12-step scale (`steps[0]` is Radix step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub steps: [ColorRgba; 12],
}

impl Scale {
    /// Radix step `1..=12`.
    pub const fn step(&self, n: usize) -> ColorRgba {
        self.steps[n - 1]
    }

    /// The neutral graphite scale for `polarity`.
    pub const fn neutral(polarity: Polarity) -> Self {
        let p = polarity.profile();
        let mut steps = [ColorRgba::TRANSPARENT; 12];
        let mut i = 0;
        while i < 12 {
            steps[i] = Oklch::new(p.neutral_l[i], p.neutral_c[i], NEUTRAL_HUE).to_srgb();
            i += 1;
        }
        Self { steps }
    }
}

const fn solid_lightness(polarity: Polarity, accent: Accent) -> f32 {
    match polarity.profile().solid_l {
        Some(l) => l,
        None => accent.solid_l,
    }
}

/// Semantic roles derived from one hue: solid fill, its foreground, tinted background, border,
/// and readable tone text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tone {
    pub solid: ColorRgba,
    pub on_solid: ColorRgba,
    pub subtle: ColorRgba,
    pub border: ColorRgba,
    pub text: ColorRgba,
}

impl Tone {
    pub const fn derive(polarity: Polarity, accent: Accent) -> Self {
        let p = polarity.profile();
        const fn at(role: (f32, f32), accent: Accent) -> ColorRgba {
            Oklch::new(role.0, accent.chroma * role.1, accent.hue).to_srgb()
        }
        let solid = Oklch::new(solid_lightness(polarity, accent), accent.chroma, accent.hue).to_srgb();
        Self {
            solid,
            on_solid: best_foreground(solid, ColorRgba::WHITE, ON_SOLID_INK),
            subtle: at(p.tone_subtle, accent),
            border: at(p.tone_border, accent),
            text: at(p.tone_text, accent),
        }
    }

    /// Focus-ring color for this hue (WCAG 2.4.11 ≥ 3:1 against every surface).
    pub const fn focus(polarity: Polarity, accent: Accent) -> ColorRgba {
        let role = polarity.profile().tone_focus;
        Oklch::new(role.0, accent.chroma * role.1, accent.hue).to_srgb()
    }
}
