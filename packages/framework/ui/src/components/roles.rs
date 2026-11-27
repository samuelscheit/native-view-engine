//! Pure theme → component color pairings. Every `(bg, fg)` a component paints comes from here so
//! `tests/theme_contrast_tests.rs` can check each variant in every polarity, brand, and accent.
//!
//! Rule: a [`Tone`]'s `solid` is only a fill or a non-text indicator; text on it is `on_solid`,
//! and tone-colored text on a surface is `text`.

use nve_abi::ColorRgba;

use crate::components::actions::{BadgeVariant, ButtonVariant};
use crate::theme::{Theme, Tone};

/// Resolved paint of one control state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlColors {
    pub bg: ColorRgba,
    pub fg: ColorRgba,
    pub border_width: f32,
    pub border: ColorRgba,
}

impl ControlColors {
    /// Filled tone: `solid` background with its paired `on_solid` ink.
    pub const fn solid(tone: Tone) -> Self {
        Self { bg: tone.solid, fg: tone.on_solid, border_width: 1.0, border: tone.solid }
    }

    const fn focused(self, theme: &Theme) -> Self {
        Self { border_width: 2.0, border: theme.border.focus, ..self }
    }
}

/// [`crate::Button`] / [`crate::IconButton`] paint.
pub const fn button_colors(theme: &Theme, variant: ButtonVariant, focused: bool, disabled: bool) -> ControlColors {
    let hc = theme.is_high_contrast();
    let filled_border = if hc { theme.border.default } else { ColorRgba::TRANSPARENT };
    let filled_width = if hc { 2.0 } else { 0.0 };
    let base = match variant {
        ButtonVariant::Primary => ControlColors {
            border_width: filled_width,
            border: filled_border,
            ..ControlColors::solid(theme.status.accent)
        },
        ButtonVariant::Destructive => ControlColors {
            border_width: filled_width,
            border: filled_border,
            ..ControlColors::solid(theme.status.danger)
        },
        ButtonVariant::Secondary => ControlColors {
            bg: theme.surface.subtle,
            fg: theme.text.primary,
            border_width: 1.0,
            border: theme.border.default,
        },
        ButtonVariant::Ghost => ControlColors {
            bg: ColorRgba::TRANSPARENT,
            fg: theme.text.primary,
            border_width: 0.0,
            border: ColorRgba::TRANSPARENT,
        },
        ButtonVariant::Outline => ControlColors {
            bg: theme.surface.base,
            fg: theme.text.primary,
            border_width: if hc { 2.0 } else { 1.0 },
            border: theme.border.default,
        },
    };
    if focused && !disabled { base.focused(theme) } else { base }
}

/// [`crate::Badge`] paint.
pub const fn badge_colors(theme: &Theme, variant: BadgeVariant) -> ControlColors {
    match variant {
        BadgeVariant::Neutral => ControlColors {
            bg: theme.surface.subtle,
            fg: theme.text.secondary,
            border_width: 1.0,
            border: theme.border.default,
        },
        BadgeVariant::Accent => ControlColors::solid(theme.status.accent),
        BadgeVariant::Success => ControlColors::solid(theme.status.success),
        BadgeVariant::Warning => ControlColors::solid(theme.status.warning),
        BadgeVariant::Danger => ControlColors::solid(theme.status.danger),
    }
}

/// [`crate::TabTrigger`] paint (selected tabs sit on `surface.base`).
pub const fn tab_colors(theme: &Theme, selected: bool, focused: bool) -> ControlColors {
    let base = if selected {
        ControlColors {
            bg: theme.surface.base,
            fg: theme.status.accent.text,
            border_width: 1.0,
            border: theme.border.default,
        }
    } else {
        ControlColors {
            bg: ColorRgba::TRANSPARENT,
            fg: if focused { theme.text.primary } else { theme.text.secondary },
            border_width: 0.0,
            border: ColorRgba::TRANSPARENT,
        }
    };
    if focused { base.focused(theme) } else { base }
}

/// Error/validation message text (on any surface).
pub const fn error_text(theme: &Theme) -> ColorRgba {
    theme.status.danger.text
}

/// Selected-option / active-item text inside lists and menus.
pub const fn selected_text(theme: &Theme) -> ColorRgba {
    theme.status.accent.text
}
