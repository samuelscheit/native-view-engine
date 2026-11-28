//! Studio bindings onto the unified [`nve_ui::Theme`]: [`AppBrand::Studio`] preset following the
//! OS appearance, the per-workspace accent override, density/radius self-edits, layout metrics
//! derived from theme spacing, and semantic tones for issue priority/status.

//!
//! Every OKLCH derivation happens at compile time: [`STUDIO_THEMES`], [`XRAY`], and the author
//! badge tones are const tables, so render paths only index.

use nve_abi::{ColorRgba, xxh64};
use nve_ui::{Accent, AppBrand, Polarity, Radius, Spacing, Theme, Tone};

/// The OS appearance [`studio_theme`] resolves against (re-exported for hosts).
pub use nve_ui::Appearance;

use crate::state::{AccentTheme, DensityMode, StudioConfig, TaskStatus};
use crate::traits::studio_density_to_trait;

/// Dense index of a polarity into the per-polarity tables (order of [`Polarity::ALL`]).
pub const fn polarity_index(polarity: Polarity) -> usize {
    match polarity {
        Polarity::Light => 0,
        Polarity::Dark => 1,
        Polarity::HighContrastLight => 2,
        Polarity::HighContrastDark => 3,
    }
}

/// Studio base themes per workspace accent ([`AccentTheme::ALL`] order) × polarity.
pub const STUDIO_THEMES: [[Theme; 4]; 4] = {
    let mut out = [[Theme::preset(Polarity::Light, AppBrand::Studio); 4]; 4];
    let mut a = 0;
    while a < 4 {
        let mut p = 0;
        while p < 4 {
            out[a][p] = Theme::preset(Polarity::ALL[p], AppBrand::Studio)
                .with_accent(AccentTheme::ALL[a].accent());
            p += 1;
        }
        a += 1;
    }
    out
};

/// X-Ray inspector hues per structural tier: fixed hues (independent of the workspace accent)
/// so nesting stays legible, derived per polarity so they stay ≥3:1 on every surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XRayPalette {
    pub panel: ColorRgba,
    pub card: ColorRgba,
    pub row: ColorRgba,
}

/// [`XRayPalette`] per polarity.
pub const XRAY: [XRayPalette; 4] = {
    let mut out = [XRayPalette {
        panel: ColorRgba::TRANSPARENT,
        card: ColorRgba::TRANSPARENT,
        row: ColorRgba::TRANSPARENT,
    }; 4];
    let mut p = 0;
    while p < 4 {
        let polarity = Polarity::ALL[p];
        out[p] = XRayPalette {
            panel: Tone::derive(polarity, Accent::CYAN).text,
            card: Tone::derive(polarity, Accent::VIOLET).text,
            row: Tone::derive(polarity, Accent::GREEN).text,
        };
        p += 1;
    }
    out
};

impl XRayPalette {
    pub const fn for_theme(theme: &Theme) -> Self {
        XRAY[polarity_index(theme.polarity)]
    }
}

/// Corner radii for Studio's self-edited [`crate::state::CornerRadiusMode`].
pub const fn studio_radius(studio: &StudioConfig) -> Radius {
    let r = studio.radius.px() as f32;
    let pill = (r / 2.0).clamp(4.0, 12.0);
    Radius {
        sm: r.min(4.0),
        md: r.min(12.0),
        lg: r,
        xl: r + 4.0,
        full: pill,
    }
}

/// Resolves Studio's theme (OS appearance + workspace accent from [`STUDIO_THEMES`], then the
/// density and radius self-edits): a table lookup, no color math.
pub const fn studio_theme(studio: &StudioConfig, accent: AccentTheme, os: Appearance) -> Theme {
    STUDIO_THEMES[accent.index()][polarity_index(Polarity::new(os.scheme, os.contrast))]
        .with_density(studio_density_to_trait(studio.density))
        .with_radius(studio_radius(studio))
}

/// Pixel layout rhythm for Studio panes, derived from theme [`Spacing`] and [`studio_radius`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutMetrics {
    pub main_pad: u32,
    pub main_gap: u32,
    pub card_pad: u32,
    pub card_gap: u32,
    pub list_gap: u32,
    pub card_radius: u32,
    pub pill_radius: u32,
    pub button_radius: u32,
}

impl LayoutMetrics {
    pub fn from_studio(studio: &StudioConfig) -> Self {
        let s = Spacing::for_density(studio_density_to_trait(studio.density));
        let r = studio_radius(studio);
        let px = |v: f32| v.round() as u32;
        Self {
            main_pad: px(s.md),
            main_gap: px(s.md),
            card_pad: px(s.md),
            card_gap: px(s.sm),
            list_gap: px(s.sm),
            card_radius: px(r.lg),
            pill_radius: px(r.full),
            button_radius: px(r.md),
        }
    }

    pub const fn is_compact(studio: &StudioConfig) -> bool {
        matches!(studio.density, DensityMode::Compact)
    }
}

/// Conservative SemiBold-safe `min-width` for a compact badge label (Phase-0 estimate until
/// Track 2 adds intrinsic text measurement).
pub fn badge_min_w(label: &str) -> u32 {
    let chars = label.chars().count() as u32;
    chars.saturating_mul(6).saturating_add(16).max(24)
}

/// Hues of chat author badges.
pub const AUTHOR_ACCENTS: [Accent; 6] = [
    Accent::CYAN,
    Accent::VIOLET,
    Accent::GREEN,
    Accent::AMBER,
    Accent::BLUE,
    Accent::RED,
];

const AUTHOR_TONES: [[Tone; 6]; 4] = {
    let mut out = [[Tone::derive(Polarity::Light, Accent::CYAN); 6]; 4];
    let mut p = 0;
    while p < 4 {
        let mut a = 0;
        while a < 6 {
            out[p][a] = Tone::derive(Polarity::ALL[p], AUTHOR_ACCENTS[a]);
            a += 1;
        }
        p += 1;
    }
    out
};

/// Badge hue slot of a chat author: derived from the CRDT-synced author name itself, so every
/// peer paints an author identically without syncing any color.
pub const fn author_slot(author: &str) -> usize {
    (xxh64(author.as_bytes(), 0) % AUTHOR_ACCENTS.len() as u64) as usize
}

/// Tone of a chat author's badge in the active polarity.
pub const fn author_tone(theme: &Theme, author: &str) -> Tone {
    AUTHOR_TONES[polarity_index(theme.polarity)][author_slot(author)]
}

pub const fn task_status_tone(theme: &Theme, status: TaskStatus) -> Tone {
    match status {
        TaskStatus::Todo => theme.neutral_tone(),
        TaskStatus::InProgress => theme.status.accent,
        TaskStatus::Done => theme.status.success,
    }
}
