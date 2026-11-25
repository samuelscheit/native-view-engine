//! Pure-Rust anti-aliased software rasterizer for NVE `DisplayList`.

mod canvas;
pub(crate) mod clip;
pub mod shadow;
pub(crate) mod text_blend;

use nve_abi::ColorRgba;
use nve_engine::{LayoutRect, TextMeasurer};

use crate::text::CosmicTextEngine;
use crate::{DisplayList, PaintOp};

use canvas::{draw_image, execute_canvas_ops};
use clip::{rounded_rect_sdf, ClipStack};
use shadow::draw_box_shadow;
pub(crate) use text_blend::{
    blend_color_glyph, blend_mask_glyph, blend_subpixel_glyph, draw_bitmap_text,
};

pub use shadow::{parse_box_shadow, BoxShadow};

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

pub(crate) fn fill_rounded_rect(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color.a == 0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (rect.x.floor() as i32 - 1).clamp(cx0, cx1);
    let max_x = ((rect.x + rect.width).ceil() as i32 + 1).clamp(cx0, cx1);
    let min_y = (rect.y.floor() as i32 - 1).clamp(cy0, cy1);
    let max_y = ((rect.y + rect.height).ceil() as i32 + 1).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let r = radius.clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

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
        let (rect_l, rect_r) = if py >= full_band_y0 && py < full_band_y1 && edge_x0 < edge_x1 {
            (edge_x0, edge_x1)
        } else if py >= inner_y0 && py < inner_y1 && inner_x0 < inner_x1 {
            (inner_x0, inner_x1)
        } else {
            (max_x, max_x)
        };
        let (span_l, span_r) = if rounded_clip {
            clip.interior_span_for_row(py, rect_l, rect_r)
        } else {
            (rect_l, rect_r)
        };

        let left_end = span_l.min(max_x);
        for px in min_x..left_end {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
        if span_l < span_r {
            buf.blend_span(py, span_l, span_r, color, opacity);
        }
        let right_start = if span_l < span_r { span_r } else { left_end };
        for px in right_start..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

fn stroke_rounded_rect(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
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
        || clip.is_empty()
    {
        return;
    }
    let hw = (stroke_width * 0.5).max(0.5);
    let pad = hw.ceil() as i32 + 1;
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (rect.x.floor() as i32 - pad).clamp(cx0, cx1);
    let max_x = ((rect.x + rect.width).ceil() as i32 + pad).clamp(cx0, cx1);
    let min_y = (rect.y.floor() as i32 - pad).clamp(cy0, cy1);
    let max_y = ((rect.y + rect.height).ceil() as i32 + pad).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let r = radius.clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

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
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov > 0.0 {
                let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
                let band_dist = sdf.abs() - hw;
                let cov = (0.5 - band_dist).clamp(0.0, 1.0) * clip_cov;
                if cov > 0.0 {
                    buf.blend_pixel(px, py, color, cov * opacity);
                }
            }
            px += 1;
        }
    }
}

/// Rasterizes a retained `DisplayList` into `buffer` using the shared [`CosmicTextEngine`].
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
/// bitmap font.
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
    let mut clip_stack = ClipStack::new(buffer.width(), buffer.height());

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
            PaintOp::PushClip { rect, radius } => {
                clip_stack.push(scale_rect(*rect, scale), radius * scale);
            }
            PaintOp::PopClip => {
                clip_stack.pop();
            }
            PaintOp::DrawShadow {
                rect,
                radius,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                color,
            } => {
                draw_box_shadow(
                    buffer,
                    &clip_stack,
                    scale_rect(*rect, scale),
                    radius * scale,
                    offset_x * scale,
                    offset_y * scale,
                    blur_radius * scale,
                    spread_radius * scale,
                    *color,
                    cur_opacity,
                );
            }
            PaintOp::FillRoundedRect {
                rect,
                radius,
                color,
            } => {
                fill_rounded_rect(
                    buffer,
                    &clip_stack,
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
                    &clip_stack,
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
                font_weight,
                text_align,
                color,
            } => {
                if clip_stack.is_empty() {
                    continue;
                }
                let phys_clip = Some(clip_stack.bounds());
                if let Some(engine) = text_engine {
                    engine.rasterize_text_styled_clipped(
                        buffer,
                        *rect,
                        text,
                        *font_size,
                        *font_weight,
                        *text_align,
                        scale,
                        *color,
                        cur_opacity,
                        phys_clip,
                    );
                } else {
                    draw_bitmap_text(
                        buffer,
                        scale_rect(*rect, scale),
                        text,
                        font_size * scale,
                        *font_weight,
                        *text_align,
                        *color,
                        cur_opacity,
                        phys_clip,
                    );
                }
            }
            PaintOp::DrawImage {
                rect,
                radius,
                src,
                resize_mode,
            } => {
                draw_image(
                    buffer,
                    &clip_stack,
                    *rect,
                    *radius,
                    src,
                    *resize_mode,
                    scale,
                    cur_opacity,
                );
            }
            PaintOp::CustomSkiaCanvas { rect, ops, .. } => {
                execute_canvas_ops(buffer, &mut clip_stack, *rect, ops, scale, cur_opacity);
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

    let pill_w_logical =
        (measured_w + pad_x * 2.0).min(buffer.width() as f32 / scale - margin * 2.0);
    let pill_h_logical = measured_h + pad_y * 2.0;
    let pill_x_logical = (buffer.width() as f32 / scale - pill_w_logical - margin).max(margin);
    let pill_y_logical = (buffer.height() as f32 / scale - pill_h_logical - margin).max(margin);

    let pill_rect = LayoutRect {
        x: pill_x_logical * scale,
        y: pill_y_logical * scale,
        width: pill_w_logical * scale,
        height: pill_h_logical * scale,
    };
    let clip = ClipStack::new(buffer.width(), buffer.height());
    fill_rounded_rect(
        buffer,
        &clip,
        pill_rect,
        8.0 * scale,
        ColorRgba::rgba(15, 23, 42, 220),
        1.0,
    );
    stroke_rounded_rect(
        buffer,
        &clip,
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
pub(crate) fn scale_rect(rect: LayoutRect, scale: f32) -> LayoutRect {
    LayoutRect {
        x: rect.x * scale,
        y: rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    }
}
