//! Native system font discovery, HarfBuzz/Unicode shaping, and antialiased glyph rasterization
//! powered by `cosmic-text` (with graceful fallback to the embedded 5x7 bitmap font when zero
//! system fonts are installed). Supports `font-weight` (`100..=900` / named weights) and
//! `text-align` (`left | center | right`).

use std::sync::{Arc, Mutex, OnceLock};

use cosmic_text::{Attrs, CacheKey, Family, FontSystem, SwashCache, SwashContent, Weight};
use nve_abi::ColorRgba;
use nve_engine::LayoutRect;
use nve_layout::{TextMeasurer, TextStyle};

use crate::font::BitmapFontTextMeasurer;
use crate::raster::{
    blend_color_glyph, blend_mask_glyph, blend_subpixel_glyph, draw_bitmap_text, PixelBuffer,
};
use crate::slab_lru::{FastBuildHasher, IdentityU64BuildHasher, SlabLru};
use crate::text_cache::{
    canonical_f32, hash_measure_key, hash_shaped_key, layout_text, line_align_offset_logical,
    line_height_for, sanitize_font_size, sanitize_font_weight, sanitize_scale, wrap_width_from_rect,
    CachedGlyphImage, MeasureCacheEntry, PositionedGlyph, ShapedCacheEntry, ShapedText,
    TextLayoutParams,
};

pub use crate::text_cache::{
    parse_font_weight, SkiaShapedGlyph, SkiaShapedRuns, TextAlign, DEFAULT_FONT_SIZE,
    DEFAULT_FONT_WEIGHT, LINE_HEIGHT_MULTIPLIER,
};

const MAX_MEASURE_CACHE_ENTRIES: usize = 4096;
const MAX_MEASURE_CACHE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SHAPED_CACHE_ENTRIES: usize = 2048;
const MAX_SHAPED_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_GLYPH_IMAGE_CACHE_ENTRIES: usize = 4096;
const MAX_GLYPH_IMAGE_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Thread-safe text shaping, measurement, and antialiased glyph rasterization engine
/// backed by `cosmic-text` (HarfBuzz + `fontdb` + `swash`), with automatic fallback to the
/// embedded 5x7 bitmap font when no system fonts are available.
pub struct CosmicTextEngine {
    font_system: Mutex<FontSystem>,
    swash_cache: Mutex<SwashCache>,
    glyph_images: Mutex<SlabLru<CacheKey, Option<CachedGlyphImage>, FastBuildHasher>>,
    measure_cache: Mutex<SlabLru<u64, MeasureCacheEntry, IdentityU64BuildHasher>>,
    shaped_cache: Mutex<SlabLru<u64, ShapedCacheEntry, IdentityU64BuildHasher>>,
    has_system_fonts: bool,
    loaded_font_count: usize,
    #[cfg(test)]
    pub(crate) fail_face_load: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(crate) fail_only_face_id: Mutex<Option<cosmic_text::fontdb::ID>>,
    #[cfg(test)]
    pub(crate) face_load_calls: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for CosmicTextEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmicTextEngine")
            .field("has_system_fonts", &self.has_system_fonts)
            .field("loaded_font_count", &self.loaded_font_count)
            .finish()
    }
}

impl Default for CosmicTextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CosmicTextEngine {
    /// Returns a process-wide shared `CosmicTextEngine` with warm font discovery,
    /// layout measurement, and rasterized glyph caches.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<CosmicTextEngine>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::new())))
    }

    /// Creates a new `CosmicTextEngine` loading all system fonts discovered by `fontdb`.
    pub fn new() -> Self {
        Self::from_font_system(FontSystem::new())
    }

    /// Creates a `CosmicTextEngine` from an explicit `cosmic_text::FontSystem`.
    pub fn from_font_system(font_system: FontSystem) -> Self {
        let loaded_font_count = font_system.db().len();
        let has_system_fonts = loaded_font_count > 0;
        let engine = Self {
            font_system: Mutex::new(font_system),
            swash_cache: Mutex::new(SwashCache::new()),
            glyph_images: Mutex::new(SlabLru::with_initial_capacity_and_hasher(
                MAX_GLYPH_IMAGE_CACHE_ENTRIES,
                MAX_GLYPH_IMAGE_CACHE_BYTES,
                256,
                FastBuildHasher,
            )),
            measure_cache: Mutex::new(SlabLru::with_initial_capacity_and_hasher(
                MAX_MEASURE_CACHE_ENTRIES,
                MAX_MEASURE_CACHE_BYTES,
                256,
                IdentityU64BuildHasher,
            )),
            shaped_cache: Mutex::new(SlabLru::with_initial_capacity_and_hasher(
                MAX_SHAPED_CACHE_ENTRIES,
                MAX_SHAPED_CACHE_BYTES,
                256,
                IdentityU64BuildHasher,
            )),
            has_system_fonts,
            loaded_font_count,
            #[cfg(test)]
            fail_face_load: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_only_face_id: Mutex::new(None),
            #[cfg(test)]
            face_load_calls: std::sync::atomic::AtomicU64::new(0),
        };
        // Eagerly prime OS mutex storage so steady-state cache lookups allocate zero bytes.
        drop(engine.font_system.lock());
        drop(engine.swash_cache.lock());
        drop(engine.glyph_images.lock());
        drop(engine.measure_cache.lock());
        drop(engine.shaped_cache.lock());
        engine
    }

    /// Creates a `CosmicTextEngine` with an empty font database to exercise the graceful
    /// bitmap-font fallback path deterministically.
    pub fn new_without_system_fonts() -> Self {
        let empty_db = cosmic_text::fontdb::Database::new();
        let fs = FontSystem::new_with_locale_and_db("en-US".to_string(), empty_db);
        Self::from_font_system(fs)
    }

    #[inline]
    pub fn has_system_fonts(&self) -> bool {
        self.has_system_fonts
    }

    #[inline]
    pub fn loaded_font_count(&self) -> usize {
        self.loaded_font_count
    }

    pub fn clear_caches(&self) {
        if let Ok(mut guard) = self.measure_cache.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.shaped_cache.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.glyph_images.lock() {
            guard.clear();
        }
        if let Ok(mut guard) = self.swash_cache.lock() {
            guard.image_cache.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn debug_swash_image_cache_len(&self) -> usize {
        self.swash_cache
            .lock()
            .map_or(0, |guard| guard.image_cache.len())
    }

    #[cfg(test)]
    pub(crate) fn debug_glyph_image_cache_stats(&self) -> (usize, usize, usize) {
        self.glyph_images
            .lock()
            .map_or((0, 0, MAX_GLYPH_IMAGE_CACHE_BYTES), |guard| {
                (guard.len(), guard.live_bytes(), guard.max_bytes())
            })
    }

    pub(crate) fn attrs_for_weight(weight: u16) -> Attrs<'static> {
        Attrs::new()
            .family(Family::SansSerif)
            .weight(Weight(sanitize_font_weight(weight)))
    }

    pub(crate) fn lock_font_system(&self) -> std::sync::MutexGuard<'_, FontSystem> {
        self.font_system
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Measures `text` at logical `font_size` under an optional `max_width` wrapping bound.
    pub fn measure_text(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_text_with_weight(text, font_size, DEFAULT_FONT_WEIGHT, max_width)
    }

    /// Measures `text` at logical `font_size` and CSS `font_weight` (`100..=900`) under an optional
    /// `max_width` wrapping bound.
    pub fn measure_text_with_weight(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        if !self.has_system_fonts {
            return BitmapFontTextMeasurer.measure_text(text, font_size, max_width);
        }

        let fs = sanitize_font_size(font_size);
        let weight = sanitize_font_weight(font_weight);
        let lh = line_height_for(fs);

        let normalized_max_w = match max_width {
            Some(mw) if !mw.is_finite() || mw <= 0.0 => {
                let explicit_lines = text.split('\n').count().max(1) as f32;
                return (0.0, (explicit_lines * lh).ceil());
            }
            Some(mw) => Some(canonical_f32(mw)),
            None => None,
        };

        let params = TextLayoutParams::new(fs, weight, TextAlign::Left, normalized_max_w, 0.0);
        let fs_bits = params.font_size_bits();
        let mw_bits = params.wrap_width_bits();
        let cache_hash = hash_measure_key(text, fs_bits, weight, mw_bits);

        if let Ok(mut cache) = self.measure_cache.lock()
            && let Some(entry) = cache.get_mut(&cache_hash)
            && entry.matches(text, fs_bits, weight, mw_bits)
        {
            return entry.size;
        }

        let size = {
            let mut font_system = self.lock_font_system();
            let buffer = layout_text(&mut font_system, text, params);

            let mut max_line_w = 0.0f32;
            let mut total_h = 0.0f32;
            let mut run_count = 0usize;
            for run in buffer.layout_runs() {
                if run.line_w.is_finite() {
                    max_line_w = max_line_w.max(run.line_w);
                }
                let bottom = run.line_top + run.line_height;
                if bottom.is_finite() {
                    total_h = total_h.max(bottom);
                }
                run_count += 1;
            }

            if run_count == 0 {
                let explicit_lines = text.split('\n').count().max(1) as f32;
                (1.0, (explicit_lines * lh).ceil())
            } else {
                // Add a 1px subpixel/hinting margin so subsequent shaping against `rect.width`
                // never wraps the final word or clips right-side glyph overhang.
                let ceiled_w = (max_line_w + 1.0).ceil().max(1.0);
                let final_w = match params.wrap_width {
                    Some(mw) => ceiled_w.min(mw),
                    None => ceiled_w,
                };
                let final_h = total_h.max(lh).ceil();
                (final_w, final_h)
            }
        };

        if let Ok(mut cache) = self.measure_cache.lock() {
            let entry_bytes = 64 + text.len();
            cache.insert(
                cache_hash,
                MeasureCacheEntry {
                    text: text.to_owned(),
                    font_size_bits: fs_bits,
                    font_weight: weight,
                    max_width_bits: mw_bits,
                    size,
                },
                entry_bytes,
            );
        }

        size
    }

    #[allow(clippy::too_many_arguments)]
    fn get_or_shape(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        wrap_width: Option<f32>,
        align_width: f32,
        scale: f32,
    ) -> Arc<ShapedText> {
        let params = TextLayoutParams::new(font_size, font_weight, text_align, wrap_width, align_width);
        let scale_bits = canonical_f32(scale).to_bits();
        let cache_hash = hash_shaped_key(text, params, scale_bits);

        if let Ok(mut cache) = self.shaped_cache.lock()
            && let Some(entry) = cache.get_mut(&cache_hash)
            && entry.matches(text, params, scale_bits)
        {
            return Arc::clone(&entry.shaped);
        }

        let shaped = {
            let mut font_system = self.lock_font_system();
            let mut swash_cache = self
                .swash_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut glyph_images = self
                .glyph_images
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            let buffer = layout_text(&mut font_system, text, params);

            let mut glyphs = Vec::new();
            for run in buffer.layout_runs() {
                let align_x_logical =
                    line_align_offset_logical(params.text_align, params.align_width, run.line_w);
                let origin_px = (align_x_logical * scale, run.line_y * scale);
                for glyph in run.glyphs {
                    let physical = glyph.physical(origin_px, scale);
                    let cached_img = if let Some(hit) = glyph_images.get_mut(&physical.cache_key) {
                        hit.clone()
                    } else {
                        // Use `get_image_uncached` so `cosmic-text`'s unbounded internal
                        // `SwashCache::image_cache` never duplicates our bounded `SlabLru`.
                        let built = swash_cache
                            .get_image_uncached(&mut font_system, physical.cache_key)
                            .filter(|img| img.placement.width > 0 && img.placement.height > 0)
                            .map(|img| CachedGlyphImage {
                                left: img.placement.left,
                                top: img.placement.top,
                                width: img.placement.width,
                                height: img.placement.height,
                                content: img.content,
                                data: Arc::from(img.data.as_slice()),
                            });
                        let bytes = 64 + built.as_ref().map_or(0, |b| b.data.len());
                        glyph_images.insert(physical.cache_key, built.clone(), bytes);
                        built
                    };

                    if let Some(img) = cached_img {
                        glyphs.push(PositionedGlyph {
                            x: physical.x + img.left,
                            y: physical.y - img.top,
                            width: img.width,
                            height: img.height,
                            content: img.content,
                            data: Arc::clone(&img.data),
                        });
                    }
                }
            }

            Arc::new(ShapedText { glyphs })
        };

        if let Ok(mut cache) = self.shaped_cache.lock() {
            let entry_bytes =
                96 + text.len() + shaped.glyphs.len() * std::mem::size_of::<PositionedGlyph>();
            cache.insert(
                cache_hash,
                ShapedCacheEntry {
                    text: text.to_owned(),
                    font_size_bits: params.font_size_bits(),
                    font_weight: params.font_weight,
                    text_align: params.text_align,
                    wrap_width_bits: params.wrap_width_bits(),
                    align_width_bits: params.align_width_bits(),
                    scale_bits,
                    shaped: Arc::clone(&shaped),
                },
                entry_bytes,
            );
        }

        shaped
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_text(
        &self,
        buf: &mut PixelBuffer,
        rect: LayoutRect,
        text: &str,
        font_size: f32,
        scale_factor: f32,
        color: ColorRgba,
        opacity: f32,
    ) {
        self.rasterize_text_styled_clipped(
            buf,
            rect,
            text,
            font_size,
            DEFAULT_FONT_WEIGHT,
            TextAlign::Left,
            scale_factor,
            color,
            opacity,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_text_styled_clipped(
        &self,
        buf: &mut PixelBuffer,
        rect: LayoutRect,
        text: &str,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        scale_factor: f32,
        color: ColorRgba,
        opacity: f32,
        phys_clip: Option<[i32; 4]>,
    ) {
        if text.is_empty()
            || !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
            || !opacity.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
            || color.a == 0
            || opacity <= 0.0
        {
            return;
        }

        let scale = sanitize_scale(scale_factor);
        let fs = sanitize_font_size(font_size);
        let weight = sanitize_font_weight(font_weight);

        if !self.has_system_fonts {
            draw_bitmap_text(
                buf,
                rect,
                text,
                fs,
                weight,
                text_align,
                scale,
                color,
                opacity,
                phys_clip,
            );
            return;
        }

        let buf_w = buf.width() as i32;
        let buf_h = buf.height() as i32;
        if buf_w <= 0 || buf_h <= 0 {
            return;
        }

        let origin_x = (rect.x * scale).round() as i32;
        let origin_y = (rect.y * scale).round() as i32;
        let clip_x0 = (rect.x * scale).floor() as i32;
        let clip_y0 = (rect.y * scale).floor() as i32;
        let clip_x1 = ((rect.x + rect.width) * scale).ceil() as i32;
        let clip_y1 = ((rect.y + rect.height) * scale).ceil() as i32;

        let (min_x, min_y, max_x, max_y) = match phys_clip {
            Some([px0, py0, px1, py1]) => (
                clip_x0.max(px0).max(0),
                clip_y0.max(py0).max(0),
                clip_x1.min(px1).min(buf_w),
                clip_y1.min(py1).min(buf_h),
            ),
            None => (
                clip_x0.max(0),
                clip_y0.max(0),
                clip_x1.min(buf_w),
                clip_y1.min(buf_h),
            ),
        };
        if min_x >= max_x || min_y >= max_y {
            return;
        }

        let wrap_width = wrap_width_from_rect(rect.width);
        let shaped = self.get_or_shape(text, fs, weight, text_align, wrap_width, rect.width, scale);

        let opacity_clamped = opacity.clamp(0.0, 1.0);
        let base_alpha = ((color.a as f32) * opacity_clamped).round() as u32;
        let opacity_u32 = (opacity_clamped * 255.0).round() as u32;
        if base_alpha == 0 && opacity_u32 == 0 {
            return;
        }

        let stride = buf_w as usize;
        let pixels = buf.pixels_mut();

        for glyph in &shaped.glyphs {
            let gx = origin_x + glyph.x;
            let gy = origin_y + glyph.y;
            let gw = glyph.width as i32;
            let gh = glyph.height as i32;

            let dst_x0 = gx.max(min_x);
            let dst_y0 = gy.max(min_y);
            let dst_x1 = (gx + gw).min(max_x);
            let dst_y1 = (gy + gh).min(max_y);
            if dst_x0 >= dst_x1 || dst_y0 >= dst_y1 {
                continue;
            }

            let src_x_off = (dst_x0 - gx) as usize;
            let src_y_off = (dst_y0 - gy) as usize;
            let span_w = (dst_x1 - dst_x0) as usize;
            let rows = (dst_y1 - dst_y0) as usize;
            let gw_usize = glyph.width as usize;

            match glyph.content {
                SwashContent::Mask => blend_mask_glyph(
                    pixels,
                    stride,
                    &glyph.data,
                    gw_usize,
                    dst_x0 as usize,
                    dst_y0 as usize,
                    src_x_off,
                    src_y_off,
                    span_w,
                    rows,
                    color,
                    base_alpha,
                ),
                SwashContent::SubpixelMask => blend_subpixel_glyph(
                    pixels,
                    stride,
                    &glyph.data,
                    gw_usize,
                    dst_x0 as usize,
                    dst_y0 as usize,
                    src_x_off,
                    src_y_off,
                    span_w,
                    rows,
                    color,
                    base_alpha,
                ),
                SwashContent::Color => blend_color_glyph(
                    pixels,
                    stride,
                    &glyph.data,
                    gw_usize,
                    dst_x0 as usize,
                    dst_y0 as usize,
                    src_x_off,
                    src_y_off,
                    span_w,
                    rows,
                    opacity_u32,
                ),
            }
        }
    }
}

/// Measures with the size and weight the Skia painter shapes with; `nowrap` / `pre` never wrap.
impl TextMeasurer for CosmicTextEngine {
    fn measure(&self, text: &str, style: &TextStyle<&str>, max_width: Option<f32>) -> (f32, f32) {
        let max_width = max_width.filter(|_| style.wraps());
        self.measure_text_with_weight(text, style.font_size, style.font_weight, max_width)
    }
}
