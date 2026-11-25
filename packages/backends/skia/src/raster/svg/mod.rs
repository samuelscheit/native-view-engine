//! Thread-safe byte-budgeted LRU [`SvgCache`] keyed by `(content_hash, px_w, px_h, tint)`
//! and premultiplied RGBA blitting for the pure-Rust NVE software rasterizer.

mod rasterize;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use nve_abi::ColorRgba;
use nve_components::svg::SvgPicture;
use nve_components::transform::affine_inverse_map_point;
use nve_engine::LayoutRect;

use super::clip::ClipStack;
use super::transform::transformed_aabb;
use super::PixelBuffer;

pub use rasterize::rasterize_svg_picture;

/// Default maximum cached SVG bitmaps in [`SvgCache`].
pub const DEFAULT_SVG_CACHE_MAX_ENTRIES: usize = 512;

/// Default maximum byte budget for cached premultiplied RGBA SVG bitmaps (8 MiB).
pub const DEFAULT_SVG_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Maximum raster dimension along either axis to bound memory for adversarial rects.
pub(super) const MAX_SVG_RASTER_DIM: u32 = 2048;

/// Deterministic cache key for a rasterized [`SvgPicture`] at a specific pixel size and tint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SvgCacheKey {
    pub content_hash: u64,
    pub px_w: u32,
    pub px_h: u32,
    pub tint: Option<[u8; 4]>,
}

impl SvgCacheKey {
    #[inline]
    pub fn new(content_hash: u64, px_w: u32, px_h: u32, tint: Option<ColorRgba>) -> Self {
        Self {
            content_hash,
            px_w,
            px_h,
            tint: tint.map(|c| [c.r, c.g, c.b, c.a]),
        }
    }
}

/// Premultiplied RGBA8888 bitmap produced by the software SVG rasterizer.
#[derive(Debug, Clone)]
pub struct CachedSvgBitmap {
    pub width: u32,
    pub height: u32,
    pub premul_rgba: Vec<u8>,
}

impl CachedSvgBitmap {
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.premul_rgba.len()
    }
}

/// Snapshot of [`SvgCache`] telemetry counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SvgCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub total_bytes: usize,
}

#[derive(Debug)]
struct SvgCacheEntry {
    bitmap: Arc<CachedSvgBitmap>,
    bytes: usize,
    last_used: u64,
}

#[derive(Debug)]
struct SvgLruInner {
    max_entries: usize,
    max_bytes: usize,
    total_bytes: usize,
    tick: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    entries: HashMap<SvgCacheKey, SvgCacheEntry>,
    recency: BTreeMap<u64, SvgCacheKey>,
}

impl SvgLruInner {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            total_bytes: 0,
            tick: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            entries: HashMap::new(),
            recency: BTreeMap::new(),
        }
    }

    #[inline]
    fn next_tick(&mut self) -> u64 {
        self.tick = self.tick.wrapping_add(1);
        self.tick
    }

    fn get(&mut self, key: &SvgCacheKey) -> Option<Arc<CachedSvgBitmap>> {
        let new_tick = self.next_tick();
        let entry = self.entries.get_mut(key)?;
        let old_tick = entry.last_used;
        entry.last_used = new_tick;
        let bmp = Arc::clone(&entry.bitmap);
        if let Some(k) = self.recency.remove(&old_tick) {
            self.recency.insert(new_tick, k);
        }
        self.hits = self.hits.saturating_add(1);
        Some(bmp)
    }

    fn insert(&mut self, key: SvgCacheKey, bitmap: Arc<CachedSvgBitmap>) {
        let bytes = bitmap.byte_len();
        if let Some(prev) = self.entries.remove(&key) {
            self.recency.remove(&prev.last_used);
            self.total_bytes = self.total_bytes.saturating_sub(prev.bytes);
        }
        if bytes > self.max_bytes {
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.total_bytes.saturating_add(bytes) > self.max_bytes)
        {
            let Some((_, evict_key)) = self.recency.pop_first() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&evict_key) {
                self.total_bytes = self.total_bytes.saturating_sub(evicted.bytes);
                self.evictions = self.evictions.saturating_add(1);
            }
        }
        let tick = self.next_tick();
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.recency.insert(tick, key);
        self.entries.insert(
            key,
            SvgCacheEntry {
                bitmap,
                bytes,
                last_used: tick,
            },
        );
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.total_bytes = 0;
        self.hits = 0;
        self.misses = 0;
        self.evictions = 0;
    }
}

/// Thread-safe byte-budgeted LRU cache for rasterized [`SvgPicture`] bitmaps.
#[derive(Debug)]
pub struct SvgCache {
    inner: Mutex<SvgLruInner>,
}

impl Default for SvgCache {
    fn default() -> Self {
        Self::new(DEFAULT_SVG_CACHE_MAX_ENTRIES, DEFAULT_SVG_CACHE_MAX_BYTES)
    }
}

impl SvgCache {
    /// Returns the process-wide shared [`SvgCache`].
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<SvgCache>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::default())))
    }

    /// Creates an isolated [`SvgCache`] bounded by `max_entries` and `max_bytes`.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(SvgLruInner::new(max_entries, max_bytes)),
        }
    }

    /// Looks up `(content_hash, px_w, px_h, tint)` in O(1) or rasterizes and caches the bitmap.
    pub fn get_or_rasterize(
        &self,
        picture: &SvgPicture,
        px_w: u32,
        px_h: u32,
        tint: Option<ColorRgba>,
    ) -> Arc<CachedSvgBitmap> {
        let w = px_w.clamp(1, MAX_SVG_RASTER_DIM);
        let h = px_h.clamp(1, MAX_SVG_RASTER_DIM);
        let key = SvgCacheKey::new(picture.content_hash, w, h, tint);
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(hit) = guard.get(&key) {
                return hit;
            }
            guard.misses = guard.misses.saturating_add(1);
        }
        let bitmap = Arc::new(rasterize_svg_picture(picture, w, h, tint));
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key, Arc::clone(&bitmap));
        bitmap
    }

    pub fn stats(&self) -> SvgCacheStats {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        SvgCacheStats {
            hits: guard.hits,
            misses: guard.misses,
            evictions: guard.evictions,
            entries: guard.entries.len(),
            total_bytes: guard.total_bytes,
        }
    }

    pub fn len(&self) -> usize {
        self.stats().entries
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_bytes(&self) -> usize {
        self.stats().total_bytes
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }
}

/// Draws a cached vector [`SvgPicture`] into `buf` at physical `rect` using the shared [`SvgCache`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_svg(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    transform: Option<&[f32; 6]>,
    rect: LayoutRect,
    picture: &SvgPicture,
    tint: Option<ColorRgba>,
    opacity: f32,
) {
    draw_svg_with_cache(
        buf,
        clip,
        transform,
        rect,
        picture,
        tint,
        opacity,
        SvgCache::shared().as_ref(),
    );
}

/// Draws a cached vector [`SvgPicture`] directly into `buf` using an explicit [`SvgCache`].
pub fn draw_svg_to_buffer(
    buf: &mut PixelBuffer,
    rect: LayoutRect,
    picture: &SvgPicture,
    tint: Option<ColorRgba>,
    opacity: f32,
    cache: &SvgCache,
) {
    let clip = ClipStack::new(buf.width(), buf.height());
    draw_svg_with_cache(buf, &clip, None, rect, picture, tint, opacity, cache);
}

/// Draws a cached vector [`SvgPicture`] into `buf` using an explicit [`SvgCache`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_svg_with_cache(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    transform: Option<&[f32; 6]>,
    rect: LayoutRect,
    picture: &SvgPicture,
    tint: Option<ColorRgba>,
    opacity: f32,
    cache: &SvgCache,
) {
    if rect.width <= 0.0
        || rect.height <= 0.0
        || opacity <= 0.0
        || clip.is_empty()
        || picture.shapes.is_empty()
    {
        return;
    }
    let px_w = (rect.width.ceil() as u32).clamp(1, MAX_SVG_RASTER_DIM);
    let px_h = (rect.height.ceil() as u32).clamp(1, MAX_SVG_RASTER_DIM);
    let bmp = cache.get_or_rasterize(picture, px_w, px_h, tint);
    blit_premul_bitmap(buf, clip, transform, rect, &bmp, opacity);
}

fn blit_premul_bitmap(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    transform: Option<&[f32; 6]>,
    rect: LayoutRect,
    bmp: &CachedSvgBitmap,
    opacity: f32,
) {
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let rounded_clip = clip.has_rounded();
    let (min_x, min_y, max_x, max_y) = if let Some(m) = transform {
        let (bx0, by0, bx1, by1) = transformed_aabb(m, rect, 1.0);
        (
            (bx0.floor() as i32).clamp(cx0, cx1),
            (by0.floor() as i32).clamp(cy0, cy1),
            (bx1.ceil() as i32).clamp(cx0, cx1),
            (by1.ceil() as i32).clamp(cy0, cy1),
        )
    } else {
        (
            (rect.x.floor() as i32).clamp(cx0, cx1),
            (rect.y.floor() as i32).clamp(cy0, cy1),
            ((rect.x + rect.width).ceil() as i32).clamp(cx0, cx1),
            ((rect.y + rect.height).ceil() as i32).clamp(cy0, cy1),
        )
    };
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let inv_w = bmp.width as f32 / rect.width.max(1e-6);
    let inv_h = bmp.height as f32 / rect.height.max(1e-6);
    let bw = bmp.width as i32;
    let bh = bmp.height as i32;
    let buf_w = buf.width() as usize;

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let (lx, ly) = if let Some(m) = transform {
                let Some(pt) = affine_inverse_map_point(m, fx, fy) else {
                    return;
                };
                pt
            } else {
                (fx, fy)
            };
            let sx = ((lx - rect.x) * inv_w).floor() as i32;
            let sy = ((ly - rect.y) * inv_h).floor() as i32;
            if sx < 0 || sy < 0 || sx >= bw || sy >= bh {
                continue;
            }
            let sidx = ((sy as usize) * (bmp.width as usize) + (sx as usize)) * 4;
            let sr = bmp.premul_rgba[sidx];
            let sg = bmp.premul_rgba[sidx + 1];
            let sb = bmp.premul_rgba[sidx + 2];
            let sa = bmp.premul_rgba[sidx + 3];
            if sa == 0 {
                continue;
            }
            let cov = (clip_cov * opacity).clamp(0.0, 1.0);
            let didx = (py as usize) * buf_w + (px as usize);
            let dst = buf.pixels()[didx];
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let eff_a = ((sa as f32) * cov).round() as u32;
            if eff_a == 0 {
                continue;
            }
            let eff_r = ((sr as f32) * cov).round() as u32;
            let eff_g = ((sg as f32) * cov).round() as u32;
            let eff_b = ((sb as f32) * cov).round() as u32;
            let inv_a = 255 - eff_a.min(255);
            let out_r = (eff_r + (dr * inv_a + 127) / 255).min(255);
            let out_g = (eff_g + (dg * inv_a + 127) / 255).min(255);
            let out_b = (eff_b + (db * inv_a + 127) / 255).min(255);
            buf.pixels_mut()[didx] = (out_r << 16) | (out_g << 8) | out_b;
        }
    }
}
