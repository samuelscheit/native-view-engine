//! Text shaping cache keys/entries, unified `TextLayoutParams` + `layout_text` shaping helper,
//! width/finiteness sanitization, `TextAlign`, and CSS `font-weight` parsing.

use std::sync::Arc;

use cosmic_text::{Buffer, FontSystem, Metrics, Shaping, SwashContent, Wrap};
use nve_abi::Xxh64Hasher;

/// Default logical font size used when an invalid (non-positive / non-finite) size is passed.
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
/// Default CSS font weight (`normal` / `400`).
pub const DEFAULT_FONT_WEIGHT: u16 = 400;
/// Line-height ratio matching NVE's cross-platform layout conventions.
pub const LINE_HEIGHT_MULTIPLIER: f32 = 1.35;
/// Upper finite bound for text layout widths so `f32::MAX`/`INFINITY` never produce `inf` positions.
const MAX_LAYOUT_WIDTH: f32 = 1_000_000.0;

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

/// Canonicalizes `-0.0` to `+0.0` so cache keys never split across signed zeros.
#[inline]
pub(crate) fn canonical_f32(v: f32) -> f32 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

#[inline]
pub(crate) fn sanitize_font_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        canonical_f32(font_size.min(4096.0))
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
        canonical_f32(scale_factor.min(64.0))
    } else {
        1.0
    }
}

/// Sanitizes an optional line-wrap width (drops non-finite / `<= 0.0`, clamps huge finite values,
/// canonicalizes `-0.0`).
#[inline]
pub(crate) fn sanitize_wrap_width(wrap_width: Option<f32>) -> Option<f32> {
    wrap_width
        .filter(|w| w.is_finite() && *w > 0.0)
        .map(|w| canonical_f32(w.min(MAX_LAYOUT_WIDTH)))
}

/// Computes the padded wrap width from a layout box width, shared by software & Skia text paint.
#[inline]
pub(crate) fn wrap_width_from_rect(rect_width: f32) -> Option<f32> {
    if rect_width.is_finite() && rect_width > 0.0 {
        Some(canonical_f32((rect_width.min(MAX_LAYOUT_WIDTH) + 0.25).ceil()))
    } else {
        None
    }
}

/// Sanitizes and quantizes `align_width` to quarter-pixel steps, guaranteed finite & `>= 0.0`
/// (with `-0.0` canonicalized to `+0.0`).
#[inline]
pub(crate) fn sanitize_align_width(text_align: TextAlign, align_width: f32) -> f32 {
    if text_align == TextAlign::Left || !align_width.is_finite() || align_width <= 0.0 {
        0.0
    } else {
        let clamped = align_width.min(MAX_LAYOUT_WIDTH);
        canonical_f32((clamped * 4.0).round() * 0.25)
    }
}

/// Normalized text layout parameters shared by measurement, software rasterization, and Skia
/// `TextBlob` assembly so all three share one exact `cosmic-text` shaping configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextLayoutParams {
    pub(crate) font_size: f32,
    pub(crate) font_weight: u16,
    pub(crate) text_align: TextAlign,
    pub(crate) wrap_width: Option<f32>,
    pub(crate) align_width: f32,
}

impl TextLayoutParams {
    #[inline]
    pub(crate) fn new(
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
    ) -> Self {
        Self {
            font_size: sanitize_font_size(font_size),
            font_weight: sanitize_font_weight(font_weight),
            text_align,
            wrap_width: sanitize_wrap_width(wrap_width),
            align_width: sanitize_align_width(text_align, align_width),
        }
    }

    #[inline]
    pub(crate) fn line_height(self) -> f32 {
        line_height_for(self.font_size)
    }

    #[inline]
    pub(crate) fn font_size_bits(self) -> u32 {
        canonical_f32(self.font_size).to_bits()
    }

    #[inline]
    pub(crate) fn wrap_width_bits(self) -> u32 {
        self.wrap_width.map_or(u32::MAX, |w| canonical_f32(w).to_bits())
    }

    #[inline]
    pub(crate) fn align_width_bits(self) -> u32 {
        canonical_f32(self.align_width).to_bits()
    }
}

/// Single canonical `cosmic-text` shaping entry point shared by measurement, software paint,
/// and Skia `TextBlob` construction.
pub(crate) fn layout_text(
    font_system: &mut FontSystem,
    text: &str,
    params: TextLayoutParams,
) -> Buffer {
    let mut buffer = Buffer::new_empty(Metrics::new(params.font_size, params.line_height()));
    buffer.set_wrap(Wrap::WordOrGlyph);
    buffer.set_size(params.wrap_width, None);
    let attrs = crate::text::CosmicTextEngine::attrs_for_weight(params.font_weight);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

/// Computes the logical X alignment offset for a laid-out line of width `line_w`.
#[inline]
pub(crate) fn line_align_offset_logical(
    text_align: TextAlign,
    align_width: f32,
    line_w: f32,
) -> f32 {
    if !line_w.is_finite() {
        return 0.0;
    }
    match text_align {
        TextAlign::Left => 0.0,
        TextAlign::Center => ((align_width - line_w) * 0.5).max(0.0),
        TextAlign::Right => (align_width - line_w).max(0.0),
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
    params: TextLayoutParams,
    scale_bits: u32,
) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(text.as_bytes());
    h.update(&[0xFE, params.text_align.as_u8()]);
    h.update(&params.font_weight.to_le_bytes());
    for bits in [
        params.font_size_bits(),
        params.wrap_width_bits(),
        params.align_width_bits(),
        scale_bits,
    ] {
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
    #[inline]
    pub(crate) fn matches(&self, text: &str, params: TextLayoutParams, scale_bits: u32) -> bool {
        self.font_size_bits == params.font_size_bits()
            && self.font_weight == params.font_weight
            && self.text_align == params.text_align
            && self.wrap_width_bits == params.wrap_width_bits()
            && self.align_width_bits == params.align_width_bits()
            && self.scale_bits == scale_bits
            && self.text == text
    }
}

/// Single HarfBuzz-shaped glyph in text-box-local logical coordinates for Skia `TextBlob` assembly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkiaShapedGlyph {
    pub font_id: cosmic_text::fontdb::ID,
    pub glyph_id: u16,
    pub font_size: f32,
    pub x: f32,
    pub y: f32,
}

/// Reusable scratch buffer of shaped logical glyphs emitted by [`crate::text::CosmicTextEngine::shape_for_skia_runs`].
#[derive(Debug, Default)]
pub struct SkiaShapedRuns {
    pub glyphs: Vec<SkiaShapedGlyph>,
    pub max_line_width: f32,
    pub total_height: f32,
}

impl SkiaShapedRuns {
    pub fn clear(&mut self) {
        self.glyphs.clear();
        self.max_line_width = 0.0;
        self.total_height = 0.0;
    }
}

/// Backing source for a `fontdb` face resolved without holding the `FontSystem` mutex during
/// Skia `Typeface` construction:
/// - `Shared` wraps `fontdb`'s existing `Arc<Mmap>` (`Source::SharedFile`) or `Arc<Vec<u8>>`
///   (`Source::Binary`) zero-copy via `skia_safe::Data::new_with_owner` (zero double-mmap, zero
///   heap copy, zero TOCTOU path races).
/// - `File` maps unshared on-disk `Source::File` faces lazily via `SkData::MakeFromFileName`.
pub enum FontFaceSource {
    File {
        path: std::path::PathBuf,
        face_index: u32,
    },
    Shared {
        data: Arc<dyn AsRef<[u8]> + Send + Sync>,
        face_index: u32,
        is_mmap: bool,
    },
}

impl crate::text::CosmicTextEngine {
    /// Resolves the backing [`FontFaceSource`] for `face_id`, releasing the `FontSystem` mutex
    /// BEFORE any Skia typeface decoding occurs.
    pub fn resolve_face_source(&self, face_id: cosmic_text::fontdb::ID) -> Option<FontFaceSource> {
        #[cfg(test)]
        {
            self.face_load_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self
                .fail_face_load
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return None;
            }
            if self
                .fail_only_face_id
                .lock()
                .ok()
                .is_some_and(|g| *g == Some(face_id))
            {
                return None;
            }
        }
        let font_system = self.lock_font_system();
        let face = font_system.db().face(face_id)?;
        match &face.source {
            cosmic_text::fontdb::Source::File(path) => Some(FontFaceSource::File {
                path: path.clone(),
                face_index: face.index,
            }),
            cosmic_text::fontdb::Source::SharedFile(_, data) => Some(FontFaceSource::Shared {
                data: Arc::clone(data),
                face_index: face.index,
                is_mmap: true,
            }),
            cosmic_text::fontdb::Source::Binary(data) => Some(FontFaceSource::Shared {
                data: Arc::clone(data),
                face_index: face.index,
                is_mmap: false,
            }),
        }
    }

    /// Invokes `f(bytes, face_index)` with the raw font file bytes backing `face_id` (tests only).
    #[cfg(test)]
    pub fn with_face_bytes<R>(
        &self,
        face_id: cosmic_text::fontdb::ID,
        f: impl FnOnce(&[u8], u32) -> R,
    ) -> Option<R> {
        if self
            .fail_face_load
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return None;
        }
        let font_system = self.lock_font_system();
        font_system.db().with_face_data(face_id, f)
    }

    /// Returns all loaded `fontdb::ID`s in this engine's font database (tests only).
    #[cfg(test)]
    pub fn all_face_ids(&self) -> Vec<cosmic_text::fontdb::ID> {
        let font_system = self.lock_font_system();
        font_system.db().faces().map(|f| f.id).collect()
    }

    /// Shapes `text` into text-box-local logical glyph positions `(0..w, 0..h)` matching the
    /// exact line-breaking, alignment, and HarfBuzz `x_offset`/`y_offset` math used by the
    /// software rasterizer. Returns `false` when zero system fonts are available.
    #[allow(clippy::too_many_arguments)]
    pub fn shape_for_skia_runs(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
        out: &mut SkiaShapedRuns,
    ) -> bool {
        out.clear();
        if text.is_empty() || !self.has_system_fonts() {
            return self.has_system_fonts();
        }
        let params = TextLayoutParams::new(font_size, font_weight, text_align, wrap_width, align_width);

        let mut font_system = self.lock_font_system();
        let buffer = layout_text(&mut font_system, text, params);

        for run in buffer.layout_runs() {
            if run.line_w.is_finite() {
                out.max_line_width = out.max_line_width.max(run.line_w);
            }
            let bottom = run.line_top + run.line_height;
            if bottom.is_finite() {
                out.total_height = out.total_height.max(bottom);
            }
            let align_x_logical =
                line_align_offset_logical(params.text_align, params.align_width, run.line_w);
            for glyph in run.glyphs {
                // `cosmic-text` stores glyph offsets in em units with Y-up sign convention
                // (see `LayoutGlyph::physical` in `cosmic-text`).
                let gx = align_x_logical + glyph.x + glyph.font_size * glyph.x_offset;
                let gy = run.line_y + glyph.y - glyph.font_size * glyph.y_offset;
                if gx.is_finite() && gy.is_finite() && glyph.font_size.is_finite() {
                    out.glyphs.push(SkiaShapedGlyph {
                        font_id: glyph.font_id,
                        glyph_id: glyph.glyph_id,
                        font_size: glyph.font_size,
                        x: gx,
                        y: gy,
                    });
                }
            }
        }
        true
    }
}
