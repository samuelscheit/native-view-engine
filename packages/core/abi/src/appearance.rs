//! Canonical cross-crate appearance primitives (`ColorScheme` and `ContrastLevel`).
//!
//! All core, backend, framework, module, and host crates share these single canonical
//! `#[repr(u8)]` types. Non-zero discriminants on [`ColorScheme`] enable niche-optimized
//! `Option<ColorScheme>` (`size_of == 1`, `None == 0`, `Some(Light) == 1`, `Some(Dark) == 2`)
//! so "follow system / unspecified" is represented explicitly and never silently coerced to light.

use core::mem::{align_of, size_of};

/// Canonical light/dark color scheme (`Light = 1`, `Dark = 2`).
///
/// Use `Option<ColorScheme>` where "unspecified / follow system" is meaningful (`0` on the wire).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ColorScheme {
    Light = 1,
    Dark = 2,
}

impl ColorScheme {
    /// Decodes a canonical non-zero byte (`1 => Light`, `2 => Dark`).
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Light),
            2 => Some(Self::Dark),
            _ => None,
        }
    }

    /// Encodes `Option<ColorScheme>` into its canonical ABI byte (`0 = None`, `1 = Light`, `2 = Dark`).
    #[inline]
    pub const fn opt_to_u8(scheme: Option<Self>) -> u8 {
        match scheme {
            None => 0,
            Some(s) => s as u8,
        }
    }

    /// Decodes a canonical ABI byte (`0 => Ok(None)`, `1 => Ok(Some(Light))`, `2 => Ok(Some(Dark))`).
    #[inline]
    pub const fn opt_from_u8(raw: u8) -> Result<Option<Self>, u8> {
        match raw {
            0 => Ok(None),
            1 => Ok(Some(Self::Light)),
            2 => Ok(Some(Self::Dark)),
            other => Err(other),
        }
    }

    #[inline]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }

    #[inline]
    pub const fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }
}

/// Canonical display contrast accessibility preference (`Standard = 0`, `High = 1`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ContrastLevel {
    #[default]
    Standard = 0,
    High = 1,
}

impl ContrastLevel {
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Standard),
            1 => Some(Self::High),
            _ => None,
        }
    }

    #[inline]
    pub const fn is_high(self) -> bool {
        matches!(self, Self::High)
    }
}

const _: () = {
    assert!(size_of::<ColorScheme>() == 1);
    assert!(align_of::<ColorScheme>() == 1);
    assert!(size_of::<Option<ColorScheme>>() == 1);
    assert!(align_of::<Option<ColorScheme>>() == 1);
    assert!(size_of::<ContrastLevel>() == 1);
    assert!(align_of::<ContrastLevel>() == 1);
};
