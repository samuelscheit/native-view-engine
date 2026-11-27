//! Studio bindings onto the unified [`nve_ui::Theme`]: [`AppBrand::Studio`] preset following the
//! OS appearance, the per-workspace accent override, density/radius self-edits, layout metrics
//! derived from theme spacing, and semantic tones for issue priority/status.

use nve_abi::ColorRgba;
use nve_ui::{parse_color_rgba, Accent, AppBrand, Appearance, Radius, Spacing, Theme, ThemeConfig, Tone};

use crate::state::{AccentTheme, DensityMode, StudioConfig, TaskStatus};
use crate::traits::studio_density_to_trait;

/// X-Ray inspector hues per structural tier: fixed hues (independent of the workspace accent)
/// so nesting stays legible, derived per polarity so they stay ≥3:1 on every surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XRayPalette {
    pub panel: ColorRgba,
    pub card: ColorRgba,
    pub row: ColorRgba,
}

impl XRayPalette {
    pub const fn new(theme: &Theme) -> Self {
        Self {
            panel: Tone::derive(theme.polarity, Accent::CYAN).text,
            card: Tone::derive(theme.polarity, Accent::VIOLET).text,
            row: Tone::derive(theme.polarity, Accent::GREEN).text,
        }
    }
}

/// Corner radii for Studio's self-edited [`crate::state::CornerRadiusMode`].
pub const fn studio_radius(studio: &StudioConfig) -> Radius {
    let r = studio.radius.px() as f32;
    let pill = (r / 2.0).clamp(4.0, 12.0);
    Radius { sm: r.min(4.0), md: r.min(12.0), lg: r, xl: r + 4.0, full: pill }
}

/// Resolves Studio's theme: OS appearance + workspace accent + density + radius.
pub fn studio_theme(studio: &StudioConfig, accent: AccentTheme, os: Appearance) -> Theme {
    ThemeConfig {
        accent: Some(accent.accent()),
        density: studio_density_to_trait(studio.density),
        ..ThemeConfig::new(AppBrand::Studio)
    }
    .resolve(os)
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

/// Flex override applied to `nve-ui` `Button`, `TabTrigger`, and `Badge` containers so their
/// centered label spans the control without wrapping (until Track 2 ships intrinsic text sizing).
pub const STRETCH_COL: &str = "flex-direction: column; justify-content: center; align-items: stretch;";

/// Conservative SemiBold-safe `min-width` for a compact badge label (Phase-0 estimate until
/// Track 2 adds intrinsic text measurement).
pub fn badge_min_w(label: &str) -> u32 {
    let chars = label.chars().count() as u32;
    chars.saturating_mul(6).saturating_add(16).max(24)
}

/// Neutral tone for idle/unclassified items.
const fn neutral_tone(theme: &Theme) -> Tone {
    Tone {
        solid: theme.border.strong,
        on_solid: theme.text.primary,
        subtle: theme.surface.subtle,
        border: theme.border.strong,
        text: theme.text.secondary,
    }
}

/// Tone of a chat author's badge. The CRDT-synced `#rrggbb` author color is only a hue seed:
/// the readable pair is re-derived for the active polarity (synced literal pairs fail HC).
pub fn author_tone(theme: &Theme, synced_color: &str) -> Tone {
    parse_color_rgba(synced_color)
        .map_or_else(|| neutral_tone(theme), |c| Tone::derive(theme.polarity, Accent::from_srgb(c)))
}

pub const fn task_status_tone(theme: &Theme, status: TaskStatus) -> Tone {
    match status {
        TaskStatus::Todo => neutral_tone(theme),
        TaskStatus::InProgress => theme.status.accent,
        TaskStatus::Done => theme.status.success,
    }
}
