//! Pure theme → component color pairings. Every `(bg, fg)` a component paints comes from here so
//! `tests/theme_contrast_tests.rs` can check each variant in every polarity, brand, and accent.
//!
//! Rule: a [`Tone`]'s `solid` is only a fill or a non-text indicator; text on it is `on_solid`,
//! and tone-colored text on a surface is `text`.

use nve_abi::ColorRgba;

use crate::components::actions::{BadgeVariant, ButtonVariant};
use crate::components::toast::ToastVariant;
use crate::theme::{wcag_contrast, Theme, Tone};

/// Resolved paint of one control state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlColors {
    pub bg: ColorRgba,
    pub fg: ColorRgba,
    pub border_width: f32,
    pub border: ColorRgba,
}

impl ControlColors {
    /// Filled tone: `solid` background with its paired `on_solid` ink. Where the fill alone falls
    /// below the non-text bar on any surface (e.g. light accents in Light mode), it gets the
    /// tone's `border` as its boundary.
    pub const fn solid(theme: &Theme, tone: Tone) -> Self {
        let s = theme.surface;
        let min = theme.non_text_min();
        let stands_out = wcag_contrast(tone.solid, s.canvas) >= min
            && wcag_contrast(tone.solid, s.base) >= min
            && wcag_contrast(tone.solid, s.raised) >= min
            && wcag_contrast(tone.solid, s.overlay) >= min;
        let border = if stands_out { tone.solid } else { tone.border };
        Self { bg: tone.solid, fg: tone.on_solid, border_width: 1.0, border }
    }

    /// Filled state indicator (checked box, on switch, selected day): `solid` fill with its
    /// `on_solid` ink, outlined in the tone's `text` so the state clears the non-text bar against
    /// any surface even where `solid` alone does not.
    pub const fn indicator(tone: Tone) -> Self {
        Self { bg: tone.solid, fg: tone.on_solid, border_width: 1.0, border: tone.text }
    }

    const fn focused(self, theme: &Theme) -> Self {
        Self { border_width: 2.0, border: theme.border.focus, ..self }
    }
}

/// [`crate::Button`] / [`crate::IconButton`] paint.
pub const fn button_colors(theme: &Theme, variant: ButtonVariant, focused: bool, disabled: bool) -> ControlColors {
    let hc = theme.is_high_contrast();
    let filled_width = if hc { 2.0 } else { 1.0 };
    let base = match variant {
        ButtonVariant::Primary => ControlColors {
            border_width: filled_width,
            ..ControlColors::solid(theme, theme.status.accent)
        },
        ButtonVariant::Destructive => ControlColors {
            border_width: filled_width,
            ..ControlColors::solid(theme, theme.status.danger)
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
        BadgeVariant::Accent => ControlColors::solid(theme, theme.status.accent),
        BadgeVariant::Success => ControlColors::solid(theme, theme.status.success),
        BadgeVariant::Warning => ControlColors::solid(theme, theme.status.warning),
        BadgeVariant::Danger => ControlColors::solid(theme, theme.status.danger),
    }
}

/// [`crate::TabTrigger`] paint (selected tabs sit on `surface.base`).
pub const fn tab_colors(theme: &Theme, selected: bool, focused: bool) -> ControlColors {
    let base = if selected {
        ControlColors {
            bg: theme.surface.base,
            fg: theme.status.accent.text,
            border_width: 1.0,
            border: accent_indicator(theme),
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

/// [`crate::Checkbox`] box paint; `fg` is the check-mark ink.
pub const fn checkbox_colors(theme: &Theme, active: bool, focused: bool) -> ControlColors {
    let base = if active {
        ControlColors::indicator(theme.status.accent)
    } else {
        ControlColors {
            bg: theme.surface.base,
            fg: theme.text.primary,
            border_width: if theme.is_high_contrast() { 2.0 } else { 1.0 },
            border: theme.border.strong,
        }
    };
    if focused { base.focused(theme) } else { base }
}

/// [`crate::Switch`] track paint and thumb color.
pub const fn switch_colors(theme: &Theme, checked: bool, focused: bool) -> (ControlColors, ColorRgba) {
    let (track, thumb) = if checked {
        (ControlColors::indicator(theme.status.accent), theme.status.accent.on_solid)
    } else {
        let off = ControlColors {
            bg: theme.border.strong,
            fg: theme.text.primary,
            border_width: 1.0,
            border: theme.border.strong,
        };
        (off, theme.surface.base)
    };
    (if focused { track.focused(theme) } else { track }, thumb)
}

/// Calendar day paint: selected / range endpoints are filled indicators, in-range days a subtle
/// band, today a ring.
pub const fn calendar_day_colors(theme: &Theme, endpoint: bool, in_range: bool, today: bool, focused: bool, muted: bool) -> ControlColors {
    let base = if endpoint {
        ControlColors::indicator(theme.status.accent)
    } else {
        ControlColors {
            bg: if in_range { theme.surface.subtle } else { ColorRgba::TRANSPARENT },
            fg: if muted { theme.text.tertiary } else { theme.text.primary },
            border_width: if today { 1.0 } else { 0.0 },
            border: if today { accent_indicator(theme) } else { ColorRgba::TRANSPARENT },
        }
    };
    if focused { base.focused(theme) } else { base }
}

/// [`crate::ToastViewport`] severity tone. The toast's status edge paints the tone's `text`
/// color, which also clears the 3:1 non-text bar on `surface.overlay`.
pub const fn toast_tone(theme: &Theme, variant: ToastVariant) -> Tone {
    match variant {
        ToastVariant::Info => theme.status.info,
        ToastVariant::Success => theme.status.success,
        ToastVariant::Warning => theme.status.warning,
        ToastVariant::Danger => theme.status.danger,
    }
}

/// Non-text accent indicator on a surface (today ring, active rail): the tone's `text` color,
/// since `solid` can fall below 3:1 against light or dark surfaces.
pub const fn accent_indicator(theme: &Theme) -> ColorRgba {
    theme.status.accent.text
}

/// Invalid-field boundary on a surface (non-text, same reasoning as [`accent_indicator`]).
pub const fn error_indicator(theme: &Theme) -> ColorRgba {
    theme.status.danger.text
}

/// Error/validation message text (on any surface).
pub const fn error_text(theme: &Theme) -> ColorRgba {
    theme.status.danger.text
}

/// Selected-option / active-item text inside lists and menus.
pub const fn selected_text(theme: &Theme) -> ColorRgba {
    theme.status.accent.text
}
