//! Image blitting (`ImageResizeMode`) and `NVE_DRAW1` vector canvas primitive rasterization.

use nve_abi::ColorRgba;
use nve_components::canvas_ir::{CanvasOp, ImageResizeMode};
use nve_engine::LayoutRect;

use crate::image_cache::{compute_image_placement, ImageCache};

use super::clip::{rounded_rect_sdf, ClipStack};
use super::{fill_rounded_rect, scale_rect, PixelBuffer};

fn fill_circle(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    cx: f32,
    cy: f32,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if radius <= 0.0 || color.a == 0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = ((cx - radius).floor() as i32 - 1).clamp(cx0, cx1);
    let max_x = ((cx + radius).ceil() as i32 + 1).clamp(cx0, cx1);
    let min_y = ((cy - radius).floor() as i32 - 1).clamp(cy0, cy1);
    let max_y = ((cy + radius).ceil() as i32 + 1).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let rounded_clip = clip.has_rounded();
    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let dy = fy - cy;
        let dy2 = dy * dy;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let dx = fx - cx;
            let dist = (dx * dx + dy2).sqrt() - radius;
            let mut cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            if rounded_clip {
                cov *= clip.rounded_coverage(fx, fy);
                if cov <= 0.0 {
                    continue;
                }
            }
            buf.blend_pixel(px, py, color, cov * opacity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke_line(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if width <= 0.0 || color.a == 0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let hw = (width * 0.5).max(0.5);
    let pad = hw.ceil() as i32 + 1;
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (x0.min(x1).floor() as i32 - pad).clamp(cx0, cx1);
    let max_x = (x0.max(x1).ceil() as i32 + pad).clamp(cx0, cx1);
    let min_y = (y0.min(y1).floor() as i32 - pad).clamp(cy0, cy1);
    let max_y = (y0.max(y1).ceil() as i32 + pad).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let dx = x1 - x0;
    let dy = y1 - y0;
    let len_sq = dx * dx + dy * dy;
    let rounded_clip = clip.has_rounded();

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let py0 = fy - y0;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let px0 = fx - x0;
            let t = if len_sq <= 1e-6 {
                0.0
            } else {
                ((px0 * dx + py0 * dy) / len_sq).clamp(0.0, 1.0)
            };
            let rx = px0 - t * dx;
            let ry = py0 - t * dy;
            let dist = (rx * rx + ry * ry).sqrt() - hw;
            let mut cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            if rounded_clip {
                cov *= clip.rounded_coverage(fx, fy);
                if cov <= 0.0 {
                    continue;
                }
            }
            buf.blend_pixel(px, py, color, cov * opacity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    src: &str,
    resize_mode: ImageResizeMode,
    scale: f32,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let Ok(img) = ImageCache::shared().get_or_decode(src) else {
        return;
    };
    let Some(placement) = compute_image_placement(rect, img.width, img.height, resize_mode) else {
        return;
    };

    let outer = scale_rect(rect, scale);
    let dst_x = placement.dst_x * scale;
    let dst_y = placement.dst_y * scale;
    let dst_w = placement.dst_w * scale;
    let dst_h = placement.dst_h * scale;
    if dst_w <= 0.0 || dst_h <= 0.0 {
        return;
    }

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let ox0 = (outer.x.floor() as i32).max(cx0);
    let oy0 = (outer.y.floor() as i32).max(cy0);
    let ox1 = ((outer.x + outer.width).ceil() as i32).min(cx1);
    let oy1 = ((outer.y + outer.height).ceil() as i32).min(cy1);

    let min_x = (dst_x.floor() as i32).clamp(ox0, ox1);
    let max_x = ((dst_x + dst_w).ceil() as i32).clamp(ox0, ox1);
    let min_y = (dst_y.floor() as i32).clamp(oy0, oy1);
    let max_y = ((dst_y + dst_h).ceil() as i32).clamp(oy0, oy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = outer.width * 0.5;
    let hy = outer.height * 0.5;
    let ocx = outer.x + hx;
    let ocy = outer.y + hy;
    let r = (radius * scale).clamp(0.0, hx.min(hy));
    let has_corner_radius = r > 0.01;
    let rounded_clip = clip.has_rounded();

    let inv_dst_w = placement.src_w / dst_w;
    let inv_dst_h = placement.src_h / dst_h;
    let use_nearest =
        (dst_w - placement.src_w).abs() < 0.01 && (dst_h - placement.src_h).abs() < 0.01;

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let sv = placement.src_y + (fy - dst_y) * inv_dst_h;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let mut cov = opacity;
            if has_corner_radius {
                let sdf = rounded_rect_sdf(fx, fy, ocx, ocy, hx, hy, r);
                cov *= (0.5 - sdf).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
            }
            if rounded_clip {
                cov *= clip.rounded_coverage(fx, fy);
                if cov <= 0.0 {
                    continue;
                }
            }
            let su = placement.src_x + (fx - dst_x) * inv_dst_w;
            let sample = if use_nearest {
                img.sample_nearest(su, sv)
            } else {
                img.sample_bilinear(su, sv)
            };
            if sample.a > 0 {
                buf.blend_pixel(px, py, sample, cov);
            }
        }
    }
}

pub(crate) fn execute_canvas_ops(
    buf: &mut PixelBuffer,
    clip: &mut ClipStack,
    canvas_rect: LayoutRect,
    ops: &[CanvasOp],
    scale: f32,
    opacity: f32,
) {
    if canvas_rect.width <= 0.0 || canvas_rect.height <= 0.0 || opacity <= 0.0 || ops.is_empty() {
        return;
    }
    let scaled_canvas = scale_rect(canvas_rect, scale);
    clip.push(scaled_canvas, 0.0);
    let mut local_clip_depth = 0usize;

    for op in ops {
        match op {
            CanvasOp::Clear { color } => {
                if color.a > 0 {
                    fill_rounded_rect(buf, clip, scaled_canvas, 0.0, *color, opacity);
                }
            }
            CanvasOp::FillRect { x, y, w, h, color } => {
                let r = LayoutRect {
                    x: (canvas_rect.x + *x) * scale,
                    y: (canvas_rect.y + *y) * scale,
                    width: *w * scale,
                    height: *h * scale,
                };
                fill_rounded_rect(buf, clip, r, 0.0, *color, opacity);
            }
            CanvasOp::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => {
                let r = LayoutRect {
                    x: (canvas_rect.x + *x) * scale,
                    y: (canvas_rect.y + *y) * scale,
                    width: *w * scale,
                    height: *h * scale,
                };
                fill_rounded_rect(buf, clip, r, *radius * scale, *color, opacity);
            }
            CanvasOp::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => {
                fill_circle(
                    buf,
                    clip,
                    (canvas_rect.x + *cx) * scale,
                    (canvas_rect.y + *cy) * scale,
                    *radius * scale,
                    *color,
                    opacity,
                );
            }
            CanvasOp::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => {
                stroke_line(
                    buf,
                    clip,
                    (canvas_rect.x + *x0) * scale,
                    (canvas_rect.y + *y0) * scale,
                    (canvas_rect.x + *x1) * scale,
                    (canvas_rect.y + *y1) * scale,
                    *width * scale,
                    *color,
                    opacity,
                );
            }
            CanvasOp::StrokePolyline {
                points,
                width,
                color,
            } => {
                if points.len() == 1 {
                    let (px, py) = points[0];
                    fill_circle(
                        buf,
                        clip,
                        (canvas_rect.x + px) * scale,
                        (canvas_rect.y + py) * scale,
                        (*width * 0.5 * scale).max(0.5),
                        *color,
                        opacity,
                    );
                } else {
                    for seg in points.windows(2) {
                        let (x0, y0) = seg[0];
                        let (x1, y1) = seg[1];
                        stroke_line(
                            buf,
                            clip,
                            (canvas_rect.x + x0) * scale,
                            (canvas_rect.y + y0) * scale,
                            (canvas_rect.x + x1) * scale,
                            (canvas_rect.y + y1) * scale,
                            *width * scale,
                            *color,
                            opacity,
                        );
                    }
                }
            }
            CanvasOp::ClipRect { x, y, w, h } => {
                let r = LayoutRect {
                    x: (canvas_rect.x + *x) * scale,
                    y: (canvas_rect.y + *y) * scale,
                    width: *w * scale,
                    height: *h * scale,
                };
                clip.push(r, 0.0);
                local_clip_depth += 1;
            }
            CanvasOp::RestoreClip => {
                if local_clip_depth > 0 {
                    clip.pop();
                    local_clip_depth -= 1;
                }
            }
        }
    }

    for _ in 0..local_clip_depth {
        clip.pop();
    }
    clip.pop();
}
