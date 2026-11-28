//! Image blitting (`ImageFit`) for the pure-Rust Skia software rasterizer.
//!
//! Vector canvas (`Canvas`) and SVG (`Svg`) rasterization live in standalone leaf modules
//! (`nve-canvas-skia`, `nve-svg-skia`) registered via `SkiaTableRegistry`.

use nve_tables::tables::ImageFit;
use nve_engine::LayoutRect;

use crate::image_cache::{compute_image_placement, ImageCache};

use super::clip::{rounded_rect_sdf, ClipStack};
use super::{scale_rect, PixelBuffer};

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    src: &str,
    fit: ImageFit,
    scale: f32,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let Ok(img) = ImageCache::shared().get_or_decode(src) else {
        return;
    };
    let Some(placement) = compute_image_placement(rect, img.width, img.height, fit) else {
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
