//! Text shaping cache keys/entries, `TextAlign`, and CSS `font-weight` parsing.

use std::sync::Arc;

use cosmic_text::SwashContent;
use nve_abi::Xxh64Hasher;

/// Default logical font size used when an invalid (non-positive / non-finite) size is passed.
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
/// Default CSS font weight (`normal` / `400`).
pub const DEFAULT_FONT_WEIGHT: u16 = 400;
/// Line-height ratio matching NVE's cross-platform layout conventions.
pub const LINE_HEIGHT_MULTIPLIER: f32 = 1.35;

/// Horizontal text alignment within a text node's layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    /// Parses a CSS `text-align` value (`left`, `start`, `center`, `right`, `end`).
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("left") || s.eq_ignore_ascii_case("start") {
            Some(Self::Left)
        } else if s.eq_ignore_ascii_case("center") {
            Some(Self::Center)
        } else if s.eq_ignore_ascii_case("right") || s.eq_ignore_ascii_case("end") {
            Some(Self::Right)
        } else {
            None
        }
    }

    #[inline]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Center => 1,
            Self::Right => 2,
        }
    }
}

/// Parses a CSS `font-weight` value (`100..=900` or keywords `thin`, `light`, `normal`,
/// `medium`, `semibold`, `bold`, `extrabold`, `black`) into a clamped `100..=900` numeric weight.
pub fn parse_font_weight(raw: &str) -> Option<u16> {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<f32>()
        && n.is_finite()
    {
        return Some((n.round() as i32).clamp(100, 900) as u16);
    }
    if trimmed.eq_ignore_ascii_case("thin") || trimmed.eq_ignore_ascii_case("hairline") {
        Some(100)
    } else if trimmed.eq_ignore_ascii_case("extralight")
        || trimmed.eq_ignore_ascii_case("ultralight")
    {
        Some(200)
    } else if trimmed.eq_ignore_ascii_case("light") {
        Some(300)
    } else if trimmed.eq_ignore_ascii_case("normal") || trimmed.eq_ignore_ascii_case("regular") {
        Some(400)
    } else if trimmed.eq_ignore_ascii_case("medium") {
        Some(500)
    } else if trimmed.eq_ignore_ascii_case("semibold") || trimmed.eq_ignore_ascii_case("demibold") {
        Some(600)
    } else if trimmed.eq_ignore_ascii_case("bold") {
        Some(700)
    } else if trimmed.eq_ignore_ascii_case("extrabold") || trimmed.eq_ignore_ascii_case("ultrabold")
    {
        Some(800)
    } else if trimmed.eq_ignore_ascii_case("black") || trimmed.eq_ignore_ascii_case("heavy") {
        Some(900)
    } else {
        None
    }
}

#[inline]
pub(crate) fn sanitize_font_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        DEFAULT_FONT_SIZE
    }
}

#[inline]
pub(crate) fn sanitize_font_weight(weight: u16) -> u16 {
    if weight == 0 {
        DEFAULT_FONT_WEIGHT
    } else {
        weight.clamp(100, 900)
    }
}

#[inline]
pub(crate) fn line_height_for(font_size: f32) -> f32 {
    (font_size * LINE_HEIGHT_MULTIPLIER).max(font_size + 2.0)
}

#[inline]
pub(crate) fn sanitize_scale(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[inline]
pub(crate) fn hash_measure_key(
    text: &str,
    font_size_bits: u32,
    font_weight: u16,
    max_width_bits: u32,
) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(text.as_bytes());
    h.update(&[0xFF]);
    h.update(&font_weight.to_le_bytes());
    h.update_u32(font_size_bits);
    h.update_u32(max_width_bits);
    h.digest()
}

#[inline]
pub(crate) fn hash_shaped_key(
    text: &str,
    font_size_bits: u32,
    font_weight: u16,
    text_align: TextAlign,
    wrap_width_bits: u32,
    align_width_bits: u32,
    scale_bits: u32,
) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(text.as_bytes());
    h.update(&[0xFE, text_align.as_u8()]);
    h.update(&font_weight.to_le_bytes());
    for bits in [font_size_bits, wrap_width_bits, align_width_bits, scale_bits] {
        h.update_u32(bits);
    }
    h.digest()
}

#[derive(Debug, Clone)]
pub(crate) struct MeasureCacheEntry {
    pub(crate) text: String,
    pub(crate) font_size_bits: u32,
    pub(crate) font_weight: u16,
    pub(crate) max_width_bits: u32,
    pub(crate) size: (f32, f32),
}

impl MeasureCacheEntry {
    #[inline]
    pub(crate) fn matches(
        &self,
        text: &str,
        font_size_bits: u32,
        font_weight: u16,
        max_width_bits: u32,
    ) -> bool {
        self.font_size_bits == font_size_bits
            && self.font_weight == font_weight
            && self.max_width_bits == max_width_bits
            && self.text == text
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CachedGlyphImage {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) content: SwashContent,
    pub(crate) data: Arc<[u8]>,
}

#[derive(Debug, Clone)]
pub(crate) struct PositionedGlyph {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) content: SwashContent,
    pub(crate) data: Arc<[u8]>,
}

#[derive(Debug)]
pub(crate) struct ShapedText {
    pub(crate) glyphs: Vec<PositionedGlyph>,
}

#[derive(Debug)]
pub(crate) struct ShapedCacheEntry {
    pub(crate) text: String,
    pub(crate) font_size_bits: u32,
    pub(crate) font_weight: u16,
    pub(crate) text_align: TextAlign,
    pub(crate) wrap_width_bits: u32,
    pub(crate) align_width_bits: u32,
    pub(crate) scale_bits: u32,
    pub(crate) shaped: Arc<ShapedText>,
}

impl ShapedCacheEntry {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn matches(
        &self,
        text: &str,
        font_size_bits: u32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width_bits: u32,
        align_width_bits: u32,
        scale_bits: u32,
    ) -> bool {
        self.font_size_bits == font_size_bits
            && self.font_weight == font_weight
            && self.text_align == text_align
            && self.wrap_width_bits == wrap_width_bits
            && self.align_width_bits == align_width_bits
            && self.scale_bits == scale_bits
            && self.text == text
    }
}
