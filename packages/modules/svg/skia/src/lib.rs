//! Skia / software rasterizer adapter for `nve-svg` (`SkiaTableSystem`).
//!
//! Owns the analytical 4-subscanline antialiased SVG rasterizer and byte-budgeted LRU [`SvgCache`]
//! so `nve-backend-skia` carries zero SVG code or dependencies.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod rasterize;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use nve_abi::ColorRgba;
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx, SkiaTableSystem};
use nve_store::{NodeId, NodeStore, NveTable, TableSlot};
use nve_svg::{Svg, SvgPicture, SvgPictureView};

pub use rasterize::rasterize_svg_picture_view;

/// Default maximum cached SVG bitmaps in [`SvgCache`].
pub const DEFAULT_SVG_CACHE_MAX_ENTRIES: usize = 512;
/// Default maximum byte budget for cached premultiplied RGBA SVG bitmaps (8 MiB).
pub const DEFAULT_SVG_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_SVG_RASTER_DIM: u32 = 2048;

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

/// Cached premultiplied RGBA8888 rasterized SVG bitmap.
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

struct CacheEntry {
    bitmap: Arc<CachedSvgBitmap>,
    /// Canonical flat words the bitmap was rendered from: a hit must match them exactly, so a
    /// 64-bit `content_hash` collision can never serve another picture's pixels.
    words: Arc<[u32]>,
    last_used: u64,
}

struct SvgCacheInner {
    entries: HashMap<SvgCacheKey, CacheEntry>,
    lru: BTreeMap<u64, SvgCacheKey>,
    clock: u64,
    total_bytes: usize,
    hits: u64,
    misses: u64,
}

/// Thread-safe byte-budgeted LRU cache for rasterized [`SvgPicture`] bitmaps.
pub struct SvgCache {
    max_entries: usize,
    max_bytes: usize,
    inner: Mutex<SvgCacheInner>,
}

impl SvgCache {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(4096),
            inner: Mutex::new(SvgCacheInner {
                entries: HashMap::new(),
                lru: BTreeMap::new(),
                clock: 0,
                total_bytes: 0,
                hits: 0,
                misses: 0,
            }),
        }
    }

    pub fn global() -> &'static Self {
        static GLOBAL: OnceLock<SvgCache> = OnceLock::new();
        GLOBAL.get_or_init(|| Self::new(DEFAULT_SVG_CACHE_MAX_ENTRIES, DEFAULT_SVG_CACHE_MAX_BYTES))
    }

    /// Returns the cached bitmap of `picture` at `(px_w, px_h, tint)`, rasterizing on a miss.
    pub fn get_or_rasterize_view(
        &self,
        picture: SvgPictureView<'_>,
        px_w: u32,
        px_h: u32,
        tint: Option<ColorRgba>,
    ) -> Arc<CachedSvgBitmap> {
        let w = px_w.clamp(1, MAX_SVG_RASTER_DIM);
        let h = px_h.clamp(1, MAX_SVG_RASTER_DIM);
        let key = SvgCacheKey::new(picture.content_hash(), w, h, tint);
        let words = picture.words();

        {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let clock = inner.clock.wrapping_add(1);
            let hit = inner
                .entries
                .get_mut(&key)
                .filter(|e| *e.words == *words)
                .map(|e| (std::mem::replace(&mut e.last_used, clock), Arc::clone(&e.bitmap)));
            if let Some((prev_tick, bitmap)) = hit {
                inner.lru.remove(&prev_tick);
                inner.clock = clock;
                inner.lru.insert(clock, key);
                inner.hits += 1;
                return bitmap;
            }
        }

        let bitmap = Arc::new(rasterize_svg_picture_view(picture, w, h, tint));
        let bytes = bitmap.byte_len();

        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.misses += 1;
        if let Some(existing) = inner.entries.get(&key) {
            // Same picture raced in, or a hash collision: never overwrite, serve our own render.
            return if *existing.words == *words {
                Arc::clone(&existing.bitmap)
            } else {
                bitmap
            };
        }
        while !inner.entries.is_empty()
            && (inner.entries.len() >= self.max_entries
                || inner.total_bytes.saturating_add(bytes) > self.max_bytes)
        {
            let Some((&oldest_tick, &oldest_key)) = inner.lru.iter().next() else {
                break;
            };
            inner.lru.remove(&oldest_tick);
            if let Some(evicted) = inner.entries.remove(&oldest_key) {
                inner.total_bytes = inner.total_bytes.saturating_sub(evicted.bitmap.byte_len());
            }
        }
        inner.clock = inner.clock.wrapping_add(1);
        let tick = inner.clock;
        inner.total_bytes = inner.total_bytes.saturating_add(bytes);
        inner.lru.insert(tick, key);
        inner.entries.insert(
            key,
            CacheEntry {
                bitmap: Arc::clone(&bitmap),
                words: Arc::from(words),
                last_used: tick,
            },
        );
        bitmap
    }
}

/// Blits `bitmap` (premultiplied RGBA8888) onto `buf` at `(ox, oy)` with optional horizontal mirror
/// and optional physical clip rectangle `[x0, y0, x1, y1]`.
pub fn blit_svg_bitmap_clipped(
    buf: &mut PixelBuffer,
    bitmap: &CachedSvgBitmap,
    ox: i32,
    oy: i32,
    opacity: f32,
    mirror_x: bool,
    clip: Option<[i32; 4]>,
) {
    let op = if opacity.is_finite() {
        opacity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if op <= 0.0 || bitmap.width == 0 || bitmap.height == 0 {
        return;
    }
    let bw = buf.width() as i32;
    let bh = buf.height() as i32;
    let [cx0, cy0, cx1, cy1] = clip.unwrap_or([0, 0, bw, bh]);
    let min_x = cx0.clamp(0, bw);
    let max_x = cx1.clamp(0, bw);
    let min_y = cy0.clamp(0, bh);
    let max_y = cy1.clamp(0, bh);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let w = bitmap.width as i32;
    let h = bitmap.height as i32;
    for sy in 0..h {
        let dy = oy + sy;
        if dy < min_y || dy >= max_y {
            continue;
        }
        for sx in 0..w {
            let dx = ox + if mirror_x { w - 1 - sx } else { sx };
            if dx < min_x || dx >= max_x {
                continue;
            }
            let idx = ((sy as usize) * (bitmap.width as usize) + (sx as usize)) * 4;
            let pa = bitmap.premul_rgba[idx + 3];
            if pa == 0 {
                continue;
            }
            let alpha = f32::from(pa) / 255.0;
            let unpremul = |c: u8| -> u8 { ((f32::from(c) / alpha).round()).clamp(0.0, 255.0) as u8 };
            let color = ColorRgba::rgba(
                unpremul(bitmap.premul_rgba[idx]),
                unpremul(bitmap.premul_rgba[idx + 1]),
                unpremul(bitmap.premul_rgba[idx + 2]),
                255,
            );
            buf.blend_pixel(dx, dy, color, alpha * op);
        }
    }
}

/// Blits `bitmap` (premultiplied RGBA8888) onto `buf` at `(ox, oy)` with optional horizontal mirror.
pub fn blit_svg_bitmap(
    buf: &mut PixelBuffer,
    bitmap: &CachedSvgBitmap,
    ox: i32,
    oy: i32,
    opacity: f32,
    mirror_x: bool,
) {
    blit_svg_bitmap_clipped(buf, bitmap, ox, oy, opacity, mirror_x, None);
}

/// `SkiaTableSystem` plugin painting [`Svg`] (`nve.svg`) nodes onto `PixelBuffer`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SvgSkiaSystem;

impl SkiaTableSystem for SvgSkiaSystem {
    fn slot(&self) -> TableSlot {
        Svg::slot()
    }

    fn paint_node(&self, ctx: &mut SkiaSystemPaintCtx<'_>, store: &NodeStore, node: NodeId) {
        let Some(row) = store.try_row::<Svg>(node) else {
            return;
        };
        let Some(pic) = store.resource::<SvgPicture>(row.picture()) else {
            return;
        };
        if !ctx.rect.width.is_finite()
            || !ctx.rect.height.is_finite()
            || ctx.rect.width <= 0.0
            || ctx.rect.height <= 0.0
        {
            return;
        }
        let px_w = ctx.rect.width.round().max(1.0) as u32;
        let px_h = ctx.rect.height.round().max(1.0) as u32;
        // Preserve multi-color SVG fills/strokes when `row.tint()` is `None` (matching Web).
        let tint = row.tint().map(|c| ColorRgba::from_u32(c.0));
        let bmp = SvgCache::global().get_or_rasterize_view(pic, px_w, px_h, tint);
        let clip = ctx.clip_rect.map(|r| {
            [
                r.x.floor() as i32,
                r.y.floor() as i32,
                (r.x + r.width).ceil() as i32,
                (r.y + r.height).ceil() as i32,
            ]
        });
        blit_svg_bitmap_clipped(
            ctx.buffer,
            &bmp,
            ctx.rect.x.round() as i32,
            ctx.rect.y.round() as i32,
            ctx.opacity,
            row.mirror_directional(),
            clip,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_engine::LayoutRect;
    use nve_abi::EventBatchBuf;
    use nve_store::{ChangeSet, Rgba};

    #[test]
    fn svg_skia_system_rasterizes_and_caches_vector_shape() {
        let pic = SvgPicture::from_paths(24.0, 24.0, &["M2 2 H22 V22 H2 Z"]);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Svg>(n)
            .picture(Some(pic))
            .tint(Some(Rgba::rgb(255, 0, 0)));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut buf = PixelBuffer::new(32, 32);
        buf.clear(ColorRgba::BLACK);
        let mut events = EventBatchBuf::new();
        let mut ctx = SkiaSystemPaintCtx {
            buffer: &mut buf,
            rect: LayoutRect {
                x: 4.0,
                y: 4.0,
                width: 24.0,
                height: 24.0,
            },
            clip_rect: None,
            scale_factor: 1.0,
            opacity: 1.0,
            inherited_text_color: Rgba::WHITE,
            events: &mut events,
        };
        SvgSkiaSystem.paint_node(&mut ctx, &store, n);
        // Center pixel of filled red square should be red, outside stays black.
        assert_eq!(buf.get_rgb(16, 16), Some((255, 0, 0)));
        assert_eq!(buf.get_rgb(0, 0), Some((0, 0, 0)));
    }
}
