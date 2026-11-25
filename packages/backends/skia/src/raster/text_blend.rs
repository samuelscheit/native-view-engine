//! Glyph alpha/subpixel/color blitting and embedded 5x7 bitmap text rasterizer (with weight
//! and alignment support for fontless fallback environments).

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use crate::font::{glyph_bitmap, GLYPH_HEIGHT, GLYPH_WIDTH};
use crate::text::TextAlign;

use super::PixelBuffer;

/// Blends an 8-bit grayscale antialiased glyph mask into `pixels`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_mask_glyph(
    pixels: &mut [u32],
    stride: usize,
    glyph_data: &[u8],
    glyph_width: usize,
    dst_x0: usize,
    dst_y0: usize,
    src_x_off: usize,
    src_y_off: usize,
    span_w: usize,
    rows: usize,
    color: ColorRgba,
    base_alpha: u32,
) {
    if base_alpha == 0 {
        return;
    }
    let packed_fg = PixelBuffer::pack_rgb(color.r, color.g, color.b);
    let sr_u32 = color.r as u32;
    let sg_u32 = color.g as u32;
    let sb_u32 = color.b as u32;

    for row in 0..rows {
        let dst_row_start = (dst_y0 + row) * stride + dst_x0;
        let src_row_start = (src_y_off + row) * glyph_width + src_x_off;
        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];
        let src_slice = &glyph_data[src_row_start..src_row_start + span_w];

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

/// Blends an RGB subpixel glyph mask (`SwashContent::SubpixelMask`) into `pixels`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_subpixel_glyph(
    pixels: &mut [u32],
    stride: usize,
    glyph_data: &[u8],
    glyph_width: usize,
    dst_x0: usize,
    dst_y0: usize,
    src_x_off: usize,
    src_y_off: usize,
    span_w: usize,
    rows: usize,
    color: ColorRgba,
    base_alpha: u32,
) {
    if base_alpha == 0 {
        return;
    }
    let sr_u32 = color.r as u32;
    let sg_u32 = color.g as u32;
    let sb_u32 = color.b as u32;

    for row in 0..rows {
        let dst_row_start = (dst_y0 + row) * stride + dst_x0;
        let src_row_px = (src_y_off + row) * glyph_width + src_x_off;
        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];

        for (col, dst_px) in dst_slice.iter_mut().enumerate() {
            let idx = (src_row_px + col) * 4;
            if idx + 3 >= glyph_data.len() {
                break;
            }
            let mr = glyph_data[idx] as u32;
            let mg = glyph_data[idx + 1] as u32;
            let mb = glyph_data[idx + 2] as u32;
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

/// Blends an RGBA color glyph (e.g. color emoji, `SwashContent::Color`) into `pixels`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_color_glyph(
    pixels: &mut [u32],
    stride: usize,
    glyph_data: &[u8],
    glyph_width: usize,
    dst_x0: usize,
    dst_y0: usize,
    src_x_off: usize,
    src_y_off: usize,
    span_w: usize,
    rows: usize,
    opacity_u32: u32,
) {
    if opacity_u32 == 0 {
        return;
    }
    for row in 0..rows {
        let dst_row_start = (dst_y0 + row) * stride + dst_x0;
        let src_row_px = (src_y_off + row) * glyph_width + src_x_off;
        let dst_slice = &mut pixels[dst_row_start..dst_row_start + span_w];

        for (col, dst_px) in dst_slice.iter_mut().enumerate() {
            let idx = (src_row_px + col) * 4;
            if idx + 3 >= glyph_data.len() {
                break;
            }
            let cr = glyph_data[idx] as u32;
            let cg = glyph_data[idx + 1] as u32;
            let cb = glyph_data[idx + 2] as u32;
            let ca = glyph_data[idx + 3] as u32;
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

/// Embedded 5x7 bitmap fallback text rasterizer supporting `font_weight` (faux-bold at >= 600)
/// and per-line `text_align` (`Left`, `Center`, `Right`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_bitmap_text(
    buf: &mut PixelBuffer,
    rect: LayoutRect,
    text: &str,
    font_size: f32,
    font_weight: u16,
    text_align: TextAlign,
    color: ColorRgba,
    opacity: f32,
    phys_clip: Option<[i32; 4]>,
) {
    if text.is_empty() || font_size <= 0.0 || color.a == 0 || opacity <= 0.0 {
        return;
    }

    let metrics = crate::font::bitmap_font_metrics(font_size);
    let glyph_h = metrics.glyph_h;
    let glyph_w = metrics.glyph_w;
    let advance_x = metrics.advance_x;
    let line_height = metrics.line_height;
    let faux_bold = font_weight >= 600;

    let vertical_pad = if !text.contains('\n') && rect.height > glyph_h {
        ((rect.height - glyph_h) * 0.5).min(font_size * 0.25)
    } else {
        0.0
    };

    let buf_w = buf.width() as i32;
    let buf_h = buf.height() as i32;
    let mut clip_x0 = (rect.x.floor() as i32).max(0);
    let mut clip_y0 = (rect.y.floor() as i32).max(0);
    let mut clip_x1 = ((rect.x + rect.width).ceil() as i32).min(buf_w);
    let mut clip_y1 = ((rect.y + rect.height).ceil() as i32).min(buf_h);
    if let Some([px0, py0, px1, py1]) = phys_clip {
        clip_x0 = clip_x0.max(px0);
        clip_y0 = clip_y0.max(py0);
        clip_x1 = clip_x1.min(px1);
        clip_y1 = clip_y1.min(py1);
    }
    if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
        return;
    }

    let mut cursor_y = rect.y + vertical_pad;

    for line in text.split('\n') {
        let char_count = line.chars().count();
        let line_w = if char_count == 0 {
            0.0
        } else {
            (char_count as f32) * advance_x - (advance_x - glyph_w).max(0.0)
        };
        let align_offset = match text_align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((rect.width - line_w) * 0.5).max(0.0),
            TextAlign::Right => (rect.width - line_w).max(0.0),
        };
        let mut cursor_x = rect.x + align_offset;

        for ch in line.chars() {
            if ch == ' ' {
                cursor_x += advance_x;
                continue;
            }
            let bitmap = glyph_bitmap(ch);
            let passes: &[f32] = if faux_bold { &[0.0, 1.0] } else { &[0.0] };
            for &dx in passes {
                let gx = cursor_x + dx;
                let x0 = (gx.floor() as i32).max(clip_x0);
                let x1 = ((gx + glyph_w).ceil() as i32).min(clip_x1);
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
                            let u = ((px as f32 + 0.5) - gx) * inv_w;
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
            }
            cursor_x += advance_x;
        }
        cursor_y += line_height;
    }
}
