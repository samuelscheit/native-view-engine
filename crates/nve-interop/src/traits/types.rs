//! Built-in environment value types (`PlatformOs`, `HostToolkit`, `SizeClass`, `ContainerSize`,
//! `Orientation`, `PointerKind`, `ColorScheme`, `UiDensity`, `Locale`) and primitive marker keys.

use std::fmt;

pub use crate::surface::SafeAreaInsets;

#[inline]
pub(crate) fn f32_changed(old: f32, new: f32) -> bool {
    if old.is_nan() && new.is_nan() {
        false
    } else {
        old != new
    }
}

/// Operating system running the active NVE host.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PlatformOs {
    MacOs = 0,
    Ios = 1,
    Windows = 2,
    Android = 3,
    Linux = 4,
    Web = 5,
    #[default]
    Unknown = 6,
}

impl PlatformOs {
    /// Detects the compile-time target OS.
    pub const fn current() -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self::Web
        }
        #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
        {
            Self::MacOs
        }
        #[cfg(all(not(target_arch = "wasm32"), target_os = "ios"))]
        {
            Self::Ios
        }
        #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
        {
            Self::Windows
        }
        #[cfg(all(not(target_arch = "wasm32"), target_os = "android"))]
        {
            Self::Android
        }
        #[cfg(all(not(target_arch = "wasm32"), target_os = "linux"))]
        {
            Self::Linux
        }
        #[cfg(not(any(
            target_arch = "wasm32",
            target_os = "macos",
            target_os = "ios",
            target_os = "windows",
            target_os = "android",
            target_os = "linux"
        )))]
        {
            Self::Unknown
        }
    }

    #[inline]
    pub const fn is_apple(self) -> bool {
        matches!(self, Self::MacOs | Self::Ios)
    }

    #[inline]
    pub const fn is_mobile(self) -> bool {
        matches!(self, Self::Ios | Self::Android)
    }

    #[inline]
    pub const fn is_desktop(self) -> bool {
        matches!(self, Self::MacOs | Self::Windows | Self::Linux)
    }
}

/// Native rendering or embedding host toolkit backing the surface.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HostToolkit {
    Skia = 0,
    AppKit = 1,
    UiKit = 2,
    Win32 = 3,
    WinUiFabric = 4,
    AndroidCompose = 5,
    AndroidViews = 6,
    Qt = 7,
    Gtk = 8,
    SwiftUi = 9,
    Flutter = 10,
    WebDom = 11,
    #[default]
    Headless = 12,
}

/// Responsive layout breakpoint class (horizontal or vertical).
///
/// Breakpoints:
/// - `< 600` logical px => [`SizeClass::Compact`]
/// - `600..1024` logical px => [`SizeClass::Medium`]
/// - `>= 1024` logical px => [`SizeClass::Expanded`]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum SizeClass {
    #[default]
    Compact = 0,
    Medium = 1,
    Expanded = 2,
}

impl SizeClass {
    /// Width threshold separating [`SizeClass::Compact`] and [`SizeClass::Medium`].
    pub const MEDIUM_MIN: f32 = 600.0;
    /// Width threshold separating [`SizeClass::Medium`] and [`SizeClass::Expanded`].
    pub const EXPANDED_MIN: f32 = 1024.0;

    /// Computes the discrete [`SizeClass`] for a logical width (or extent).
    #[inline]
    pub fn from_width(width: f32) -> Self {
        if !width.is_finite() || width < Self::MEDIUM_MIN {
            Self::Compact
        } else if width < Self::EXPANDED_MIN {
            Self::Medium
        } else {
            Self::Expanded
        }
    }

    /// Computes the discrete [`SizeClass`] for a logical height.
    #[inline]
    pub fn from_height(height: f32) -> Self {
        Self::from_width(height)
    }
}

/// Scoped `@container`-style dimensions and derived [`SizeClass`] for Skia/Fabric/Web subtrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContainerSize {
    pub width: f32,
    pub height: f32,
    pub class: SizeClass,
}

impl ContainerSize {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
        class: SizeClass::Compact,
    };

    #[inline]
    pub fn new(width: f32, height: f32) -> Self {
        let w = if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        };
        let h = if height.is_finite() {
            height.max(0.0)
        } else {
            0.0
        };
        Self {
            width: w,
            height: h,
            class: SizeClass::from_width(w),
        }
    }
}

impl Default for ContainerSize {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Viewport orientation derived from width vs height.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Orientation {
    #[default]
    Portrait = 0,
    Landscape = 1,
}

impl Orientation {
    #[inline]
    pub fn from_dimensions(width: f32, height: f32) -> Self {
        if width >= height {
            Self::Landscape
        } else {
            Self::Portrait
        }
    }
}

/// Primary pointing device precision (`@media (pointer: fine | coarse | none)`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerKind {
    #[default]
    Fine = 0,
    Coarse = 1,
    None = 2,
}

/// Binary light/dark appearance scheme for the trait environment.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorScheme {
    #[default]
    Light = 0,
    Dark = 1,
}

impl ColorScheme {
    #[inline]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

impl From<crate::surface::ColorScheme> for ColorScheme {
    #[inline]
    fn from(value: crate::surface::ColorScheme) -> Self {
        match value {
            crate::surface::ColorScheme::Dark | crate::surface::ColorScheme::HighContrastDark => {
                Self::Dark
            }
            crate::surface::ColorScheme::Light
            | crate::surface::ColorScheme::HighContrastLight
            | crate::surface::ColorScheme::System => Self::Light,
        }
    }
}

impl From<ColorScheme> for crate::surface::ColorScheme {
    #[inline]
    fn from(value: ColorScheme) -> Self {
        match value {
            ColorScheme::Light => Self::Light,
            ColorScheme::Dark => Self::Dark,
        }
    }
}

/// Visual spacing/control density preference.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UiDensity {
    Compact = 0,
    #[default]
    Comfortable = 1,
    Spacious = 2,
}

/// Inline zero-allocation BCP-47 language tag (up to 23 UTF-8 bytes) with RTL detection.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Locale {
    bytes: [u8; 23],
    len: u8,
}

impl Locale {
    /// Default US English locale (`"en-US"`).
    pub const EN_US: Self = Self::from_ascii_bytes(b"en-US");

    const fn from_ascii_bytes(src: &[u8]) -> Self {
        let mut bytes = [0u8; 23];
        let max = if src.len() < 23 { src.len() } else { 23 };
        let mut i = 0;
        while i < max {
            bytes[i] = src[i];
            i += 1;
        }
        Self {
            bytes,
            len: max as u8,
        }
    }

    /// Constructs a compact [`Locale`] from a BCP-47 tag (defaults to `"en-US"` if blank).
    pub fn new(tag: &str) -> Self {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            return Self::EN_US;
        }
        let mut end = trimmed.len().min(23);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0u8; 23];
        bytes[..end].copy_from_slice(&trimmed.as_bytes()[..end]);
        Self {
            bytes,
            len: end as u8,
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("en-US")
    }

    /// Returns the primary language subtag (e.g. `"ar"` from `"ar-SA"` or `"en"` from `"en_US"`).
    #[inline]
    pub fn primary_language(&self) -> &str {
        let s = self.as_str();
        match s.find(['-', '_']) {
            Some(idx) => &s[..idx],
            None => s,
        }
    }

    /// Returns `true` if the locale's primary script/language is right-to-left (RTL).
    #[inline]
    pub fn is_rtl(&self) -> bool {
        let lang = self.primary_language();
        lang.eq_ignore_ascii_case("ar")
            || lang.eq_ignore_ascii_case("he")
            || lang.eq_ignore_ascii_case("iw")
            || lang.eq_ignore_ascii_case("fa")
            || lang.eq_ignore_ascii_case("ur")
            || lang.eq_ignore_ascii_case("ps")
            || lang.eq_ignore_ascii_case("sd")
            || lang.eq_ignore_ascii_case("ug")
            || lang.eq_ignore_ascii_case("yi")
            || lang.eq_ignore_ascii_case("dv")
            || lang.eq_ignore_ascii_case("ku")
            || lang.eq_ignore_ascii_case("ckb")
            || lang.eq_ignore_ascii_case("syr")
            || lang.eq_ignore_ascii_case("nqo")
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::EN_US
    }
}

impl fmt::Debug for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Locale").field(&self.as_str()).finish()
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Locale {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Locale {
    fn from(value: String) -> Self {
        Self::new(&value)
    }
}

// =============================================================================
// Marker TraitKeys for Primitive-Valued Built-In Traits
// =============================================================================

/// Vertical responsive [`SizeClass`] key (`< 600` Compact, `600..1024` Medium, `>= 1024` Expanded).
pub struct VerticalSizeClass;
/// Display scale factor (`f32`, e.g. `1.0`, `2.0`, `3.0`).
pub struct ScaleFactor;
/// Whether the primary pointer supports hover interactions (`bool`).
pub struct CanHover;
/// Whether a hardware keyboard is attached (`bool`).
pub struct HasHardwareKeyboard;
/// Whether the user prefers reduced motion (`bool`).
pub struct ReducedMotion;
/// Whether high-contrast accessibility mode is active (`bool`).
pub struct HighContrast;
/// Dynamic typography scale factor (`f32`, default `1.0`).
pub struct TextScale;
