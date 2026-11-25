//! Native system font discovery, HarfBuzz/Unicode shaping, and antialiased glyph rasterization
//! powered by `cosmic-text` (with graceful fallback to the embedded 5x7 bitmap font when zero
//! system fonts are installed).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap,
};
use nve_abi::ColorRgba;
use nve_engine::{LayoutRect, TextMeasurer};

use crate::font::BitmapFontTextMeasurer;
use crate::raster::{draw_bitmap_text, PixelBuffer};

/// Default logical font size used when an invalid (non-positive / non-finite) size is passed.
pub const DEFAULT_FONT_SIZE: f32 = 16.0;
/// Line-height ratio matching NVE's cross-platform layout conventions.
pub const LINE_HEIGHT_MULTIPLIER: f32 = 1.35;

const MAX_MEASURE_CACHE_ENTRIES: usize = 4096;
const MAX_SHAPED_CACHE_ENTRIES: usize = 2048;
const MAX_GLYPH_IMAGE_CACHE_ENTRIES: usize = 4096;

#[inline]
fn sanitize_font_size(font_size: f32) -> f32 {
    if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        DEFAULT_FONT_SIZE
    }
}

#[inline]
fn line_height_for(font_size: f32) -> f32 {
    (font_size * LINE_HEIGHT_MULTIPLIER).max(font_size + 2.0)
}

#[inline]
fn sanitize_scale(scale_factor: f32) -> f32 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[inline]
fn fnv1a_step(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[inline]
fn hash_measure_key(text: &str, font_size_bits: u32, max_width_bits: u32) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    h = fnv1a_step(h, text.as_bytes());
    h = fnv1a_step(h, &[0xFF]);
    h = fnv1a_step(h, &font_size_bits.to_le_bytes());
    h = fnv1a_step(h, &max_width_bits.to_le_bytes());
    h
}

#[inline]
fn hash_shaped_key(
    text: &str,
    font_size_bits: u32,
    wrap_width_bits: u32,
    scale_bits: u32,
) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    h = fnv1a_step(h, text.as_bytes());
    h = fnv1a_step(h, &[0xFE]);
    h = fnv1a_step(h, &font_size_bits.to_le_bytes());
    h = fnv1a_step(h, &wrap_width_bits.to_le_bytes());
    h = fnv1a_step(h, &scale_bits.to_le_bytes());
    h
}

#[derive(Debug, Clone)]
struct MeasureCacheEntry {
    text: String,
    font_size_bits: u32,
    max_width_bits: u32,
    size: (f32, f32),
}

impl MeasureCacheEntry {
    #[inline]
    fn matches(&self, text: &str, font_size_bits: u32, max_width_bits: u32) -> bool {
        self.font_size_bits == font_size_bits
            && self.max_width_bits == max_width_bits
            && self.text == text
    }
}

#[derive(Debug, Clone)]
struct CachedGlyphImage {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    content: SwashContent,
    data: Arc<[u8]>,
}

#[derive(Debug, Clone)]
struct PositionedGlyph {
    /// Physical X offset relative to the text box origin (already includes `placement.left`).
    x: i32,
    /// Physical Y offset relative to the text box origin (already includes baseline - `placement.top`).
    y: i32,
    width: u32,
    height: u32,
    content: SwashContent,
    data: Arc<[u8]>,
}

#[derive(Debug)]
struct ShapedText {
    glyphs: Vec<PositionedGlyph>,
}

#[derive(Debug)]
struct ShapedCacheEntry {
    text: String,
    font_size_bits: u32,
    wrap_width_bits: u32,
    scale_bits: u32,
    shaped: Arc<ShapedText>,
}

impl ShapedCacheEntry {
    #[inline]
    fn matches(
        &self,
        text: &str,
        font_size_bits: u32,
        wrap_width_bits: u32,
        scale_bits: u32,
    ) -> bool {
        self.font_size_bits == font_size_bits
            && self.wrap_width_bits == wrap_width_bits
            && self.scale_bits == scale_bits
            && self.text == text
    }
}

/// Thread-safe text shaping, measurement, and antialiased glyph rasterization engine
/// backed by `cosmic-text` (HarfBuzz + `fontdb` + `swash`), with automatic fallback to the
/// embedded 5x7 bitmap font when no system fonts are available.
pub struct CosmicTextEngine {
    font_system: Mutex<FontSystem>,
    swash_cache: Mutex<SwashCache>,
    glyph_images: Mutex<HashMap<CacheKey, Option<CachedGlyphImage>>>,
    measure_cache: Mutex<HashMap<u64, MeasureCacheEntry>>,
    shaped_cache: Mutex<HashMap<u64, ShapedCacheEntry>>,
    has_system_fonts: bool,
    loaded_font_count: usize,
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
    ///
    /// If `font_system.db()` contains zero faces (e.g. inside a stripped container without
    /// system fonts), measurement and rasterization automatically fall back to the embedded
    /// 5x7 bitmap font.
    pub fn from_font_system(font_system: FontSystem) -> Self {
        let loaded_font_count = font_system.db().len();
        let has_system_fonts = loaded_font_count > 0;
        Self {
            font_system: Mutex::new(font_system),
            swash_cache: Mutex::new(SwashCache::new()),
            glyph_images: Mutex::new(HashMap::with_capacity(256)),
            measure_cache: Mutex::new(HashMap::with_capacity(256)),
            shaped_cache: Mutex::new(HashMap::with_capacity(256)),
            has_system_fonts,
            loaded_font_count,
        }
    }

    /// Creates a `CosmicTextEngine` with an empty font database to exercise the graceful
    /// bitmap-font fallback path deterministically.
    pub fn new_without_system_fonts() -> Self {
        let empty_db = cosmic_text::fontdb::Database::new();
        let fs = FontSystem::new_with_locale_and_db("en-US".to_string(), empty_db);
        Self::from_font_system(fs)
    }

    /// Returns `true` if at least one native system font face was discovered.
    #[inline]
    pub fn has_system_fonts(&self) -> bool {
        self.has_system_fonts
    }

    /// Returns the number of font faces discovered in `fontdb`.
    #[inline]
    pub fn loaded_font_count(&self) -> usize {
        self.loaded_font_count
    }

    /// Clears the measurement and shaped-glyph caches (useful in tests or after memory pressure).
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

    fn default_attrs() -> Attrs<'static> {
        Attrs::new().family(Family::SansSerif)
    }

    /// Measures `text` at logical `font_size` under an optional `max_width` wrapping bound.
    pub fn measure_text(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        if text.is_empty() {
            return (0.0, 0.0);
        }
        if !self.has_system_fonts {
            return BitmapFontTextMeasurer.measure(text, font_size, max_width);
        }

        let fs = sanitize_font_size(font_size);
        let lh = line_height_for(fs);

        // Handle min-content / degenerate width probes identically to HeuristicTextMeasurer.
        let normalized_max_w = match max_width {
            Some(mw) if !mw.is_finite() || mw <= 0.0 => {
                let explicit_lines = text.split('\n').count().max(1) as f32;
                return (0.0, (explicit_lines * lh).ceil());
            }
            Some(mw) => Some(mw),
            None => None,
        };

        let fs_bits = fs.to_bits();
        let mw_bits = normalized_max_w.map_or(u32::MAX, f32::to_bits);
        let cache_hash = hash_measure_key(text, fs_bits, mw_bits);

        if let Ok(cache) = self.measure_cache.lock() {
            if let Some(entry) = cache.get(&cache_hash) {
                if entry.matches(text, fs_bits, mw_bits) {
                    return entry.size;
                }
            }
        }

        let size = {
            let mut font_system = self
                .font_system
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut buffer = Buffer::new_empty(Metrics::new(fs, lh));
            buffer.set_wrap(Wrap::WordOrGlyph);
            buffer.set_size(normalized_max_w, None);
            let attrs = Self::default_attrs();
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut font_system, false);

            let mut max_line_w = 0.0f32;
            let mut total_h = 0.0f32;
            let mut run_count = 0usize;
            for run in buffer.layout_runs() {
                max_line_w = max_line_w.max(run.line_w);
                total_h = total_h.max(run.line_top + run.line_height);
                run_count += 1;
            }

            if run_count == 0 {
                // Whitespace-only or empty-run fallback still occupies one line height.
                let explicit_lines = text.split('\n').count().max(1) as f32;
                (1.0, (explicit_lines * lh).ceil())
            } else {
                // Ceil to integer logical points so Taffy's layout box never rounds below
                // the shaped line width and triggers spurious wrapping at rasterization time.
                let ceiled_w = max_line_w.ceil().max(1.0);
                let final_w = match normalized_max_w {
                    Some(mw) => ceiled_w.min(mw),
                    None => ceiled_w,
                };
                let final_h = total_h.max(lh).ceil();
                (final_w, final_h)
            }
        };

        if let Ok(mut cache) = self.measure_cache.lock() {
            if cache.len() >= MAX_MEASURE_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(
                cache_hash,
                MeasureCacheEntry {
                    text: text.to_owned(),
                    font_size_bits: fs_bits,
                    max_width_bits: mw_bits,
                    size,
                },
            );
        }

        size
    }

    fn get_or_shape(
        &self,
        text: &str,
        font_size: f32,
        wrap_width: Option<f32>,
        scale: f32,
    ) -> Arc<ShapedText> {
        let fs_bits = font_size.to_bits();
        let ww_bits = wrap_width.map_or(u32::MAX, f32::to_bits);
        let scale_bits = scale.to_bits();
        let cache_hash = hash_shaped_key(text, fs_bits, ww_bits, scale_bits);

        if let Ok(cache) = self.shaped_cache.lock() {
            if let Some(entry) = cache.get(&cache_hash) {
                if entry.matches(text, fs_bits, ww_bits, scale_bits) {
                    return Arc::clone(&entry.shaped);
                }
            }
        }

        let lh = line_height_for(font_size);
        let shaped = {
            let mut font_system = self
                .font_system
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut swash_cache = self
                .swash_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let mut glyph_images = self
                .glyph_images
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            if swash_cache.image_cache.len() > MAX_GLYPH_IMAGE_CACHE_ENTRIES {
                swash_cache.image_cache.clear();
            }
            if glyph_images.len() > MAX_GLYPH_IMAGE_CACHE_ENTRIES {
                glyph_images.clear();
            }

            let mut buffer = Buffer::new_empty(Metrics::new(font_size, lh));
            buffer.set_wrap(Wrap::WordOrGlyph);
            buffer.set_size(wrap_width, None);
            let attrs = Self::default_attrs();
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut font_system, false);

            let mut glyphs = Vec::new();
            for run in buffer.layout_runs() {
                let baseline_y = run.line_y * scale;
                for glyph in run.glyphs {
                    let physical = glyph.physical((0.0, baseline_y), scale);
                    let cached_img = glyph_images
                        .entry(physical.cache_key)
                        .or_insert_with(|| {
                            swash_cache
                                .get_image(&mut font_system, physical.cache_key)
                                .as_ref()
                                .filter(|img| img.placement.width > 0 && img.placement.height > 0)
                                .map(|img| CachedGlyphImage {
                                    left: img.placement.left,
                                    top: img.placement.top,
                                    width: img.placement.width,
                                    height: img.placement.height,
                                    content: img.content,
                                    data: Arc::from(img.data.as_slice()),
                                })
                        });

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
            if cache.len() >= MAX_SHAPED_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(
                cache_hash,
                ShapedCacheEntry {
                    text: text.to_owned(),
                    font_size_bits: fs_bits,
                    wrap_width_bits: ww_bits,
                    scale_bits,
                    shaped: Arc::clone(&shaped),
                },
            );
        }

        shaped
    }

    /// Rasterizes `text` into `buf` inside logical `rect`, scaled by `scale_factor`,
    /// alpha-blended with `color` and group `opacity`, and strictly clipped to `rect`.
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
        self.rasterize_text_clipped(buf, rect, text, font_size, scale_factor, color, opacity, None);
    }

    /// Rasterizes `text` into `buf` inside logical `rect`, additionally clipped to an optional
    /// physical-pixel bounding box `[min_x, min_y, max_x, max_y]`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rasterize_text_clipped(
        &self,
        buf: &mut PixelBuffer,
        rect: LayoutRect,
        text: &str,
        font_size: f32,
        scale_factor: f32,
        color: ColorRgba,
        opacity: f32,
        phys_clip: Option<[i32; 4]>,
    ) {
        if text.is_empty()
            || rect.width <= 0.0
            || rect.height <= 0.0
            || color.a == 0
            || opacity <= 0.0
        {
            return;
        }

        let scale = sanitize_scale(scale_factor);
        let fs = sanitize_font_size(font_size);

        if !self.has_system_fonts {
            let scaled_rect = LayoutRect {
                x: rect.x * scale,
                y: rect.y * scale,
                width: rect.width * scale,
                height: rect.height * scale,
            };
            draw_bitmap_text(buf, scaled_rect, text, fs * scale, color, opacity, phys_clip);
            return;
        }

        let buf_w = buf.width() as i32;
        let buf_h = buf.height() as i32;
        if buf_w <= 0 || buf_h <= 0 {
            return;
        }

        // Compute physical pixel clip bounds from logical target rect intersected with active clip.
        let origin_x = (rect.x * scale).round() as i32;
        let origin_y = (rect.y * scale).round() as i32;
        let clip_x0 = (rect.x * scale).floor() as i32;
        let clip_y0 = (rect.y * scale).floor() as i32;
        let clip_x1 = ((rect.x + rect.width) * scale).ceil() as i32;
        let clip_y1 = ((rect.y + rect.height) * scale).ceil() as i32;

        let (cx0, cy0, cx1, cy1) = match phys_clip {
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
        let min_x = cx0;
        let min_y = cy0;
        let max_x = cx1;
        let max_y = cy1;
        if min_x >= max_x || min_y >= max_y {
            return;
        }

        // Quantize wrap width slightly upward (+0.25pt) to prevent float rounding from
        // Taffy layout causing a spurious extra wrap on exact-fit single-line text.
        let wrap_width = if rect.width.is_finite() && rect.width > 0.0 {
            Some((rect.width + 0.25).ceil())
        } else {
            None
        };

        let shaped = self.get_or_shape(text, fs, wrap_width, scale);

        let opacity_clamped = opacity.clamp(0.0, 1.0);
        let base_alpha = ((color.a as f32) * opacity_clamped).round() as u32;
        let opacity_u32 = (opacity_clamped * 255.0).round() as u32;
        if base_alpha == 0 && opacity_u32 == 0 {
            return;
        }

        let packed_fg = PixelBuffer::pack_rgb(color.r, color.g, color.b);
        let sr_u32 = color.r as u32;
        let sg_u32 = color.g as u32;
        let sb_u32 = color.b as u32;
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
            let gw_usize = glyph.width as usize;

            match glyph.content {
                SwashContent::Mask => {
                    if base_alpha == 0 {
                        continue;
                    }
                    for row in 0..(dst_y1 - dst_y0) as usize {
                        let dst_row_start = (dst_y0 as usize + row) * stride + dst_x0 as usize;
                        let src_row_start = (src_y_off + row) * gw_usize + src_x_off;
                        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];
                        let src_slice = &glyph.data[src_row_start..src_row_start + span_w];

                        if base_alpha >= 255 {
                            for (dst_px, &mask_a) in dst_slice.iter_mut().zip(src_slice.iter()) {
                                if mask_a == 0 {
                                    continue;
                                }
                                if mask_a == 255 {
                                    *dst_px = packed_fg;
                                    continue;
                                }
                                let a = mask_a as u32;
                                let inv_a = 255 - a;
                                let dst = *dst_px;
                                let dr = (dst >> 16) & 0xFF;
                                let dg = (dst >> 8) & 0xFF;
                                let db = dst & 0xFF;
                                let r = (sr_u32 * a + dr * inv_a + 127) / 255;
                                let g = (sg_u32 * a + dg * inv_a + 127) / 255;
                                let b = (sb_u32 * a + db * inv_a + 127) / 255;
                                *dst_px = (r << 16) | (g << 8) | b;
                            }
                        } else {
                            for (dst_px, &mask_a) in dst_slice.iter_mut().zip(src_slice.iter()) {
                                if mask_a == 0 {
                                    continue;
                                }
                                let a = (mask_a as u32 * base_alpha + 127) / 255;
                                if a == 0 {
                                    continue;
                                }
                                let inv_a = 255 - a;
                                let dst = *dst_px;
                                let dr = (dst >> 16) & 0xFF;
                                let dg = (dst >> 8) & 0xFF;
                                let db = dst & 0xFF;
                                let r = (sr_u32 * a + dr * inv_a + 127) / 255;
                                let g = (sg_u32 * a + dg * inv_a + 127) / 255;
                                let b = (sb_u32 * a + db * inv_a + 127) / 255;
                                *dst_px = (r << 16) | (g << 8) | b;
                            }
                        }
                    }
                }
                SwashContent::SubpixelMask => {
                    if base_alpha == 0 {
                        continue;
                    }
                    for row in 0..(dst_y1 - dst_y0) as usize {
                        let dst_row_start = (dst_y0 as usize + row) * stride + dst_x0 as usize;
                        let src_row_px = (src_y_off + row) * gw_usize + src_x_off;
                        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];

                        for (col, dst_px) in dst_slice.iter_mut().enumerate() {
                            let idx = (src_row_px + col) * 4;
                            if idx + 3 >= glyph.data.len() {
                                break;
                            }
                            let mr = glyph.data[idx] as u32;
                            let mg = glyph.data[idx + 1] as u32;
                            let mb = glyph.data[idx + 2] as u32;
                            if (mr | mg | mb) == 0 {
                                continue;
                            }
                            let ar = if base_alpha >= 255 {
                                mr
                            } else {
                                (mr * base_alpha + 127) / 255
                            };
                            let ag = if base_alpha >= 255 {
                                mg
                            } else {
                                (mg * base_alpha + 127) / 255
                            };
                            let ab = if base_alpha >= 255 {
                                mb
                            } else {
                                (mb * base_alpha + 127) / 255
                            };
                            let dst = *dst_px;
                            let dr = (dst >> 16) & 0xFF;
                            let dg = (dst >> 8) & 0xFF;
                            let db = dst & 0xFF;
                            let r = (sr_u32 * ar + dr * (255 - ar) + 127) / 255;
                            let g = (sg_u32 * ag + dg * (255 - ag) + 127) / 255;
                            let b = (sb_u32 * ab + db * (255 - ab) + 127) / 255;
                            *dst_px = (r << 16) | (g << 8) | b;
                        }
                    }
                }
                SwashContent::Color => {
                    if opacity_u32 == 0 {
                        continue;
                    }
                    for row in 0..(dst_y1 - dst_y0) as usize {
                        let dst_row_start = (dst_y0 as usize + row) * stride + dst_x0 as usize;
                        let src_row_px = (src_y_off + row) * gw_usize + src_x_off;
                        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];

                        for (col, dst_px) in dst_slice.iter_mut().enumerate() {
                            let idx = (src_row_px + col) * 4;
                            if idx + 3 >= glyph.data.len() {
                                break;
                            }
                            let cr = glyph.data[idx] as u32;
                            let cg = glyph.data[idx + 1] as u32;
                            let cb = glyph.data[idx + 2] as u32;
                            let ca = glyph.data[idx + 3] as u32;
                            if ca == 0 {
                                continue;
                            }
                            let a = if opacity_u32 >= 255 {
                                ca
                            } else {
                                (ca * opacity_u32 + 127) / 255
                            };
                            if a == 0 {
                                continue;
                            }
                            if a >= 255 {
                                *dst_px = (cr << 16) | (cg << 8) | cb;
                            } else {
                                let inv_a = 255 - a;
                                let dst = *dst_px;
                                let dr = (dst >> 16) & 0xFF;
                                let dg = (dst >> 8) & 0xFF;
                                let db = dst & 0xFF;
                                let r = (cr * a + dr * inv_a + 127) / 255;
                                let g = (cg * a + dg * inv_a + 127) / 255;
                                let b = (cb * a + db * inv_a + 127) / 255;
                                *dst_px = (r << 16) | (g << 8) | b;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl TextMeasurer for CosmicTextEngine {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_text(text, font_size, max_width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::PixelBuffer;

    #[test]
    fn cosmic_engine_discovers_system_fonts_and_measures_proportionally() {
        let engine = CosmicTextEngine::shared();
        assert!(
            engine.has_system_fonts(),
            "expected native system fonts to be discovered on host"
        );
        assert!(engine.loaded_font_count() > 0);

        // Empty string -> exact 0x0.
        assert_eq!(engine.measure("", 16.0, None), (0.0, 0.0));

        // Real proportional font shaping: narrow "iiii" must be strictly narrower than wide "WWWW".
        let (w_narrow, h_narrow) = engine.measure("iiii", 20.0, None);
        let (w_wide, h_wide) = engine.measure("WWWW", 20.0, None);
        assert!(
            w_narrow > 0.0 && w_wide > w_narrow * 1.8,
            "expected proportional shaping ('iiii'={w_narrow} vs 'WWWW'={w_wide})"
        );
        assert_eq!(h_narrow, h_wide);

        // Larger font size produces strictly taller line height and wider advance.
        let (w_small, h_small) = engine.measure("Native View Engine", 12.0, None);
        let (w_large, h_large) = engine.measure("Native View Engine", 28.0, None);
        assert!(h_large > h_small * 1.8, "h_small={h_small}, h_large={h_large}");
        assert!(w_large > w_small * 1.8, "w_small={w_small}, w_large={w_large}");
    }

    #[test]
    fn cosmic_engine_wraps_under_max_width_handles_newlines_and_unicode() {
        let engine = CosmicTextEngine::shared();

        // Wrapping under a tight max_width strictly increases height and clamps width.
        let sample = "Native View Engine renders real HarfBuzz shaped proportional typography";
        let (unwrapped_w, unwrapped_h) = engine.measure(sample, 16.0, None);
        let (wrapped_w, wrapped_h) = engine.measure(sample, 16.0, Some(120.0));
        assert!(
            unwrapped_w > 200.0,
            "expected long line width > 200, got {unwrapped_w}"
        );
        assert!(
            wrapped_w <= 120.0,
            "wrapped width must respect max_width=120, got {wrapped_w}"
        );
        assert!(
            wrapped_h >= unwrapped_h * 2.0,
            "wrapping must increase height: unwrapped_h={unwrapped_h}, wrapped_h={wrapped_h}"
        );

        // Explicit newlines increase height monotonically.
        let (_, h1) = engine.measure("Line 1", 16.0, None);
        let (_, h3) = engine.measure("Line 1\nLine 2\nLine 3", 16.0, None);
        assert!(
            h3 >= h1 * 2.5,
            "expected 3 explicit lines to be ~3x single line height (h1={h1}, h3={h3})"
        );

        // Degenerate inputs (NaN, <= 0 max_width, invalid font_size) never panic or emit NaN.
        let (mw0_w, mw0_h) = engine.measure("Probe\nLine", 16.0, Some(0.0));
        assert_eq!(mw0_w, 0.0);
        assert!(mw0_h.is_finite() && mw0_h > 0.0);
        let (nan_w, nan_h) = engine.measure("Fallback", f32::NAN, Some(f32::INFINITY));
        assert!(nan_w.is_finite() && nan_h.is_finite() && nan_h > 0.0);

        // Unicode / UTF-8 / punctuation / CJK / emoji safety.
        let (uni_w, uni_h) = engine.measure("Café — naïve • 日本語 🚀", 18.0, None);
        assert!(uni_w > 20.0 && uni_h > 18.0);
    }

    #[test]
    fn cosmic_engine_rasterizes_antialiased_subpixel_edges_and_clips_strictly() {
        let engine = CosmicTextEngine::shared();
        let bg = ColorRgba::rgb(15, 23, 42);
        let fg = ColorRgba::rgb(56, 189, 248);

        let mut buf = PixelBuffer::new(320, 120);
        buf.clear(bg);

        let rect = LayoutRect {
            x: 20.0,
            y: 20.0,
            width: 240.0,
            height: 48.0,
        };
        engine.rasterize_text(&mut buf, rect, "Antialiased Orbit", 24.0, 1.0, fg, 1.0);

        let mut exact_fg_count = 0usize;
        let mut antialiased_edge_count = 0usize;
        for y in 20..68u32 {
            for x in 20..260u32 {
                let rgb = buf.get_rgb(x, y).unwrap();
                if rgb == (fg.r, fg.g, fg.b) {
                    exact_fg_count += 1;
                } else if rgb != (bg.r, bg.g, bg.b) {
                    // Must be a valid interpolated color between bg and fg!
                    assert!(
                        rgb.0 >= bg.r && rgb.0 <= fg.r,
                        "red channel out of blend range: {rgb:?}"
                    );
                    assert!(
                        rgb.1 >= bg.g && rgb.1 <= fg.g,
                        "green channel out of blend range: {rgb:?}"
                    );
                    assert!(
                        rgb.2 >= bg.b && rgb.2 <= fg.b,
                        "blue channel out of blend range: {rgb:?}"
                    );
                    antialiased_edge_count += 1;
                }
            }
        }
        assert!(
            exact_fg_count > 30,
            "expected solid core glyph pixels, got {exact_fg_count}"
        );
        assert!(
            antialiased_edge_count > 30,
            "expected smooth intermediate antialiased edge pixels (not 1-bit binary), got {antialiased_edge_count}"
        );

        // Strict clipping test: render long string into a tight 40x24 box and verify zero bleed outside!
        let mut clip_buf = PixelBuffer::new(200, 80);
        clip_buf.clear(bg);
        let clip_rect = LayoutRect {
            x: 30.0,
            y: 20.0,
            width: 40.0,
            height: 24.0,
        };
        engine.rasterize_text(
            &mut clip_buf,
            clip_rect,
            "WWWWWWWWWWWWWWWWWWWW",
            22.0,
            1.0,
            fg,
            1.0,
        );
        for y in 0..80u32 {
            for x in 0..200u32 {
                let inside = (30..70).contains(&x) && (20..44).contains(&y);
                if !inside {
                    assert_eq!(
                        clip_buf.get_rgb(x, y),
                        Some((bg.r, bg.g, bg.b)),
                        "pixel ({x}, {y}) bled outside clip rect {clip_rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn cosmic_engine_scales_cleanly_at_2x_hidpi_and_caches_consistently() {
        let engine = CosmicTextEngine::shared();
        let bg = ColorRgba::BLACK;
        let fg = ColorRgba::WHITE;
        let rect = LayoutRect {
            x: 10.0,
            y: 10.0,
            width: 180.0,
            height: 40.0,
        };

        let mut buf_1x = PixelBuffer::new(200, 60);
        buf_1x.clear(bg);
        engine.rasterize_text(&mut buf_1x, rect, "Retina 2x", 18.0, 1.0, fg, 1.0);

        let mut buf_2x_a = PixelBuffer::new(400, 120);
        buf_2x_a.clear(bg);
        engine.rasterize_text(&mut buf_2x_a, rect, "Retina 2x", 18.0, 2.0, fg, 1.0);

        // Second render hits the shaped cache and must produce bit-identical pixels.
        let mut buf_2x_b = PixelBuffer::new(400, 120);
        buf_2x_b.clear(bg);
        engine.rasterize_text(&mut buf_2x_b, rect, "Retina 2x", 18.0, 2.0, fg, 1.0);
        assert_eq!(buf_2x_a.pixels(), buf_2x_b.pixels());

        let lit_1x = buf_1x.pixels().iter().filter(|&&p| p != 0).count();
        let lit_2x = buf_2x_a.pixels().iter().filter(|&&p| p != 0).count();
        assert!(
            lit_2x > lit_1x * 2,
            "expected 2x HiDPI render to cover >2x pixels of 1x (1x={lit_1x}, 2x={lit_2x})"
        );
    }

    #[test]
    fn cosmic_engine_falls_back_cleanly_to_bitmap_font_when_no_system_fonts_exist() {
        let fallback = CosmicTextEngine::new_without_system_fonts();
        assert!(!fallback.has_system_fonts());
        assert_eq!(fallback.loaded_font_count(), 0);

        let expected = BitmapFontTextMeasurer.measure("Fallback OK", 16.0, None);
        assert_eq!(fallback.measure("Fallback OK", 16.0, None), expected);

        let mut buf = PixelBuffer::new(200, 60);
        buf.clear(ColorRgba::BLACK);
        fallback.rasterize_text(
            &mut buf,
            LayoutRect {
                x: 8.0,
                y: 8.0,
                width: 180.0,
                height: 32.0,
            },
            "Fallback OK",
            16.0,
            1.0,
            ColorRgba::WHITE,
            1.0,
        );
        let lit = buf.pixels().iter().filter(|&&p| p != 0).count();
        assert!(
            lit > 20,
            "expected bitmap fallback to render visible glyph pixels, got {lit}"
        );
    }
}
