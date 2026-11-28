//! Cached PNG and raw RGBA image decoder with bounded O(1) `SlabLru` eviction and cross-backend
//! `ImageResizeMode` placement math.
//!
//! Supports:
//! - `data:image/png;base64,...` data URIs (RFC 2083 PNG: grayscale, RGB, indexed/PLTE,
//!   grayscale+alpha, RGBA, `tRNS`, bit depths 1/2/4/8/16, all 5 scanline filters, and
//!   non-interlaced + Adam7 interlaced frames)
//! - `data:image/rgba;width=W;height=H;base64,...` raw RGBA data URIs
//! - `file://...` local PNG files
//! - Direct raw RGBA buffer insertion (`ImageCache::insert_raw_rgba`) and raw PNG byte decoding

mod data_uri;
mod lru;
mod png;
mod png_rows;
#[cfg(test)]
mod tests;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_abi::ColorRgba;
use nve_tables::tables::ImageFit;
use nve_engine::LayoutRect;

pub use data_uri::{decode_base64, decode_image_uri};
pub use lru::ImageCache;
pub use png::{decode_png, encode_rgb_png};

/// Default maximum number of decoded images retained in [`ImageCache`].
pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 256;
/// Upper bound on `ImageCache::new(max_entries, ...)` slot count so the eager per-slot atomic
/// liveness/CLOCK tables remain bounded even if a caller passes `usize::MAX`.
pub const MAX_IMAGE_CACHE_SLOTS: usize = 65_536;
/// Default maximum total RGBA byte footprint retained in [`ImageCache`] (64 MiB).
pub const DEFAULT_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum supported image dimension in pixels along either axis.
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;
/// Maximum decoded RGBA byte length for a single image (64 MiB).
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

static NEXT_DECODED_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

#[inline]
pub(crate) fn next_decoded_image_id() -> u64 {
    NEXT_DECODED_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Sealed dense slot index inside [`ImageCache`] (`[INV:typed-domain-handles]`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageSlot(u32);

impl ImageSlot {
    #[inline]
    pub(self) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[inline]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Compact `Copy` handle referencing a decoded image slot in [`ImageCache`] without pinning
/// `Arc<DecodedImage>`. Fields and constructor are private to `image_cache` so external code
/// cannot forge handles; warm frames validate liveness in one lock-free `Acquire` load on
/// the per-slot atomic `decoded_id` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageSlotHandle {
    slot: ImageSlot,
    decoded_id: u64,
}

impl ImageSlotHandle {
    #[inline]
    pub(self) const fn new(slot: ImageSlot, decoded_id: u64) -> Self {
        Self { slot, decoded_id }
    }

    /// Returns the sealed [`ImageSlot`] index for this handle.
    #[inline]
    pub const fn slot(self) -> ImageSlot {
        self.slot
    }

    /// Returns the monotonic `DecodedImage` allocation identity bound to this handle.
    #[inline]
    pub const fn decoded_id(self) -> u64 {
        self.decoded_id
    }
}

#[cfg(test)]
static SHARED_IMAGE_CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_shared_image_cache() -> std::sync::MutexGuard<'static, ()> {
    SHARED_IMAGE_CACHE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Decoded straight-alpha (unpremultiplied) RGBA8888 image buffer.
#[derive(Debug)]
pub struct DecodedImage {
    /// Process-wide monotonic allocation identity used as an ABA-free, zero-hashing cache key
    /// for GPU texture caches (`SkiaImageCache`). Private so external code cannot forge or alias
    /// identities across distinct pixel buffers.
    id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Clone for DecodedImage {
    fn clone(&self) -> Self {
        Self {
            id: next_decoded_image_id(),
            width: self.width,
            height: self.height,
            rgba: self.rgba.clone(),
        }
    }
}

impl PartialEq for DecodedImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.rgba == other.rgba
    }
}

impl Eq for DecodedImage {}

impl DecodedImage {
    /// Returns the process-wide unique monotonic allocation ID for this decoded buffer.
    #[inline]
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Creates a validated [`DecodedImage`] from raw straight-alpha RGBA8888 bytes.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ImageDecodeError> {
        let expected = checked_rgba_len(width, height)?;
        if rgba.len() != expected {
            return Err(ImageDecodeError::RawBufferLengthMismatch {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            id: next_decoded_image_id(),
            width,
            height,
            rgba,
        })
    }

    /// Returns the total byte footprint of the decoded RGBA pixels.
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.rgba.len()
    }

    /// Reads the RGBA pixel at integer coordinates `(x, y)` (clamped to image bounds).
    #[inline]
    pub fn pixel_clamped(&self, x: i32, y: i32) -> ColorRgba {
        if self.width == 0 || self.height == 0 {
            return ColorRgba::TRANSPARENT;
        }
        let cx = x.clamp(0, (self.width - 1).min(i32::MAX as u32) as i32) as usize;
        let cy = y.clamp(0, (self.height - 1).min(i32::MAX as u32) as i32) as usize;
        let Some(idx) = cy
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(cx))
            .and_then(|px| px.checked_mul(4))
        else {
            return ColorRgba::TRANSPARENT;
        };
        let Some(end) = idx.checked_add(4) else {
            return ColorRgba::TRANSPARENT;
        };
        let Some(px) = self.rgba.get(idx..end) else {
            return ColorRgba::TRANSPARENT;
        };
        ColorRgba::rgba(px[0], px[1], px[2], px[3])
    }

    /// Nearest-neighbor sample at pixel-space coordinates `(sx, sy)` (pixel centers at `i + 0.5`).
    #[inline]
    pub fn sample_nearest(&self, sx: f32, sy: f32) -> ColorRgba {
        self.pixel_clamped(sx.floor() as i32, sy.floor() as i32)
    }

    /// Bilinear sample at pixel-space coordinates `(sx, sy)` using premultiplied-alpha
    /// interpolation so transparent borders never bleed dark fringes.
    #[inline]
    pub fn sample_bilinear(&self, sx: f32, sy: f32) -> ColorRgba {
        let px = sx - 0.5;
        let py = sy - 0.5;
        let x0 = px.floor() as i32;
        let y0 = py.floor() as i32;
        let fx = (px - x0 as f32).clamp(0.0, 1.0);
        let fy = (py - y0 as f32).clamp(0.0, 1.0);

        let c00 = self.pixel_clamped(x0, y0);
        let c10 = self.pixel_clamped(x0 + 1, y0);
        let c01 = self.pixel_clamped(x0, y0 + 1);
        let c11 = self.pixel_clamped(x0 + 1, y0 + 1);

        let w00 = (1.0 - fx) * (1.0 - fy);
        let w10 = fx * (1.0 - fy);
        let w01 = (1.0 - fx) * fy;
        let w11 = fx * fy;

        let a00 = c00.a as f32;
        let a10 = c10.a as f32;
        let a01 = c01.a as f32;
        let a11 = c11.a as f32;

        let a = a00 * w00 + a10 * w10 + a01 * w01 + a11 * w11;
        if a <= 0.5 {
            return ColorRgba::TRANSPARENT;
        }

        let inv_a = 1.0 / a;
        let r = ((c00.r as f32 * a00 * w00
            + c10.r as f32 * a10 * w10
            + c01.r as f32 * a01 * w01
            + c11.r as f32 * a11 * w11)
            * inv_a)
            .round()
            .clamp(0.0, 255.0) as u8;
        let g = ((c00.g as f32 * a00 * w00
            + c10.g as f32 * a10 * w10
            + c01.g as f32 * a01 * w01
            + c11.g as f32 * a11 * w11)
            * inv_a)
            .round()
            .clamp(0.0, 255.0) as u8;
        let b = ((c00.b as f32 * a00 * w00
            + c10.b as f32 * a10 * w10
            + c01.b as f32 * a01 * w01
            + c11.b as f32 * a11 * w11)
            * inv_a)
            .round()
            .clamp(0.0, 255.0) as u8;

        ColorRgba::rgba(r, g, b, a.round().clamp(0.0, 255.0) as u8)
    }
}

/// Errors returned while decoding or registering an image in [`ImageCache`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDecodeError {
    InvalidDataUri,
    InvalidBase64,
    UnsupportedFormat(String),
    Io(String),
    InvalidPng(String),
    DimensionsOutOfBounds { width: u32, height: u32 },
    RawBufferLengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for ImageDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDataUri => write!(f, "malformed data: URI"),
            Self::InvalidBase64 => write!(f, "invalid base64 payload"),
            Self::UnsupportedFormat(msg) => write!(f, "unsupported image format: {msg}"),
            Self::Io(msg) => write!(f, "image I/O error: {msg}"),
            Self::InvalidPng(msg) => write!(f, "invalid PNG image: {msg}"),
            Self::DimensionsOutOfBounds { width, height } => {
                write!(f, "image dimensions {width}x{height} out of bounds")
            }
            Self::RawBufferLengthMismatch { expected, actual } => write!(
                f,
                "raw RGBA buffer length mismatch: expected {expected} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for ImageDecodeError {}

/// Source-to-destination mapping produced by [`compute_image_placement`] for a given
/// [`ImageResizeMode`], shared by both the software rasterizer and the Skia GPU renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDrawPlacement {
    pub src_x: f32,
    pub src_y: f32,
    pub src_w: f32,
    pub src_h: f32,
    pub dst_x: f32,
    pub dst_y: f32,
    pub dst_w: f32,
    pub dst_h: f32,
}

/// Computes exact source crop and destination rect for rendering an `(img_w, img_h)` image into
/// `dst_rect` under `resize_mode`. The resulting destination rectangle is guaranteed to lie inside
/// `dst_rect`.
pub fn compute_image_placement(
    dst_rect: LayoutRect,
    img_w: u32,
    img_h: u32,
    fit: ImageFit,
) -> Option<ImageDrawPlacement> {
    if dst_rect.width <= 0.0 || dst_rect.height <= 0.0 || img_w == 0 || img_h == 0 {
        return None;
    }
    let iw = img_w as f32;
    let ih = img_h as f32;
    let dw = dst_rect.width;
    let dh = dst_rect.height;

    let effective_fit = match fit {
        ImageFit::ScaleDown => {
            if iw > dw || ih > dh {
                ImageFit::Contain
            } else {
                ImageFit::None
            }
        }
        other => other,
    };

    match effective_fit {
        ImageFit::Fill => Some(ImageDrawPlacement {
            src_x: 0.0,
            src_y: 0.0,
            src_w: iw,
            src_h: ih,
            dst_x: dst_rect.x,
            dst_y: dst_rect.y,
            dst_w: dw,
            dst_h: dh,
        }),
        ImageFit::Contain => {
            let scale = (dw / iw).min(dh / ih);
            let out_w = (iw * scale).max(0.0);
            let out_h = (ih * scale).max(0.0);
            Some(ImageDrawPlacement {
                src_x: 0.0,
                src_y: 0.0,
                src_w: iw,
                src_h: ih,
                dst_x: dst_rect.x + (dw - out_w) * 0.5,
                dst_y: dst_rect.y + (dh - out_h) * 0.5,
                dst_w: out_w,
                dst_h: out_h,
            })
        }
        ImageFit::Cover => {
            let scale = (dw / iw).max(dh / ih);
            let src_w = (dw / scale).clamp(0.0, iw);
            let src_h = (dh / scale).clamp(0.0, ih);
            Some(ImageDrawPlacement {
                src_x: (iw - src_w) * 0.5,
                src_y: (ih - src_h) * 0.5,
                src_w,
                src_h,
                dst_x: dst_rect.x,
                dst_y: dst_rect.y,
                dst_w: dw,
                dst_h: dh,
            })
        }
        ImageFit::None | ImageFit::ScaleDown => {
            let unclipped_x = dst_rect.x + (dw - iw) * 0.5;
            let unclipped_y = dst_rect.y + (dh - ih) * 0.5;
            let clip_x0 = unclipped_x.max(dst_rect.x);
            let clip_y0 = unclipped_y.max(dst_rect.y);
            let clip_x1 = (unclipped_x + iw).min(dst_rect.x + dw);
            let clip_y1 = (unclipped_y + ih).min(dst_rect.y + dh);
            let vis_w = clip_x1 - clip_x0;
            let vis_h = clip_y1 - clip_y0;
            if vis_w <= 0.0 || vis_h <= 0.0 {
                return None;
            }
            Some(ImageDrawPlacement {
                src_x: (clip_x0 - unclipped_x).clamp(0.0, iw),
                src_y: (clip_y0 - unclipped_y).clamp(0.0, ih),
                src_w: vis_w.min(iw),
                src_h: vis_h.min(ih),
                dst_x: clip_x0,
                dst_y: clip_y0,
                dst_w: vis_w,
                dst_h: vis_h,
            })
        }
    }
}

pub(crate) fn checked_rgba_len(width: u32, height: u32) -> Result<usize, ImageDecodeError> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(ImageDecodeError::DimensionsOutOfBounds { width, height });
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageDecodeError::DimensionsOutOfBounds { width, height })?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or(ImageDecodeError::DimensionsOutOfBounds { width, height })?;
    if bytes > MAX_IMAGE_BYTES {
        return Err(ImageDecodeError::DimensionsOutOfBounds { width, height });
    }
    Ok(bytes)
}
