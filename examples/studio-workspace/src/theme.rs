//! Dark slate design tokens, density metrics, and X-Ray HUD border helpers.
//!
//! Every dynamic style string explicitly writes every property that may change
//! (such as `border-width`, `border-color`, `background-color`, `border-radius`)
//! so incremental `CommandBuffer` style updates cleanly reset prior values.

use crate::state::{DensityMode, StudioConfig};

pub const CANVAS_BG: &str = "#0b1120";
pub const PANEL_BG: &str = "#111827";
pub const CARD_BG: &str = "#1e293b";
pub const CARD_ALT_BG: &str = "#172033";
pub const CONTROL_BG: &str = "#1f2937";
pub const BORDER_SUBTLE: &str = "#1e293b";
pub const BORDER_DEFAULT: &str = "#334155";
pub const TEXT_PRIMARY: &str = "#f8fafc";
pub const TEXT_SECONDARY: &str = "#cbd5e1";
pub const TEXT_MUTED: &str = "#94a3b8";

/// X-Ray HUD high-contrast diagnostic border colors by structural tier.
pub const XRAY_PANEL: &str = "#38bdf8";
pub const XRAY_CARD: &str = "#a78bfa";
pub const XRAY_ROW: &str = "#22c55e";

#[derive(Debug, Clone, Copy)]
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
        let r = studio.radius.px();
        let pill_radius = (r / 2).clamp(4, 12);
        let button_radius = r.min(12);
        match studio.density {
            DensityMode::Normal => Self {
                main_pad: 14,
                main_gap: 10,
                card_pad: 11,
                card_gap: 6,
                list_gap: 8,
                card_radius: r,
                pill_radius,
                button_radius,
            },
            DensityMode::Compact => Self {
                main_pad: 10,
                main_gap: 7,
                card_pad: 8,
                card_gap: 4,
                list_gap: 6,
                card_radius: r,
                pill_radius,
                button_radius,
            },
        }
    }
}

/// Returns `(border_width, border_color)` for a top-level panel (sidebar, workspace, studio).
pub fn panel_border(xray: bool) -> (u32, &'static str) {
    if xray {
        (2, XRAY_PANEL)
    } else {
        (1, BORDER_SUBTLE)
    }
}

/// Returns `(border_width, border_color)` for a card surface.
pub fn card_border(xray: bool) -> (u32, &'static str) {
    if xray {
        (2, XRAY_CARD)
    } else {
        (1, BORDER_DEFAULT)
    }
}

/// Returns `(border_width, border_color)` for an inner row/header strip.
pub fn row_border(xray: bool) -> (u32, &'static str) {
    if xray {
        (1, XRAY_ROW)
    } else {
        (0, "transparent")
    }
}

/// Returns `(border_width, border_color)` for an interactive button/pill.
pub fn control_border(xray: bool, active: bool, accent_hex: &'static str) -> (u32, &'static str) {
    if xray {
        (1, XRAY_ROW)
    } else if active {
        (1, accent_hex)
    } else {
        (1, BORDER_DEFAULT)
    }
}
