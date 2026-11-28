//! Contiguous `0x00RRGGBB` pixel buffer for the NVE software rasterizer.

use nve_abi::ColorRgba;

/// Contiguous `0x00RRGGBB` pixel buffer suitable for direct blitting into `softbuffer`,
/// with an optional alpha channel (`has_alpha == true` only for offscreen RGBA compositing).
#[derive(Debug, Clone)]
pub struct PixelBuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
    alpha: Vec<u8>,
    has_alpha: bool,
}

impl PixelBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            pixels: vec![0; len],
            alpha: Vec::new(),
            has_alpha: false,
        }
    }

    /// Creates an alpha-tracking transparent RGBA pixel buffer cleared to `(0, 0, 0, 0)`.
    pub fn new_transparent_rgba(width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            pixels: vec![0; len],
            alpha: vec![0; len],
            has_alpha: true,
        }
    }

    /// Resizes (reusing existing capacity) and clears this buffer to transparent `(0, 0, 0, 0)`.
    pub fn reset_transparent_rgba(&mut self, width: u32, height: u32) {
        let len = (width as usize).saturating_mul(height as usize);
        self.width = width;
        self.height = height;
        self.has_alpha = true;
        self.pixels.clear();
        self.pixels.resize(len, 0);
        self.alpha.clear();
        self.alpha.resize(len, 0);
    }

    /// Reclaims excess backing capacity after transient oversized draws (e.g. a 4Kx4K plugin).
    pub fn shrink_if_capacity_exceeds(&mut self, max_pixels: usize) {
        if self.pixels.capacity() > max_pixels {
            self.width = 0;
            self.height = 0;
            self.pixels = Vec::new();
            self.alpha = Vec::new();
        }
    }

    /// Writes contiguous unpremultiplied `RGBA8888` bytes into `out` (reusing `out`'s capacity).
    pub fn write_rgba8_into(&self, out: &mut Vec<u8>) {
        let len = self.pixels.len();
        out.clear();
        out.reserve(len * 4);
        if self.has_alpha {
            for (&px, &a) in self.pixels.iter().zip(self.alpha.iter()) {
                let (r, g, b) = Self::unpack_rgb(px);
                out.extend_from_slice(&[r, g, b, a]);
            }
        } else {
            for &px in &self.pixels {
                let (r, g, b) = Self::unpack_rgb(px);
                out.extend_from_slice(&[r, g, b, 255]);
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        let len = (width as usize).saturating_mul(height as usize);
        self.pixels.resize(len, 0);
        if self.has_alpha {
            self.alpha.resize(len, 0);
        }
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[inline]
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    #[inline]
    pub fn pixel_capacity(&self) -> usize {
        self.pixels.capacity()
    }

    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    /// Returns `true` when this buffer tracks per-pixel alpha (isolated offscreen RGBA surface).
    #[inline]
    pub const fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    #[inline]
    pub const fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
        ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }

    #[inline]
    pub const fn unpack_rgb(pixel: u32) -> (u8, u8, u8) {
        (
            ((pixel >> 16) & 0xFF) as u8,
            ((pixel >> 8) & 0xFF) as u8,
            (pixel & 0xFF) as u8,
        )
    }

    pub fn clear(&mut self, color: ColorRgba) {
        let packed = Self::pack_rgb(color.r, color.g, color.b);
        self.pixels.fill(packed);
        if self.has_alpha {
            self.alpha.fill(color.a);
        }
    }

    /// Overwrites pixels (and alpha, when alpha-tracking is active) inside `[x0, x1) x [y0, y1)`.
    pub fn fill_rect_rgba(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: ColorRgba) {
        let sx0 = x0.clamp(0, self.width as i32) as usize;
        let sx1 = x1.clamp(0, self.width as i32) as usize;
        let sy0 = y0.clamp(0, self.height as i32) as usize;
        let sy1 = y1.clamp(0, self.height as i32) as usize;
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }
        let packed = Self::pack_rgb(color.r, color.g, color.b);
        let stride = self.width as usize;
        let has_alpha = self.has_alpha;
        for py in sy0..sy1 {
            let row = py * stride;
            self.pixels[row + sx0..row + sx1].fill(packed);
            if has_alpha {
                self.alpha[row + sx0..row + sx1].fill(color.a);
            }
        }
    }

    #[inline]
    pub fn get_rgb(&self, x: u32, y: u32) -> Option<(u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let idx = (y as usize) * (self.width as usize) + (x as usize);
        self.pixels.get(idx).copied().map(Self::unpack_rgb)
    }

    #[inline]
    pub fn get_alpha(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        if !self.has_alpha {
            return Some(255);
        }
        let idx = (y as usize) * (self.width as usize) + (x as usize);
        self.alpha.get(idx).copied()
    }

    #[inline]
    pub fn set_rgb(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x < self.width && y < self.height {
            let idx = (y as usize) * (self.width as usize) + (x as usize);
            self.pixels[idx] = Self::pack_rgb(r, g, b);
            if self.has_alpha {
                self.alpha[idx] = 255;
            }
        }
    }

    /// Src-over blends `color` with extra coverage `coverage` (`0.0..=1.0`) at `(x, y)`.
    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: ColorRgba, coverage: f32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let eff_a = ((color.a as f32) * coverage.clamp(0.0, 1.0)).round() as u32;
        if eff_a == 0 {
            return;
        }
        let idx = (y as usize) * (self.width as usize) + (x as usize);
        if eff_a >= 255 {
            self.pixels[idx] = Self::pack_rgb(color.r, color.g, color.b);
            if self.has_alpha {
                self.alpha[idx] = 255;
            }
            return;
        }
        if self.has_alpha {
            let dst_a = self.alpha[idx] as u32;
            let inv_a = 255 - eff_a;
            let dst_w = (dst_a * inv_a + 127) / 255;
            let out_a = eff_a + dst_w; // >= 1 since eff_a >= 1
            let dst = self.pixels[idx];
            let dr = (dst >> 16) & 0xFF;
            let dg = (dst >> 8) & 0xFF;
            let db = dst & 0xFF;
            let r = ((color.r as u32 * eff_a + dr * dst_w + out_a / 2) / out_a).min(255);
            let g = ((color.g as u32 * eff_a + dg * dst_w + out_a / 2) / out_a).min(255);
            let b = ((color.b as u32 * eff_a + db * dst_w + out_a / 2) / out_a).min(255);
            self.pixels[idx] = (r << 16) | (g << 8) | b;
            self.alpha[idx] = out_a.min(255) as u8;
            return;
        }
        let dst = self.pixels[idx];
        let dr = (dst >> 16) & 0xFF;
        let dg = (dst >> 8) & 0xFF;
        let db = dst & 0xFF;
        let inv_a = 255 - eff_a;
        let r = (color.r as u32 * eff_a + dr * inv_a + 127) / 255;
        let g = (color.g as u32 * eff_a + dg * inv_a + 127) / 255;
        let b = (color.b as u32 * eff_a + db * inv_a + 127) / 255;
        self.pixels[idx] = (r << 16) | (g << 8) | b;
    }

    /// Fast horizontal span fill for opaque or semi-transparent interior rows.
    #[inline]
    pub(crate) fn blend_span(&mut self, y: i32, x0: i32, x1: i32, color: ColorRgba, opacity: f32) {
        if y < 0 || y >= self.height as i32 || x1 <= x0 {
            return;
        }
        let start = x0.max(0) as usize;
        let end = x1.min(self.width as i32).max(0) as usize;
        if start >= end {
            return;
        }
        let eff_a = ((color.a as f32) * opacity.clamp(0.0, 1.0)).round() as u32;
        if eff_a == 0 {
            return;
        }
        let row_offset = (y as usize) * (self.width as usize);
        if self.has_alpha {
            if eff_a >= 255 {
                self.pixels[row_offset + start..row_offset + end]
                    .fill(Self::pack_rgb(color.r, color.g, color.b));
                self.alpha[row_offset + start..row_offset + end].fill(255);
            } else {
                let sr = color.r as u32 * eff_a;
                let sg = color.g as u32 * eff_a;
                let sb = color.b as u32 * eff_a;
                let inv_a = 255 - eff_a;
                for idx in (row_offset + start)..(row_offset + end) {
                    let dst_a = self.alpha[idx] as u32;
                    let dst_w = (dst_a * inv_a + 127) / 255;
                    let out_a = eff_a + dst_w;
                    let dst = self.pixels[idx];
                    let dr = (dst >> 16) & 0xFF;
                    let dg = (dst >> 8) & 0xFF;
                    let db = dst & 0xFF;
                    let r = ((sr + dr * dst_w + out_a / 2) / out_a).min(255);
                    let g = ((sg + dg * dst_w + out_a / 2) / out_a).min(255);
                    let b = ((sb + db * dst_w + out_a / 2) / out_a).min(255);
                    self.pixels[idx] = (r << 16) | (g << 8) | b;
                    self.alpha[idx] = out_a.min(255) as u8;
                }
            }
            return;
        }
        let slice = &mut self.pixels[row_offset + start..row_offset + end];
        if eff_a >= 255 {
            slice.fill(Self::pack_rgb(color.r, color.g, color.b));
        } else {
            let sr = color.r as u32 * eff_a;
            let sg = color.g as u32 * eff_a;
            let sb = color.b as u32 * eff_a;
            let inv_a = 255 - eff_a;
            for px in slice.iter_mut() {
                let dst = *px;
                let dr = (dst >> 16) & 0xFF;
                let dg = (dst >> 8) & 0xFF;
                let db = dst & 0xFF;
                let r = (sr + dr * inv_a + 127) / 255;
                let g = (sg + dg * inv_a + 127) / 255;
                let b = (sb + db * inv_a + 127) / 255;
                *px = (r << 16) | (g << 8) | b;
            }
        }
    }

    /// Src-over composites an alpha-tracking `src` buffer onto `self` at `(dst_x, dst_y)`,
    /// honoring `clip` (including rounded clips) and `opacity` with zero heap allocations.
    pub(crate) fn composite_rgba_over(
        &mut self,
        src: &PixelBuffer,
        dst_x: i32,
        dst_y: i32,
        clip: &super::clip::ClipStack,
        opacity: f32,
    ) {
        if !src.has_alpha || !opacity.is_finite() || opacity <= 0.0 || clip.is_empty() {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let [cx0, cy0, cx1, cy1] = clip.bounds();
        let x0 = dst_x.max(cx0).max(0);
        let y0 = dst_y.max(cy0).max(0);
        let x1 = (dst_x + src.width as i32).min(cx1).min(self.width as i32);
        let y1 = (dst_y + src.height as i32).min(cy1).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let rounded_clip = clip.has_rounded();
        let opaque_pass = opacity >= 1.0 && !rounded_clip;
        let src_stride = src.width as usize;
        let dst_stride = self.width as usize;

        for py in y0..y1 {
            let sy = (py - dst_y) as usize;
            let src_row = sy * src_stride;
            let dst_row = (py as usize) * dst_stride;
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let sx = (px - dst_x) as usize;
                let sidx = src_row + sx;
                let sa = src.alpha[sidx];
                if sa == 0 {
                    continue;
                }
                let rgb = src.pixels[sidx];
                if opaque_pass && sa == 255 {
                    let didx = dst_row + (px as usize);
                    self.pixels[didx] = rgb;
                    if self.has_alpha {
                        self.alpha[didx] = 255;
                    }
                    continue;
                }
                let cov = if rounded_clip {
                    clip.rounded_coverage(px as f32 + 0.5, fy) * opacity
                } else {
                    opacity
                };
                if cov > 0.0 {
                    let (r, g, b) = Self::unpack_rgb(rgb);
                    self.blend_pixel(px, py, ColorRgba::rgba(r, g, b, sa), cov);
                }
            }
        }
    }
}
