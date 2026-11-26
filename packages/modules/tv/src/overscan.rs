//! Title-safe TV overscan safe-area calculator and typed remote action key mapping.

use crate::spatial_focus::SpatialRect;

/// Cardinal 2D directional focus movement.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusDirection {
    Up = 0,
    Down = 1,
    Left = 2,
    Right = 3,
}

/// High-level TV remote, gamepad, or keyboard action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteAction {
    Move(FocusDirection),
    Select,
    Back,
    PlayPause,
    Menu,
    ContextMenu,
    ChannelUp,
    ChannelDown,
}

impl RemoteAction {
    /// Maps standard W3C / TV remote keydown names to a typed [`RemoteAction`].
    pub fn from_key_name(key: &str) -> Option<Self> {
        match key {
            "ArrowUp" => Some(Self::Move(FocusDirection::Up)),
            "ArrowDown" => Some(Self::Move(FocusDirection::Down)),
            "ArrowLeft" => Some(Self::Move(FocusDirection::Left)),
            "ArrowRight" => Some(Self::Move(FocusDirection::Right)),
            "Enter" | "Select" => Some(Self::Select),
            "Escape" | "Back" | "GoBack" => Some(Self::Back),
            "MediaPlayPause" => Some(Self::PlayPause),
            "ContextMenu" => Some(Self::ContextMenu),
            "Menu" => Some(Self::Menu),
            "ChannelUp" => Some(Self::ChannelUp),
            "ChannelDown" => Some(Self::ChannelDown),
            _ => None,
        }
    }

    /// Canonical W3C / TV remote key name for this remote action.
    #[inline]
    pub const fn key_name(self) -> &'static str {
        match self {
            Self::Move(FocusDirection::Up) => "ArrowUp",
            Self::Move(FocusDirection::Down) => "ArrowDown",
            Self::Move(FocusDirection::Left) => "ArrowLeft",
            Self::Move(FocusDirection::Right) => "ArrowRight",
            Self::Select => "Select",
            Self::Back => "Back",
            Self::PlayPause => "MediaPlayPause",
            Self::Menu => "Menu",
            Self::ContextMenu => "ContextMenu",
            Self::ChannelUp => "ChannelUp",
            Self::ChannelDown => "ChannelDown",
        }
    }
}

/// TV title-safe overscan insets (5% per edge by default: `48dp` horizontal and `27dp` vertical
/// on a standard `960x540dp` / 1080p@2x television viewport, or `96px` / `54px` at 1.0x).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TvOverscanInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl TvOverscanInsets {
    /// Standard 5% title-safe overscan margin per edge (Android TV Leanback & Apple tvOS HIG).
    pub const DEFAULT_OVERSCAN_RATIO: f32 = 0.05;
    /// Upper clamp preventing degenerate zero/inverted safe areas.
    pub const MAX_OVERSCAN_RATIO: f32 = 0.45;

    /// Standard 1080p logical `960x540dp` (2.0x density) 5% overscan insets.
    pub const STANDARD_1080P_DP: Self = Self {
        top: 27.0,
        right: 48.0,
        bottom: 27.0,
        left: 48.0,
    };

    /// Standard 1080p raw-point `1920x1080` (tvOS 1.0x logical coordinates) 5% overscan insets.
    pub const STANDARD_TVOS_1080P: Self = Self {
        top: 54.0,
        right: 96.0,
        bottom: 54.0,
        left: 96.0,
    };

    /// Computes symmetric overscan insets for a logical viewport size in dp/points.
    pub fn for_viewport(width: f32, height: f32, ratio: f32) -> Self {
        let safe_ratio = if ratio.is_finite() {
            ratio.clamp(0.0, Self::MAX_OVERSCAN_RATIO)
        } else {
            Self::DEFAULT_OVERSCAN_RATIO
        };
        let safe_w = if width.is_finite() && width > 0.0 { width } else { 0.0 };
        let safe_h = if height.is_finite() && height > 0.0 { height } else { 0.0 };
        let horizontal = safe_w * safe_ratio;
        let vertical = safe_h * safe_ratio;
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    /// Computes symmetric overscan insets from physical pixel dimensions and display scale/density.
    pub fn for_viewport_px(width_px: u32, height_px: u32, density: f32, ratio: f32) -> Self {
        let safe_density = if density.is_finite() && density > 0.0 { density } else { 1.0 };
        Self::for_viewport(
            width_px as f32 / safe_density,
            height_px as f32 / safe_density,
            ratio,
        )
    }

    /// Remaining title-safe width inside `viewport_width`.
    #[inline]
    pub fn safe_width(&self, viewport_width: f32) -> f32 {
        if viewport_width.is_finite() {
            (viewport_width - self.left - self.right).max(0.0)
        } else {
            0.0
        }
    }

    /// Remaining title-safe height inside `viewport_height`.
    #[inline]
    pub fn safe_height(&self, viewport_height: f32) -> f32 {
        if viewport_height.is_finite() {
            (viewport_height - self.top - self.bottom).max(0.0)
        } else {
            0.0
        }
    }

    /// Title-safe inner rectangle inside a viewport rooted at `(0, 0)`.
    #[inline]
    pub fn safe_rect(&self, viewport_width: f32, viewport_height: f32) -> SpatialRect {
        SpatialRect::new(
            self.left,
            self.top,
            self.safe_width(viewport_width),
            self.safe_height(viewport_height),
        )
    }
}
