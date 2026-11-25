//! Pure-Rust anti-aliased software rasterizer for NVE `DisplayList`.
//!
//! Renders `FillRoundedRect`, `StrokeRoundedRect`, `DrawText` (using `CosmicTextEngine`
//! native system font shaping & antialiased glyph rasterization, with automatic fallback to
//! the embedded 5x7 bitmap font), and `SaveOpacity`/`Restore` into a `0x00RRGGBB`
//! `PixelBuffer` directly compatible with `softbuffer`.

use nve_abi::ColorRgba;
use nve_engine::{LayoutRect, TextMeasurer};

use crate::font::{glyph_bitmap, GLYPH_HEIGHT, GLYPH_WIDTH};
use crate::text::CosmicTextEngine;
use crate::{DisplayList, PaintOp};

/// Contiguous `0x00RRGGBB` pixel buffer suitable for direct blitting into `softbuffer`.
#[derive(Debug, Clone)]
pub struct PixelBuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl PixelBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            width,
            height,
            pixels: vec![0; len],
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
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
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
    pub fn set_rgb(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x < self.width && y < self.height {
            let idx = (y as usize) * (self.width as usize) + (x as usize);
            self.pixels[idx] = Self::pack_rgb(r, g, b);
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
    fn blend_span(&mut self, y: i32, x0: i32, x1: i32, color: ColorRgba, opacity: f32) {
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
}

/// Signed distance to a rounded rectangle centered at `(cx, cy)` with half-size `(hx, hy)`
/// and corner radius `r`. Negative inside, positive outside.
#[inline]
fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let out_x = qx.max(0.0);
    let out_y = qy.max(0.0);
    let outside = (out_x * out_x + out_y * out_y).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

fn fill_rounded_rect(
    buf: &mut PixelBuffer,
    rect: LayoutRect,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color.a == 0 || opacity <= 0.0 {
        return;
    }
    let w = buf.width() as i32;
    let h = buf.height() as i32;
    let min_x = (rect.x.floor() as i32 - 1).clamp(0, w);
    let max_x = ((rect.x + rect.width).ceil() as i32 + 1).clamp(0, w);
    let min_y = (rect.y.floor() as i32 - 1).clamp(0, h);
    let max_y = ((rect.y + rect.height).ceil() as i32 + 1).clamp(0, h);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let r = radius.clamp(0.0, hx.min(hy));

    // Inner safe bounds guaranteed to have coverage == 1.0 (avoiding SDF per pixel).
    let inner_x0 = ((rect.x + r).ceil() as i32 + 1).max(min_x);
    let inner_x1 = ((rect.x + rect.width - r).floor() as i32 - 1).min(max_x);
    let inner_y0 = ((rect.y + 1.0).ceil() as i32).max(min_y);
    let inner_y1 = ((rect.y + rect.height - 1.0).floor() as i32).min(max_y);
    let full_band_y0 = ((rect.y + r).ceil() as i32 + 1).max(min_y);
    let full_band_y1 = ((rect.y + rect.height - r).floor() as i32 - 1).min(max_y);
    let edge_x0 = ((rect.x + 1.0).ceil() as i32).max(min_x);
    let edge_x1 = ((rect.x + rect.width - 1.0).floor() as i32).min(max_x);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let (span_l, span_r) = if py >= full_band_y0 && py < full_band_y1 && edge_x0 < edge_x1 {
            (edge_x0, edge_x1)
        } else if py >= inner_y0 && py < inner_y1 && inner_x0 < inner_x1 {
            (inner_x0, inner_x1)
        } else {
            (max_x, max_x)
        };

        // Left anti-aliased fringe.
        for px in min_x..span_l {
            let sdf = rounded_rect_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
        // Solid interior span.
        if span_l < span_r {
            buf.blend_span(py, span_l, span_r, color, opacity);
        }
        // Right anti-aliased fringe.
        for px in span_r..max_x {
            let sdf = rounded_rect_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

fn stroke_rounded_rect(
    buf: &mut PixelBuffer,
    rect: LayoutRect,
    radius: f32,
    stroke_width: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if rect.width <= 0.0
        || rect.height <= 0.0
        || stroke_width <= 0.0
        || color.a == 0
        || opacity <= 0.0
    {
        return;
    }
    let hw = (stroke_width * 0.5).max(0.5);
    let pad = hw.ceil() as i32 + 1;
    let w = buf.width() as i32;
    let h = buf.height() as i32;
    let min_x = (rect.x.floor() as i32 - pad).clamp(0, w);
    let max_x = ((rect.x + rect.width).ceil() as i32 + pad).clamp(0, w);
    let min_y = (rect.y.floor() as i32 - pad).clamp(0, h);
    let max_y = ((rect.y + rect.height).ceil() as i32 + pad).clamp(0, h);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let r = radius.clamp(0.0, hx.min(hy));

    // Hollow interior bounds that are guaranteed well inside `-hw - 1.0` so we skip them.
    let margin = hw + r + 1.5;
    let skip_x0 = ((rect.x + margin).ceil() as i32).max(min_x);
    let skip_x1 = ((rect.x + rect.width - margin).floor() as i32).min(max_x);
    let skip_y0 = ((rect.y + margin).ceil() as i32).max(min_y);
    let skip_y1 = ((rect.y + rect.height - margin).floor() as i32).min(max_y);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let can_skip_mid = py >= skip_y0 && py < skip_y1 && skip_x0 < skip_x1;
        let mut px = min_x;
        while px < max_x {
            if can_skip_mid && px == skip_x0 {
                px = skip_x1;
                continue;
            }
            let sdf = rounded_rect_sdf(px as f32 + 0.5, fy, cx, cy, hx, hy, r);
            let band_dist = sdf.abs() - hw;
            let cov = (0.5 - band_dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
            px += 1;
        }
    }
}

pub(crate) fn draw_bitmap_text(
    buf: &mut PixelBuffer,
    rect: LayoutRect,
    text: &str,
    font_size: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if text.is_empty() || font_size <= 0.0 || color.a == 0 || opacity <= 0.0 {
        return;
    }

    // Share exact metrics with `BitmapFontTextMeasurer` so layout and rasterization stay in lockstep.
    let metrics = crate::font::bitmap_font_metrics(font_size);
    let glyph_h = metrics.glyph_h;
    let glyph_w = metrics.glyph_w;
    let advance_x = metrics.advance_x;
    let line_height = metrics.line_height;

    // Vertically center single-line text gently within its computed box.
    let vertical_pad = if !text.contains('\n') && rect.height > glyph_h {
        ((rect.height - glyph_h) * 0.5).min(font_size * 0.25)
    } else {
        0.0
    };

    let buf_w = buf.width() as i32;
    let buf_h = buf.height() as i32;
    let clip_x0 = (rect.x.floor() as i32).max(0);
    let clip_y0 = (rect.y.floor() as i32).max(0);
    let clip_x1 = ((rect.x + rect.width).ceil() as i32).min(buf_w);
    let clip_y1 = ((rect.y + rect.height).ceil() as i32).min(buf_h);
    if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
        return;
    }

    let mut cursor_x = rect.x;
    let mut cursor_y = rect.y + vertical_pad;

    for ch in text.chars() {
        if ch == '\n' {
            cursor_x = rect.x;
            cursor_y += line_height;
            continue;
        }
        if ch == ' ' {
            cursor_x += advance_x;
            continue;
        }

        let bitmap = glyph_bitmap(ch);
        let x0 = (cursor_x.floor() as i32).max(clip_x0);
        let x1 = ((cursor_x + glyph_w).ceil() as i32).min(clip_x1);
        let y0 = (cursor_y.floor() as i32).max(clip_y0);
        let y1 = ((cursor_y + glyph_h).ceil() as i32).min(clip_y1);

        if x0 < x1 && y0 < y1 {
            let inv_w = GLYPH_WIDTH as f32 / glyph_w;
            let inv_h = GLYPH_HEIGHT as f32 / glyph_h;
            for py in y0..y1 {
                let v = ((py as f32 + 0.5) - cursor_y) * inv_h;
                let row_idx = v.floor() as i32;
                if row_idx < 0 || row_idx >= GLYPH_HEIGHT as i32 {
                    continue;
                }
                let row_bits = bitmap[row_idx as usize];
                if row_bits == 0 {
                    continue;
                }
                for px in x0..x1 {
                    let u = ((px as f32 + 0.5) - cursor_x) * inv_w;
                    let col_idx = u.floor() as i32;
                    if col_idx < 0 || col_idx >= GLYPH_WIDTH as i32 {
                        continue;
                    }
                    let bit = (row_bits >> (GLYPH_WIDTH as i32 - 1 - col_idx)) & 1;
                    if bit != 0 {
                        buf.blend_pixel(px, py, color, opacity);
                    }
                }
            }
        }

        cursor_x += advance_x;
    }
}

/// Rasterizes a retained `DisplayList` into `buffer` using the shared [`CosmicTextEngine`],
/// scaling logical points by `scale_factor`.
pub fn rasterize_display_list(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
) {
    let engine = CosmicTextEngine::shared();
    rasterize_display_list_with_text_engine(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        engine.as_ref(),
    );
}

/// Rasterizes a retained `DisplayList` into `buffer` using an explicit [`CosmicTextEngine`].
pub fn rasterize_display_list_with_text_engine(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: &CosmicTextEngine,
) {
    rasterize_display_list_inner(
        display_list,
        buffer,
        scale_factor,
        clear_color,
        Some(text_engine),
    );
}

/// Rasterizes a retained `DisplayList` into `buffer` using the deterministic embedded 5x7
/// bitmap font (useful for headless pixel regression tests independent of installed OS fonts).
pub fn rasterize_display_list_bitmap(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
) {
    rasterize_display_list_inner(display_list, buffer, scale_factor, clear_color, None);
}

fn rasterize_display_list_inner(
    display_list: &DisplayList,
    buffer: &mut PixelBuffer,
    scale_factor: f32,
    clear_color: ColorRgba,
    text_engine: Option<&CosmicTextEngine>,
) {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    buffer.clear(clear_color);

    let mut opacity_stack: Vec<f32> = Vec::with_capacity(8);
    opacity_stack.push(1.0);

    for op in &display_list.ops {
        let cur_opacity = *opacity_stack.last().unwrap_or(&1.0);
        match op {
            PaintOp::SaveOpacity(alpha) => {
                opacity_stack.push(cur_opacity * alpha.clamp(0.0, 1.0));
            }
            PaintOp::Restore => {
                if opacity_stack.len() > 1 {
                    opacity_stack.pop();
                }
            }
            PaintOp::FillRoundedRect {
                rect,
                radius,
                color,
            } => {
                fill_rounded_rect(
                    buffer,
                    scale_rect(*rect, scale),
                    radius * scale,
                    *color,
                    cur_opacity,
                );
            }
            PaintOp::StrokeRoundedRect {
                rect,
                radius,
                width,
                color,
            } => {
                stroke_rounded_rect(
                    buffer,
                    scale_rect(*rect, scale),
                    radius * scale,
                    width * scale,
                    *color,
                    cur_opacity,
                );
            }
            PaintOp::DrawText {
                rect,
                text,
                font_size,
                color,
            } => {
                if let Some(engine) = text_engine {
                    engine.rasterize_text(
                        buffer,
                        *rect,
                        text,
                        *font_size,
                        scale,
                        *color,
                        cur_opacity,
                    );
                } else {
                    draw_bitmap_text(
                        buffer,
                        scale_rect(*rect, scale),
                        text,
                        font_size * scale,
                        *color,
                        cur_opacity,
                    );
                }
            }
            PaintOp::CustomSkiaCanvas { rect, .. } => {
                // Draw a subtle dashed-style border placeholder in software mode.
                stroke_rounded_rect(
                    buffer,
                    scale_rect(*rect, scale),
                    4.0 * scale,
                    1.0 * scale,
                    ColorRgba::rgba(148, 163, 184, 160),
                    cur_opacity,
                );
            }
        }
    }
}

/// Renders a compact floating diagnostic pill at the bottom-right corner of the buffer.
pub fn draw_hud_banner(buffer: &mut PixelBuffer, scale_factor: f32, text: &str) {
    if text.is_empty() || buffer.width() < 64 || buffer.height() < 32 {
        return;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let text_engine = CosmicTextEngine::shared();
    let logical_font_size = 11.0f32;
    let pad_x = 10.0f32;
    let pad_y = 5.0f32;
    let margin = 10.0f32;
    let max_logical_w = (buffer.width() as f32 / scale - margin * 2.0 - pad_x * 2.0).max(16.0);
    let (measured_w, measured_h) = text_engine.measure(text, logical_font_size, Some(max_logical_w));

    let pill_w_logical = (measured_w + pad_x * 2.0).min(buffer.width() as f32 / scale - margin * 2.0);
    let pill_h_logical = measured_h + pad_y * 2.0;
    let pill_x_logical = (buffer.width() as f32 / scale - pill_w_logical - margin).max(margin);
    let pill_y_logical = (buffer.height() as f32 / scale - pill_h_logical - margin).max(margin);

    let pill_rect = LayoutRect {
        x: pill_x_logical * scale,
        y: pill_y_logical * scale,
        width: pill_w_logical * scale,
        height: pill_h_logical * scale,
    };
    fill_rounded_rect(
        buffer,
        pill_rect,
        8.0 * scale,
        ColorRgba::rgba(15, 23, 42, 220),
        1.0,
    );
    stroke_rounded_rect(
        buffer,
        pill_rect,
        8.0 * scale,
        1.0 * scale,
        ColorRgba::rgba(56, 189, 248, 180),
        1.0,
    );
    text_engine.rasterize_text(
        buffer,
        LayoutRect {
            x: pill_x_logical + pad_x,
            y: pill_y_logical + pad_y,
            width: (pill_w_logical - pad_x * 2.0).max(1.0),
            height: (pill_h_logical - pad_y * 2.0).max(1.0),
        },
        text,
        logical_font_size,
        scale,
        ColorRgba::rgb(226, 232, 240),
        1.0,
    );
}

#[inline]
fn scale_rect(rect: LayoutRect, scale: f32) -> LayoutRect {
    LayoutRect {
        x: rect.x * scale,
        y: rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    }
}
