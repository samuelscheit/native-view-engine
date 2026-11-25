//! Cached PNG and raw RGBA image decoder with bounded LRU eviction and cross-backend
//! `ImageResizeMode` placement math.
//!
//! Supports:
//! - `data:image/png;base64,...` data URIs (RFC 2083 PNG: grayscale, RGB, indexed/PLTE,
//!   grayscale+alpha, RGBA, `tRNS`, bit depths 1/2/4/8/16, all 5 scanline filters, and
//!   non-interlaced + Adam7 interlaced frames)
//! - `data:image/rgba;width=W;height=H;base64,...` raw RGBA data URIs
//! - `file://...` local PNG files
//! - Direct raw RGBA buffer insertion (`ImageCache::insert_raw_rgba`) and raw PNG byte decoding

use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use flate2::read::ZlibDecoder;
use nve_abi::ColorRgba;
use nve_components::canvas_ir::ImageResizeMode;
use nve_engine::LayoutRect;

/// Default maximum number of decoded images retained in [`ImageCache`].
pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 256;
/// Default maximum total RGBA byte footprint retained in [`ImageCache`] (64 MiB).
pub const DEFAULT_MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum supported image dimension in pixels along either axis.
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;
/// Maximum decoded RGBA byte length for a single image (64 MiB).
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum compressed IDAT payload accepted from a PNG stream (64 MiB).
const MAX_COMPRESSED_IDAT_BYTES: usize = 64 * 1024 * 1024;

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Decoded straight-alpha (unpremultiplied) RGBA8888 image buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
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
        let cx = x.clamp(0, self.width as i32 - 1) as usize;
        let cy = y.clamp(0, self.height as i32 - 1) as usize;
        let idx = (cy * self.width as usize + cx) * 4;
        ColorRgba::rgba(
            self.rgba[idx],
            self.rgba[idx + 1],
            self.rgba[idx + 2],
            self.rgba[idx + 3],
        )
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
    resize_mode: ImageResizeMode,
) -> Option<ImageDrawPlacement> {
    if dst_rect.width <= 0.0 || dst_rect.height <= 0.0 || img_w == 0 || img_h == 0 {
        return None;
    }
    let iw = img_w as f32;
    let ih = img_h as f32;
    let dw = dst_rect.width;
    let dh = dst_rect.height;

    match resize_mode {
        ImageResizeMode::Stretch => Some(ImageDrawPlacement {
            src_x: 0.0,
            src_y: 0.0,
            src_w: iw,
            src_h: ih,
            dst_x: dst_rect.x,
            dst_y: dst_rect.y,
            dst_w: dw,
            dst_h: dh,
        }),
        ImageResizeMode::Contain => {
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
        ImageResizeMode::Cover => {
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
        ImageResizeMode::Center => {
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

#[derive(Debug)]
struct CacheEntry {
    image: Arc<DecodedImage>,
    bytes: usize,
    last_used: u64,
}

#[derive(Debug)]
struct LruInner {
    max_entries: usize,
    max_bytes: usize,
    total_bytes: usize,
    tick: u64,
    entries: HashMap<String, CacheEntry>,
}

impl LruInner {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
            total_bytes: 0,
            tick: 0,
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<DecodedImage>> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        entry.last_used = tick;
        Some(Arc::clone(&entry.image))
    }

    fn insert(&mut self, key: String, image: Arc<DecodedImage>) {
        let bytes = image.byte_len();
        if let Some(prev) = self.entries.remove(&key) {
            self.total_bytes = self.total_bytes.saturating_sub(prev.bytes);
        }
        if bytes > self.max_bytes {
            // Single image exceeds entire cache byte budget; caller still receives it, but skip caching.
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.total_bytes.saturating_add(bytes) > self.max_bytes)
        {
            let lru_key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone());
            if let Some(evict_key) = lru_key {
                if let Some(evicted) = self.entries.remove(&evict_key) {
                    self.total_bytes = self.total_bytes.saturating_sub(evicted.bytes);
                }
            } else {
                break;
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.entries.insert(
            key,
            CacheEntry {
                image,
                bytes,
                last_used: self.tick,
            },
        );
    }
}

/// Thread-safe bounded LRU cache for decoded PNG and raw RGBA images.
#[derive(Debug)]
pub struct ImageCache {
    inner: Mutex<LruInner>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CACHE_ENTRIES, DEFAULT_MAX_CACHE_BYTES)
    }
}

impl ImageCache {
    /// Returns the process-wide shared [`ImageCache`].
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<ImageCache>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::default())))
    }

    /// Creates a new [`ImageCache`] bounded by both `max_entries` and `max_bytes`.
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            inner: Mutex::new(LruInner::new(max_entries, max_bytes)),
        }
    }

    /// Returns a cached image by key, updating its LRU recency.
    pub fn get(&self, key: &str) -> Option<Arc<DecodedImage>> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key)
    }

    /// Inserts a validated raw RGBA8888 buffer under `key` and returns its cached handle.
    pub fn insert_raw_rgba(
        &self,
        key: impl Into<String>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        let decoded = Arc::new(DecodedImage::from_rgba(width, height, rgba)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.into(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Decodes raw PNG bytes and caches the resulting image under `key`.
    pub fn get_or_decode_png_bytes(
        &self,
        key: &str,
        png_bytes: &[u8],
    ) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        if let Some(hit) = self.get(key) {
            return Ok(hit);
        }
        let decoded = Arc::new(decode_png(png_bytes)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.to_owned(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Resolves `src` from the cache or decodes it on demand (`data:image/png;base64,...`,
    /// `data:image/rgba;width=W;height=H;base64,...`, or `file://...`).
    pub fn get_or_decode(&self, src: &str) -> Result<Arc<DecodedImage>, ImageDecodeError> {
        if src.is_empty() {
            return Err(ImageDecodeError::InvalidDataUri);
        }
        if let Some(hit) = self.get(src) {
            return Ok(hit);
        }
        let decoded = Arc::new(decode_image_uri(src)?);
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(src.to_owned(), Arc::clone(&decoded));
        Ok(decoded)
    }

    /// Registers raw PNG bytes under a deterministic content-addressed cache key and returns the key.
    pub fn register_png_bytes(&self, png_bytes: &[u8]) -> Result<String, ImageDecodeError> {
        let key = format!("nve-mem://png/{:016x}-{}", fnv1a64(png_bytes), png_bytes.len());
        let _ = self.get_or_decode_png_bytes(&key, png_bytes)?;
        Ok(key)
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn total_bytes(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .total_bytes
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.entries.clear();
        guard.total_bytes = 0;
    }
}

#[inline]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Decodes a `data:image/png;base64,...`, `data:image/rgba;width=W;height=H;base64,...`,
/// or `file://...` image URI into a [`DecodedImage`].
pub fn decode_image_uri(src: &str) -> Result<DecodedImage, ImageDecodeError> {
    let trimmed = src.trim();
    if let Some(rest) = strip_prefix_ascii_ci(trimmed, "data:") {
        let Some((meta, payload)) = rest.split_once(',') else {
            return Err(ImageDecodeError::InvalidDataUri);
        };
        let mut parts = meta.split(';');
        let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let mut is_base64 = false;
        let mut raw_w: Option<u32> = None;
        let mut raw_h: Option<u32> = None;
        for param in parts {
            let p = param.trim();
            if p.eq_ignore_ascii_case("base64") {
                is_base64 = true;
            } else if let Some((k, v)) = p.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                if key.eq_ignore_ascii_case("width") || key.eq_ignore_ascii_case("w") {
                    raw_w = val.parse::<u32>().ok();
                } else if key.eq_ignore_ascii_case("height") || key.eq_ignore_ascii_case("h") {
                    raw_h = val.parse::<u32>().ok();
                }
            }
        }
        if !is_base64 {
            return Err(ImageDecodeError::UnsupportedFormat(
                "data URI must be base64-encoded".to_string(),
            ));
        }
        let decoded_bytes = decode_base64(payload)?;
        return match mime.as_str() {
            "image/png" => decode_png(&decoded_bytes),
            "image/rgba" => {
                let (Some(w), Some(h)) = (raw_w, raw_h) else {
                    return Err(ImageDecodeError::InvalidDataUri);
                };
                DecodedImage::from_rgba(w, h, decoded_bytes)
            }
            other => Err(ImageDecodeError::UnsupportedFormat(other.to_string())),
        };
    }

    if let Some(path_part) = strip_prefix_ascii_ci(trimmed, "file://") {
        let path_str = path_part
            .strip_prefix("localhost")
            .unwrap_or(path_part);
        let decoded_path = percent_decode(path_str);
        let path = PathBuf::from(decoded_path);
        let bytes = std::fs::read(&path)
            .map_err(|e| ImageDecodeError::Io(format!("{}: {e}", path.display())))?;
        return decode_png(&bytes);
    }

    Err(ImageDecodeError::UnsupportedFormat(trimmed.to_string()))
}

fn strip_prefix_ascii_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[inline]
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes standard or URL-safe base64 ignoring ASCII whitespace.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, ImageDecodeError> {
    let mut cleaned = Vec::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        cleaned.push(b);
    }
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }
    if cleaned.len() % 4 == 1 {
        return Err(ImageDecodeError::InvalidBase64);
    }
    let mut out = Vec::with_capacity((cleaned.len() * 3) / 4);
    let mut i = 0;
    while i < cleaned.len() {
        let rem = cleaned.len() - i;
        let b0 = cleaned[i];
        let b1 = cleaned[i + 1];
        let b2 = if rem > 2 { cleaned[i + 2] } else { b'=' };
        let b3 = if rem > 3 { cleaned[i + 3] } else { b'=' };

        let v0 = b64_val(b0).ok_or(ImageDecodeError::InvalidBase64)?;
        let v1 = b64_val(b1).ok_or(ImageDecodeError::InvalidBase64)?;
        if b2 == b'=' {
            if b3 != b'=' || i + 4 < cleaned.len() {
                return Err(ImageDecodeError::InvalidBase64);
            }
            out.push((v0 << 2) | (v1 >> 4));
            break;
        }
        let v2 = b64_val(b2).ok_or(ImageDecodeError::InvalidBase64)?;
        if b3 == b'=' {
            if i + 4 < cleaned.len() {
                return Err(ImageDecodeError::InvalidBase64);
            }
            out.push((v0 << 2) | (v1 >> 4));
            out.push((v1 << 4) | (v2 >> 2));
            break;
        }
        let v3 = b64_val(b3).ok_or(ImageDecodeError::InvalidBase64)?;
        out.push((v0 << 2) | (v1 >> 4));
        out.push((v1 << 4) | (v2 >> 2));
        out.push((v2 << 6) | v3);
        i += 4;
    }
    Ok(out)
}

#[inline]
fn b64_val(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

fn checked_rgba_len(width: u32, height: u32) -> Result<usize, ImageDecodeError> {
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

const fn make_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

const CRC32_TABLE: [u32; 256] = make_crc32_table();

fn crc32_ieee(chunk_type: &[u8], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in chunk_type.iter().chain(data.iter()) {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = CRC32_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

#[derive(Debug, Clone, Copy)]
struct PngHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

impl PngHeader {
    fn channels(self) -> usize {
        match self.color_type {
            0 => 1, // Grayscale
            2 => 3, // Truecolor RGB
            3 => 1, // Indexed-color
            4 => 2, // Grayscale + alpha
            6 => 4, // Truecolor + alpha
            _ => 0,
        }
    }

    fn bits_per_pixel(self) -> usize {
        self.channels() * (self.bit_depth as usize)
    }

    fn filter_bpp(self) -> usize {
        self.bits_per_pixel().div_ceil(8).max(1)
    }

    fn scanline_bytes(self, pass_width: u32) -> Result<usize, ImageDecodeError> {
        let bits = (pass_width as usize)
            .checked_mul(self.bits_per_pixel())
            .ok_or_else(|| ImageDecodeError::InvalidPng("scanline size overflow".to_string()))?;
        Ok(bits.div_ceil(8))
    }
}

/// Strictly bounds-checked RFC 2083 PNG decoder producing straight-alpha RGBA8888 pixels.
///
/// Never panics on truncated, corrupted, or adversarial PNG payloads.
pub fn decode_png(bytes: &[u8]) -> Result<DecodedImage, ImageDecodeError> {
    if bytes.len() < PNG_SIGNATURE.len() || bytes[..8] != PNG_SIGNATURE {
        return Err(ImageDecodeError::InvalidPng("bad PNG signature".to_string()));
    }

    let mut pos = 8usize;
    let mut header: Option<PngHeader> = None;
    let mut palette: Option<Vec<[u8; 3]>> = None;
    let mut trns: Option<Vec<u8>> = None;
    let mut idat: Vec<u8> = Vec::new();
    let mut saw_iend = false;

    while pos < bytes.len() {
        if bytes.len() - pos < 12 {
            return Err(ImageDecodeError::InvalidPng(
                "truncated PNG chunk header".to_string(),
            ));
        }
        let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
            as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        pos += 8;

        if bytes.len() - pos < len + 4 {
            return Err(ImageDecodeError::InvalidPng(
                "truncated PNG chunk payload".to_string(),
            ));
        }
        let data = &bytes[pos..pos + len];
        let expected_crc = u32::from_be_bytes([
            bytes[pos + len],
            bytes[pos + len + 1],
            bytes[pos + len + 2],
            bytes[pos + len + 3],
        ]);
        pos += len + 4;

        let actual_crc = crc32_ieee(chunk_type, data);
        if actual_crc != expected_crc {
            return Err(ImageDecodeError::InvalidPng(format!(
                "CRC mismatch in chunk {:?}",
                std::str::from_utf8(chunk_type).unwrap_or("????")
            )));
        }

        match chunk_type {
            b"IHDR" => {
                if header.is_some() || data.len() != 13 {
                    return Err(ImageDecodeError::InvalidPng("invalid IHDR chunk".to_string()));
                }
                let width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                let height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                let _ = checked_rgba_len(width, height)?;
                let bit_depth = data[8];
                let color_type = data[9];
                let compression = data[10];
                let filter = data[11];
                let interlace = data[12];

                if compression != 0 || filter != 0 || interlace > 1 {
                    return Err(ImageDecodeError::InvalidPng(
                        "unsupported PNG compression/filter/interlace method".to_string(),
                    ));
                }
                let valid_depth = match color_type {
                    0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
                    2 | 4 | 6 => matches!(bit_depth, 8 | 16),
                    3 => matches!(bit_depth, 1 | 2 | 4 | 8),
                    _ => false,
                };
                if !valid_depth {
                    return Err(ImageDecodeError::InvalidPng(format!(
                        "invalid PNG color_type={color_type} bit_depth={bit_depth}"
                    )));
                }
                header = Some(PngHeader {
                    width,
                    height,
                    bit_depth,
                    color_type,
                    interlace,
                });
            }
            b"PLTE" => {
                if header.is_none() || data.is_empty() || data.len() % 3 != 0 || data.len() > 256 * 3
                {
                    return Err(ImageDecodeError::InvalidPng("invalid PLTE chunk".to_string()));
                }
                let mut pal = Vec::with_capacity(data.len() / 3);
                for rgb in data.chunks_exact(3) {
                    pal.push([rgb[0], rgb[1], rgb[2]]);
                }
                palette = Some(pal);
            }
            b"tRNS" => {
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "tRNS chunk before IHDR".to_string(),
                    ));
                }
                trns = Some(data.to_vec());
            }
            b"IDAT" => {
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "IDAT chunk before IHDR".to_string(),
                    ));
                }
                if idat.len().saturating_add(data.len()) > MAX_COMPRESSED_IDAT_BYTES {
                    return Err(ImageDecodeError::InvalidPng(
                        "IDAT payload exceeds safety limit".to_string(),
                    ));
                }
                idat.extend_from_slice(data);
            }
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {
                // Ancillary chunks (sRGB, gAMA, pHYs, iTXt, etc.) are safely ignored.
                if header.is_none() {
                    return Err(ImageDecodeError::InvalidPng(
                        "missing IHDR as first chunk".to_string(),
                    ));
                }
            }
        }
    }

    let hdr = header.ok_or_else(|| ImageDecodeError::InvalidPng("missing IHDR".to_string()))?;
    if !saw_iend || idat.is_empty() {
        return Err(ImageDecodeError::InvalidPng(
            "missing IDAT or IEND chunk".to_string(),
        ));
    }
    if hdr.color_type == 3 && palette.is_none() {
        return Err(ImageDecodeError::InvalidPng(
            "indexed PNG missing PLTE chunk".to_string(),
        ));
    }

    let expected_raw = expected_decompressed_size(hdr)?;
    let mut inflated = Vec::with_capacity(expected_raw.min(1024 * 1024));
    let mut decoder = ZlibDecoder::new(Cursor::new(&idat)).take((expected_raw as u64) + 1);
    decoder
        .read_to_end(&mut inflated)
        .map_err(|e| ImageDecodeError::InvalidPng(format!("zlib inflate error: {e}")))?;
    if inflated.len() < expected_raw {
        return Err(ImageDecodeError::InvalidPng(format!(
            "inflated PNG data truncated: expected {expected_raw} bytes, got {}",
            inflated.len()
        )));
    }

    let out_len = checked_rgba_len(hdr.width, hdr.height)?;
    let mut rgba = vec![0u8; out_len];

    if hdr.interlace == 0 {
        decode_pass(
            hdr,
            &inflated[..expected_raw],
            palette.as_deref(),
            trns.as_deref(),
            0,
            0,
            1,
            1,
            hdr.width,
            hdr.height,
            &mut rgba,
        )?;
    } else {
        // Adam7 interlace passes: (x_start, y_start, x_step, y_step)
        const ADAM7: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let mut cursor = 0usize;
        for &(xs, ys, xstep, ystep) in &ADAM7 {
            if xs >= hdr.width || ys >= hdr.height {
                continue;
            }
            let pw = (hdr.width - xs).div_ceil(xstep);
            let ph = (hdr.height - ys).div_ceil(ystep);
            let row_bytes = hdr.scanline_bytes(pw)?;
            let pass_bytes = (ph as usize)
                .checked_mul(row_bytes + 1)
                .ok_or_else(|| ImageDecodeError::InvalidPng("pass size overflow".to_string()))?;
            let slice = inflated
                .get(cursor..cursor + pass_bytes)
                .ok_or_else(|| ImageDecodeError::InvalidPng("truncated Adam7 pass".to_string()))?;
            decode_pass(
                hdr,
                slice,
                palette.as_deref(),
                trns.as_deref(),
                xs,
                ys,
                xstep,
                ystep,
                pw,
                ph,
                &mut rgba,
            )?;
            cursor += pass_bytes;
        }
    }

    Ok(DecodedImage {
        width: hdr.width,
        height: hdr.height,
        rgba,
    })
}

fn expected_decompressed_size(hdr: PngHeader) -> Result<usize, ImageDecodeError> {
    if hdr.interlace == 0 {
        let row_bytes = hdr.scanline_bytes(hdr.width)?;
        return (hdr.height as usize)
            .checked_mul(row_bytes + 1)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()));
    }
    const ADAM7: [(u32, u32, u32, u32); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];
    let mut total = 0usize;
    for &(xs, ys, xstep, ystep) in &ADAM7 {
        if xs >= hdr.width || ys >= hdr.height {
            continue;
        }
        let pw = (hdr.width - xs).div_ceil(xstep);
        let ph = (hdr.height - ys).div_ceil(ystep);
        let row_bytes = hdr.scanline_bytes(pw)?;
        let pass_bytes = (ph as usize)
            .checked_mul(row_bytes + 1)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()))?;
        total = total
            .checked_add(pass_bytes)
            .ok_or_else(|| ImageDecodeError::InvalidPng("decompressed size overflow".to_string()))?;
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
fn decode_pass(
    hdr: PngHeader,
    pass_data: &[u8],
    palette: Option<&[[u8; 3]]>,
    trns: Option<&[u8]>,
    x_start: u32,
    y_start: u32,
    x_step: u32,
    y_step: u32,
    pass_w: u32,
    pass_h: u32,
    out_rgba: &mut [u8],
) -> Result<(), ImageDecodeError> {
    let row_bytes = hdr.scanline_bytes(pass_w)?;
    let bpp = hdr.filter_bpp();
    let mut prev_row = vec![0u8; row_bytes];
    let mut cur_row = vec![0u8; row_bytes];

    for py in 0..pass_h {
        let src_off = (py as usize) * (row_bytes + 1);
        let filter = pass_data[src_off];
        let filtered = &pass_data[src_off + 1..src_off + 1 + row_bytes];
        unfilter_scanline(filter, filtered, &prev_row, &mut cur_row, bpp)?;

        let dst_y = (y_start + py * y_step) as usize;
        let row_dst_base = dst_y * (hdr.width as usize);
        expand_row_to_rgba(
            hdr,
            &cur_row,
            pass_w,
            palette,
            trns,
            x_start,
            x_step,
            row_dst_base,
            out_rgba,
        )?;

        std::mem::swap(&mut prev_row, &mut cur_row);
    }
    Ok(())
}

fn unfilter_scanline(
    filter: u8,
    src: &[u8],
    prev: &[u8],
    dst: &mut [u8],
    bpp: usize,
) -> Result<(), ImageDecodeError> {
    match filter {
        0 => dst.copy_from_slice(src),
        1 => {
            for i in 0..src.len() {
                let left = if i >= bpp { dst[i - bpp] } else { 0 };
                dst[i] = src[i].wrapping_add(left);
            }
        }
        2 => {
            for i in 0..src.len() {
                dst[i] = src[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..src.len() {
                let left = if i >= bpp { dst[i - bpp] as u16 } else { 0 };
                let up = prev[i] as u16;
                dst[i] = src[i].wrapping_add(((left + up) >> 1) as u8);
            }
        }
        4 => {
            for i in 0..src.len() {
                let a = if i >= bpp { dst[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                dst[i] = src[i].wrapping_add(paeth_predictor(a, b, c));
            }
        }
        other => {
            return Err(ImageDecodeError::InvalidPng(format!(
                "unknown PNG scanline filter {other}"
            )));
        }
    }
    Ok(())
}

#[inline]
fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i32;
    let b = b as i32;
    let c = c as i32;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

#[inline]
fn read_packed_sample(row: &[u8], bit_depth: u8, index: usize) -> u8 {
    match bit_depth {
        1 => (row[index >> 3] >> (7 - (index & 7))) & 1,
        2 => (row[index >> 2] >> (6 - ((index & 3) * 2))) & 3,
        4 => (row[index >> 1] >> (4 - ((index & 1) * 4))) & 15,
        8 => row[index],
        _ => 0,
    }
}

#[inline]
fn scale_subbyte_gray(v: u8, bit_depth: u8) -> u8 {
    match bit_depth {
        1 => v * 255,
        2 => v * 85,
        4 => v * 17,
        _ => v,
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_row_to_rgba(
    hdr: PngHeader,
    row: &[u8],
    pass_w: u32,
    palette: Option<&[[u8; 3]]>,
    trns: Option<&[u8]>,
    x_start: u32,
    x_step: u32,
    row_dst_base: usize,
    out_rgba: &mut [u8],
) -> Result<(), ImageDecodeError> {
    let trns_gray16 = trns
        .filter(|t| t.len() >= 2)
        .map(|t| u16::from_be_bytes([t[0], t[1]]));
    let trns_rgb16 = trns.filter(|t| t.len() >= 6).map(|t| {
        (
            u16::from_be_bytes([t[0], t[1]]),
            u16::from_be_bytes([t[2], t[3]]),
            u16::from_be_bytes([t[4], t[5]]),
        )
    });

    for px in 0..pass_w as usize {
        let (r, g, b, a) = match (hdr.color_type, hdr.bit_depth) {
            (0, d @ (1 | 2 | 4 | 8)) => {
                let raw = read_packed_sample(row, d, px);
                let g8 = scale_subbyte_gray(raw, d);
                let alpha = if trns_gray16 == Some(raw as u16) {
                    0
                } else {
                    255
                };
                (g8, g8, g8, alpha)
            }
            (0, 16) => {
                let raw = u16::from_be_bytes([row[px * 2], row[px * 2 + 1]]);
                let g8 = (raw >> 8) as u8;
                let alpha = if trns_gray16 == Some(raw) { 0 } else { 255 };
                (g8, g8, g8, alpha)
            }
            (2, 8) => {
                let i = px * 3;
                let (r, g, b) = (row[i], row[i + 1], row[i + 2]);
                let alpha = if trns_rgb16 == Some((r as u16, g as u16, b as u16)) {
                    0
                } else {
                    255
                };
                (r, g, b, alpha)
            }
            (2, 16) => {
                let i = px * 6;
                let r16 = u16::from_be_bytes([row[i], row[i + 1]]);
                let g16 = u16::from_be_bytes([row[i + 2], row[i + 3]]);
                let b16 = u16::from_be_bytes([row[i + 4], row[i + 5]]);
                let alpha = if trns_rgb16 == Some((r16, g16, b16)) {
                    0
                } else {
                    255
                };
                ((r16 >> 8) as u8, (g16 >> 8) as u8, (b16 >> 8) as u8, alpha)
            }
            (3, d @ (1 | 2 | 4 | 8)) => {
                let idx = read_packed_sample(row, d, px) as usize;
                let pal = palette.ok_or_else(|| {
                    ImageDecodeError::InvalidPng("missing PLTE for indexed PNG".to_string())
                })?;
                let rgb = *pal.get(idx).ok_or_else(|| {
                    ImageDecodeError::InvalidPng(format!("palette index {idx} out of bounds"))
                })?;
                let alpha = trns.and_then(|t| t.get(idx).copied()).unwrap_or(255);
                (rgb[0], rgb[1], rgb[2], alpha)
            }
            (4, 8) => {
                let i = px * 2;
                let g8 = row[i];
                (g8, g8, g8, row[i + 1])
            }
            (4, 16) => {
                let i = px * 4;
                let g8 = row[i];
                let a8 = row[i + 2];
                (g8, g8, g8, a8)
            }
            (6, 8) => {
                let i = px * 4;
                (row[i], row[i + 1], row[i + 2], row[i + 3])
            }
            (6, 16) => {
                let i = px * 8;
                (row[i], row[i + 2], row[i + 4], row[i + 6])
            }
            _ => {
                return Err(ImageDecodeError::InvalidPng(
                    "unsupported color_type/bit_depth combination".to_string(),
                ));
            }
        };

        let dst_x = (x_start + (px as u32) * x_step) as usize;
        let out_idx = (row_dst_base + dst_x) * 4;
        out_rgba[out_idx] = r;
        out_rgba[out_idx + 1] = g;
        out_rgba[out_idx + 2] = b;
        out_rgba[out_idx + 3] = a;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AVATAR_MAYA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEWBxd4fKHggFOB1JD4MAKjRfBnTmv6zAAAAAElFTkSuQmCC";
    const AVATAR_LIAM: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAg9gVSsGEXB8u5fKHggFOB1JD4MAGcqfXUz+ORAAAAAAElFTkSuQmCC";
    const AVATAR_SARA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhWPTGsGEWB0tE4FDwQCvA6Eh8GAE5RZ/09pCRNAAAAAElFTkSuQmCC";
    const AVATAR_DEV: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAHElEQVR42mOQ07T+jw8zEK1gyuIdKHggFJDtCwAdM3cRKMfnvgAAAABJRU5ErkJggg==";
    const AVATAR_ARIA: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAhXKWasGEXB13ncKHggFOB1JD4MAEs7b4ERxIxeAAAAAElFTkSuQmCC";
    const AVATAR_NOAH: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAgerzysGEVB7eU/KHggFOB1JD4MAL/Ggglv5kNyAAAAAElFTkSuQmCC";
    const AVATAR_YOU: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAYAAADED76LAAAAJElEQVR42mOQ07T+jw8zgAh+cS2sGEXBj19/UPBAKMDrSHwYAFnZgxk5UYNxAAAAAElFTkSuQmCC";

    #[test]
    fn decodes_all_studio_workspace_png_avatars_to_8x8_rgba() {
        let cache = ImageCache::default();
        for uri in [
            AVATAR_MAYA,
            AVATAR_LIAM,
            AVATAR_SARA,
            AVATAR_DEV,
            AVATAR_ARIA,
            AVATAR_NOAH,
            AVATAR_YOU,
        ] {
            let img = cache.get_or_decode(uri).expect("valid Studio avatar PNG");
            assert_eq!(img.width, 8);
            assert_eq!(img.height, 8);
            assert_eq!(img.rgba.len(), 8 * 8 * 4);
            let opaque_non_black = img
                .rgba
                .chunks_exact(4)
                .filter(|px| px[3] > 0 && (px[0] > 0 || px[1] > 0 || px[2] > 0))
                .count();
            assert!(
                opaque_non_black >= 16,
                "expected rich avatar pixels, got {opaque_non_black}"
            );
            // Second lookup hits LRU cache with exact Arc pointer identity.
            let hit = cache.get_or_decode(uri).expect("cached avatar");
            assert!(Arc::ptr_eq(&img, &hit));
        }
        assert_eq!(cache.len(), 7);
    }

    #[test]
    fn decodes_file_uri_and_raw_rgba_and_evicts_lru() {
        let raw_png = decode_base64(
            AVATAR_MAYA
                .strip_prefix("data:image/png;base64,")
                .unwrap(),
        )
        .unwrap();
        let tmp_path = std::env::temp_dir().join(format!(
            "nve_image_cache_test_{}.png",
            std::process::id()
        ));
        std::fs::write(&tmp_path, &raw_png).unwrap();
        let file_uri = format!("file://{}", tmp_path.display());

        let cache = ImageCache::new(2, 1024);
        let from_file = cache.get_or_decode(&file_uri).expect("decode file:// PNG");
        assert_eq!((from_file.width, from_file.height), (8, 8));

        let raw = cache
            .insert_raw_rgba("raw://2x2", 2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255])
            .expect("insert raw RGBA");
        assert_eq!(raw.pixel_clamped(0, 0), ColorRgba::rgb(255, 0, 0));
        assert_eq!(raw.pixel_clamped(1, 0), ColorRgba::rgb(0, 255, 0));

        // Touch file_uri so raw://2x2 becomes least-recently-used.
        assert!(cache.get(&file_uri).is_some());
        let _third = cache.get_or_decode(AVATAR_SARA).expect("third image");
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&file_uri).is_some());
        assert!(cache.get(AVATAR_SARA).is_some());
        assert!(cache.get("raw://2x2").is_none());

        let _ = std::fs::remove_file(&tmp_path);
    }

    #[test]
    fn rejects_corrupt_and_truncated_pngs_without_panicking() {
        let valid_png = decode_base64(
            AVATAR_MAYA
                .strip_prefix("data:image/png;base64,")
                .unwrap(),
        )
        .unwrap();
        // Every prefix truncation must fail cleanly.
        for cut in 0..valid_png.len() {
            assert!(decode_png(&valid_png[..cut]).is_err());
        }
        // Flipping a byte inside chunk data triggers CRC failure.
        let mut flipped = valid_png.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 0xFF;
        assert!(decode_png(&flipped).is_err());

        assert!(decode_image_uri("").is_err());
        assert!(decode_image_uri("data:image/png;base64,!!!not-base64!!!").is_err());
        assert!(decode_image_uri("https://example.com/nope.png").is_err());
        assert!(DecodedImage::from_rgba(0, 10, vec![]).is_err());
        assert!(DecodedImage::from_rgba(2, 2, vec![0; 4]).is_err());
    }

    #[test]
    fn computes_exact_placement_for_all_four_resize_modes() {
        let dst = LayoutRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 50.0,
        };
        // 40x40 square into 100x50 wide rect:
        let stretch = compute_image_placement(dst, 40, 40, ImageResizeMode::Stretch).unwrap();
        assert_eq!((stretch.dst_x, stretch.dst_y, stretch.dst_w, stretch.dst_h), (10.0, 20.0, 100.0, 50.0));
        assert_eq!((stretch.src_x, stretch.src_y, stretch.src_w, stretch.src_h), (0.0, 0.0, 40.0, 40.0));

        let contain = compute_image_placement(dst, 40, 40, ImageResizeMode::Contain).unwrap();
        assert_eq!((contain.dst_x, contain.dst_y, contain.dst_w, contain.dst_h), (35.0, 20.0, 50.0, 50.0));

        let cover = compute_image_placement(dst, 40, 40, ImageResizeMode::Cover).unwrap();
        assert_eq!((cover.dst_x, cover.dst_y, cover.dst_w, cover.dst_h), (10.0, 20.0, 100.0, 50.0));
        assert!((cover.src_y - 10.0).abs() < 1e-4 && (cover.src_h - 20.0).abs() < 1e-4);

        let center = compute_image_placement(dst, 40, 40, ImageResizeMode::Center).unwrap();
        assert_eq!((center.dst_x, center.dst_y, center.dst_w, center.dst_h), (40.0, 25.0, 40.0, 40.0));

        // Oversized image in Center mode crops cleanly to dst_rect bounds.
        let center_crop = compute_image_placement(dst, 200, 100, ImageResizeMode::Center).unwrap();
        assert_eq!((center_crop.dst_x, center_crop.dst_y, center_crop.dst_w, center_crop.dst_h), (10.0, 20.0, 100.0, 50.0));
        assert_eq!((center_crop.src_x, center_crop.src_y, center_crop.src_w, center_crop.src_h), (50.0, 25.0, 100.0, 50.0));
    }
}
