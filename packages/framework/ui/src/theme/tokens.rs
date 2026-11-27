//! Semantic token groups: color roles, typography, spacing, radius, elevation, motion, z-layers,
//! and breakpoints. All groups are `Copy` so [`super::Theme`] lookups never allocate.

use nve_abi::ColorRgba;
use nve_interop::traits::{SizeClass, UiDensity};

use super::scale::Tone;

/// Tonal surface stack (Precision Instrument: depth from luminance steps, not shadows).
///
/// Light: tinted `canvas` under crisp white `base`/`raised`/`overlay`. Dark: graphite steps
/// ascending `canvas < base < raised < overlay`. `sunken` is a recessed well (code, inputs);
/// `subtle` is a filled control background (chips, segmented controls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub canvas: ColorRgba,
    pub base: ColorRgba,
    pub raised: ColorRgba,
    pub overlay: ColorRgba,
    pub sunken: ColorRgba,
    pub subtle: ColorRgba,
}

/// Translucent interaction layers composited over any surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateLayer {
    pub hover: ColorRgba,
    pub active: ColorRgba,
    pub selected: ColorRgba,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub primary: ColorRgba,
    pub secondary: ColorRgba,
    pub tertiary: ColorRgba,
    pub link: ColorRgba,
    pub on_accent: ColorRgba,
}

/// `subtle`/`default` are 1px translucent hairlines; `strong` is the ≥ 3:1 control boundary;
/// `focus` is the keyboard focus ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    pub subtle: ColorRgba,
    pub default: ColorRgba,
    pub strong: ColorRgba,
    pub focus: ColorRgba,
}

/// Semantic status tones; each carries `solid`, `on_solid`, `subtle`, `border`, and `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub accent: Tone,
    pub success: Tone,
    pub warning: Tone,
    pub danger: Tone,
    pub info: Tone,
}

/// Code syntax palette shared by docs code blocks and Studio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syntax {
    pub plain: ColorRgba,
    pub keyword: ColorRgba,
    pub type_name: ColorRgba,
    pub string: ColorRgba,
    pub comment: ColorRgba,
    pub number: ColorRgba,
    pub function: ColorRgba,
    pub macro_name: ColorRgba,
    pub attribute: ColorRgba,
    pub punctuation: ColorRgba,
    pub diff_add: ColorRgba,
    pub diff_del: ColorRgba,
}

/// Hybrid font strategy: OS system UI sans + bundled monospace with `ui-monospace` fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontStack {
    UiSans,
    Mono,
}

impl FontStack {
    pub const fn css(self) -> &'static str {
        match self {
            Self::UiSans => {
                "system-ui, -apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", Roboto, \"Helvetica Neue\", sans-serif"
            }
            Self::Mono => {
                "\"JetBrains Mono\", \"Geist Mono\", ui-monospace, \"SF Mono\", Menlo, Consolas, monospace"
            }
        }
    }
}

/// One typographic role: size (px), unitless line height, weight, and tracking (em).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub tracking: f32,
    pub family: FontStack,
}

impl TypeStyle {
    const fn sans(size: f32, line_height: f32, weight: u16, tracking: f32) -> Self {
        Self { size, line_height, weight, tracking, family: FontStack::UiSans }
    }

    /// Absolute line height in px.
    pub const fn line_height_px(&self) -> f32 {
        self.size * self.line_height
    }
}

/// Type roles: dense UI (13/12/11), reading prose (15, docs 16/1.65), and display (48/36/28).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography {
    pub caption: TypeStyle,
    pub label: TypeStyle,
    pub ui: TypeStyle,
    pub body: TypeStyle,
    pub docs_body: TypeStyle,
    pub title: TypeStyle,
    pub heading: TypeStyle,
    pub display_sm: TypeStyle,
    pub display_md: TypeStyle,
    pub display_lg: TypeStyle,
    pub code: TypeStyle,
}

impl Typography {
    pub const DEFAULT: Self = Self {
        caption: TypeStyle::sans(11.0, 1.4, 500, 0.01),
        label: TypeStyle::sans(12.0, 1.35, 500, 0.0),
        ui: TypeStyle::sans(13.0, 1.45, 400, 0.0),
        body: TypeStyle::sans(15.0, 1.55, 400, 0.0),
        docs_body: TypeStyle::sans(16.0, 1.65, 400, 0.0),
        title: TypeStyle::sans(18.0, 1.35, 600, -0.01),
        heading: TypeStyle::sans(24.0, 1.25, 600, -0.015),
        display_sm: TypeStyle::sans(28.0, 1.15, 600, -0.025),
        display_md: TypeStyle::sans(36.0, 1.1, 600, -0.03),
        display_lg: TypeStyle::sans(48.0, 1.08, 600, -0.035),
        code: TypeStyle {
            size: 13.0,
            line_height: 1.6,
            weight: 400,
            tracking: 0.0,
            family: FontStack::Mono,
        },
    };
}

/// Spacing rhythm in logical px, scaled by [`UiDensity`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

impl Spacing {
    pub const fn for_density(density: UiDensity) -> Self {
        match density {
            UiDensity::Compact => Self { xs: 2.0, sm: 4.0, md: 6.0, lg: 10.0, xl: 14.0, xxl: 20.0 },
            UiDensity::Comfortable => Self { xs: 4.0, sm: 8.0, md: 12.0, lg: 16.0, xl: 24.0, xxl: 32.0 },
            UiDensity::Spacious => Self { xs: 6.0, sm: 10.0, md: 16.0, lg: 22.0, xl: 32.0, xxl: 44.0 },
        }
    }
}

/// Corner radii in logical px (concentric: nested radius = outer − padding).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radius {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub full: f32,
}

impl Radius {
    pub const DEFAULT: Self = Self { sm: 4.0, md: 6.0, lg: 10.0, xl: 14.0, full: 999.0 };
}

/// Shadow stack per elevation tier (CSS `box-shadow` until Phase 1 types shadows). Dark and
/// high-contrast modes keep in-flow tiers flat and reserve shadows for floating layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elevation {
    pub raised: &'static str,
    pub overlay: &'static str,
    pub modal: &'static str,
}

impl Elevation {
    pub const LIGHT: Self = Self {
        raised: "0 1px 2px rgba(9, 9, 11, 0.06)",
        overlay: "0 1px 2px rgba(9, 9, 11, 0.06), 0 8px 24px rgba(9, 9, 11, 0.10)",
        modal: "0 2px 4px rgba(9, 9, 11, 0.08), 0 24px 48px rgba(9, 9, 11, 0.16)",
    };
    pub const DARK: Self = Self {
        raised: "none",
        overlay: "0 8px 24px rgba(0, 0, 0, 0.45)",
        modal: "0 24px 64px rgba(0, 0, 0, 0.6)",
    };
    pub const FLAT: Self = Self { raised: "none", overlay: "none", modal: "none" };
}

/// CSS-style cubic Bézier easing control points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier(pub [f32; 4]);

/// Critically-damped-ish spring for direct manipulation (drag release, sheet detents).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

/// Functional motion: 110–150 ms ease-out for state changes, springs only for drags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub fast_ms: u16,
    pub base_ms: u16,
    pub slow_ms: u16,
    pub ease_out: CubicBezier,
    pub ease_in_out: CubicBezier,
    pub drag: Spring,
}

impl Motion {
    pub const DEFAULT: Self = Self {
        fast_ms: 110,
        base_ms: 150,
        slow_ms: 240,
        ease_out: CubicBezier([0.2, 0.0, 0.0, 1.0]),
        ease_in_out: CubicBezier([0.4, 0.0, 0.2, 1.0]),
        drag: Spring { stiffness: 520.0, damping: 42.0, mass: 1.0 },
    };

    /// `prefers-reduced-motion`: durations collapse to zero, springs become critically damped.
    pub const fn reduced(self) -> Self {
        Self {
            fast_ms: 0,
            base_ms: 0,
            slow_ms: 0,
            drag: Spring { stiffness: 1000.0, damping: 63.3, mass: 1.0 },
            ..self
        }
    }
}

/// Stacking layers replacing ad-hoc `z-index` literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Base,
    Sticky,
    Drawer,
    Popover,
    Modal,
    Toast,
    Tooltip,
}

impl Layer {
    pub const fn z(self) -> i32 {
        match self {
            Self::Base => 0,
            Self::Sticky => 100,
            Self::Drawer => 200,
            Self::Popover => 300,
            Self::Modal => 400,
            Self::Toast => 500,
            Self::Tooltip => 600,
        }
    }
}

/// Window-width breakpoints aligned 1:1 with [`SizeClass`] (`< 600`, `600..1024`, `≥ 1024`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Breakpoint {
    Compact,
    Medium,
    Expanded,
}

impl Breakpoint {
    pub const fn min_width(self) -> f32 {
        match self {
            Self::Compact => 0.0,
            Self::Medium => SizeClass::MEDIUM_MIN,
            Self::Expanded => SizeClass::EXPANDED_MIN,
        }
    }

    pub const fn from_width(width: f32) -> Self {
        if width >= SizeClass::EXPANDED_MIN {
            Self::Expanded
        } else if width >= SizeClass::MEDIUM_MIN {
            Self::Medium
        } else {
            Self::Compact
        }
    }
}

impl From<SizeClass> for Breakpoint {
    fn from(size: SizeClass) -> Self {
        match size {
            SizeClass::Compact => Self::Compact,
            SizeClass::Medium => Self::Medium,
            SizeClass::Expanded => Self::Expanded,
        }
    }
}
