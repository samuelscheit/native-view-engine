//! Title-safe TV overscan safe-area calculator and direct binary FFI `NK1` binary keydown codec.

use crate::spatial_focus::{RemoteAction, SpatialRect};

/// Magic prefix bytes (`b"NK1"`) for binary NVE keydown wire payloads.
pub const NK1_MAGIC: [u8; 3] = [0x4E, 0x4B, 0x31];
pub const NK1_HEADER_SIZE: usize = 4;
pub const NK1_MIN_LEN: usize = 5;

pub const NK1_FLAG_META: u8 = 1 << 0;
pub const NK1_FLAG_CTRL: u8 = 1 << 1;
pub const NK1_FLAG_SHIFT: u8 = 1 << 2;
pub const NK1_FLAG_ALT: u8 = 1 << 3;

/// Modifier bitflags for an `NK1` binary keydown payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Nk1Flags {
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Nk1Flags {
    #[inline]
    pub const fn to_byte(self) -> u8 {
        let mut bits = 0u8;
        if self.meta {
            bits |= NK1_FLAG_META;
        }
        if self.ctrl {
            bits |= NK1_FLAG_CTRL;
        }
        if self.shift {
            bits |= NK1_FLAG_SHIFT;
        }
        if self.alt {
            bits |= NK1_FLAG_ALT;
        }
        bits
    }

    #[inline]
    pub const fn from_byte(bits: u8) -> Self {
        Self {
            meta: (bits & NK1_FLAG_META) != 0,
            ctrl: (bits & NK1_FLAG_CTRL) != 0,
            shift: (bits & NK1_FLAG_SHIFT) != 0,
            alt: (bits & NK1_FLAG_ALT) != 0,
        }
    }
}

/// Validation errors returned when encoding or decoding `NK1` payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nk1Error {
    EmptyKey,
    Truncated,
    InvalidMagic,
    InvalidUtf8,
}

/// Binary `NK1` binary keydown payload matching `packages/core/abi/src/events.rs` byte-for-byte.
///
/// Wire layout: `[0x4E ('N'), 0x4B ('K'), 0x31 ('1'), flags_u8, utf8_key...]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nk1KeyDown {
    pub key: String,
    pub flags: Nk1Flags,
}

impl Nk1KeyDown {
    pub fn new(key: impl Into<String>, flags: Nk1Flags) -> Self {
        Self { key: key.into(), flags }
    }

    /// Encodes a [`RemoteAction`] directly into `NK1` wire bytes.
    pub fn encode_action(action: RemoteAction, flags: Nk1Flags) -> Vec<u8> {
        let key_bytes = action.key_name().as_bytes();
        let mut out = Vec::with_capacity(NK1_HEADER_SIZE + key_bytes.len());
        out.extend_from_slice(&NK1_MAGIC);
        out.push(flags.to_byte());
        out.extend_from_slice(key_bytes);
        out
    }

    /// Encodes this keydown event into `NK1` wire bytes.
    pub fn encode(&self) -> Result<Vec<u8>, Nk1Error> {
        if self.key.is_empty() {
            return Err(Nk1Error::EmptyKey);
        }
        let key_bytes = self.key.as_bytes();
        let mut out = Vec::with_capacity(NK1_HEADER_SIZE + key_bytes.len());
        out.extend_from_slice(&NK1_MAGIC);
        out.push(self.flags.to_byte());
        out.extend_from_slice(key_bytes);
        Ok(out)
    }

    /// Decodes a raw `NK1` wire payload with strict UTF-8 and header validation.
    pub fn decode(bytes: &[u8]) -> Result<Self, Nk1Error> {
        if bytes.len() < NK1_MIN_LEN {
            return Err(Nk1Error::Truncated);
        }
        if bytes[0..3] != NK1_MAGIC {
            return Err(Nk1Error::InvalidMagic);
        }
        let flags = Nk1Flags::from_byte(bytes[3]);
        let key = std::str::from_utf8(&bytes[NK1_HEADER_SIZE..])
            .map_err(|_| Nk1Error::InvalidUtf8)?;
        if key.is_empty() {
            return Err(Nk1Error::EmptyKey);
        }
        Ok(Self { key: key.to_owned(), flags })
    }

    /// Maps the decoded key name to a typed [`RemoteAction`], if applicable.
    #[inline]
    pub fn as_remote_action(&self) -> Option<RemoteAction> {
        RemoteAction::from_key_name(&self.key)
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
