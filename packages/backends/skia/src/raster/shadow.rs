//! Analytical Gaussian-falloff rounded-rect shadow rasterizer.
//!
//! Uses an exact SDF + `erfc` Gaussian edge falloff with a solid interior-span fast path so soft
//! rounded shadows render in microseconds without 2D convolution passes or scratch allocations.

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::clip::{rounded_rect_sdf, ClipStack};
use super::{fill_rounded_rect, PixelBuffer};

/// Fast Abramowitz & Stegun 7.1.26 approximation of `0.5 * erfc(x)` accurate to `< 1e-6`.
/// Maps signed distance normalized by `sigma * sqrt(2)` into smooth Gaussian coverage `0.0..=1.0`.
#[inline]
fn gaussian_edge_coverage(x: f32) -> f32 {
    if x <= -3.0 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let ax = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * ax);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_72
                + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    let erfc_abs = poly * (-ax * ax).exp();
    if x >= 0.0 {
        0.5 * erfc_abs
    } else {
        1.0 - 0.5 * erfc_abs
    }
}

/// Rasterizes a soft rounded box shadow in physical pixel coordinates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_box_shadow(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if color.a == 0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let shadow_w = rect.width + spread_radius * 2.0;
    let shadow_h = rect.height + spread_radius * 2.0;
    if shadow_w <= 0.0 || shadow_h <= 0.0 {
        return;
    }
    let shadow_rect = LayoutRect {
        x: rect.x + offset_x - spread_radius,
        y: rect.y + offset_y - spread_radius,
        width: shadow_w,
        height: shadow_h,
    };
    let base_r = if radius > 0.01 {
        (radius + spread_radius).max(0.0)
    } else {
        0.0
    };

    if blur_radius <= 0.5 {
        fill_rounded_rect(buf, clip, shadow_rect, base_r, color, opacity);
        return;
    }

    // Standard CSS blur-radius ~= 2 * sigma; 3*sigma (~1.5*blur) covers >99.7% of Gaussian energy.
    let sigma = (blur_radius * 0.5).max(0.5);
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let pad = (sigma * 3.0).ceil();
    let pad_i = pad as i32;

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (shadow_rect.x.floor() as i32 - pad_i).clamp(cx0, cx1);
    let max_x = ((shadow_rect.x + shadow_rect.width).ceil() as i32 + pad_i).clamp(cx0, cx1);
    let min_y = (shadow_rect.y.floor() as i32 - pad_i).clamp(cy0, cy1);
    let max_y = ((shadow_rect.y + shadow_rect.height).ceil() as i32 + pad_i).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = shadow_rect.width * 0.5;
    let hy = shadow_rect.height * 0.5;
    let cx = shadow_rect.x + hx;
    let cy = shadow_rect.y + hy;
    let r = (base_r + sigma * 0.35).clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

    // Guaranteed coverage == 1.0 interior core where sdf <= -3*sigma (bypasses exp & sqrt).
    let core_inset = r + pad;
    let core_x0 = ((shadow_rect.x + core_inset).ceil() as i32).max(min_x);
    let core_x1 = ((shadow_rect.x + shadow_rect.width - core_inset).floor() as i32).min(max_x);
    let core_y0 = ((shadow_rect.y + pad).ceil() as i32).max(min_y);
    let core_y1 = ((shadow_rect.y + shadow_rect.height - pad).floor() as i32).min(max_y);
    let full_band_y0 = ((shadow_rect.y + core_inset).ceil() as i32).max(min_y);
    let full_band_y1 = ((shadow_rect.y + shadow_rect.height - core_inset).floor() as i32).min(max_y);
    let band_x0 = ((shadow_rect.x + pad).ceil() as i32).max(min_x);
    let band_x1 = ((shadow_rect.x + shadow_rect.width - pad).floor() as i32).min(max_x);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let (rect_l, rect_r) = if py >= full_band_y0 && py < full_band_y1 && band_x0 < band_x1 {
            (band_x0, band_x1)
        } else if py >= core_y0 && py < core_y1 && core_x0 < core_x1 {
            (core_x0, core_x1)
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
            let cov = gaussian_edge_coverage(sdf * inv_sigma_sqrt2) * clip_cov;
            if cov > 0.002 {
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
            let cov = gaussian_edge_coverage(sdf * inv_sigma_sqrt2) * clip_cov;
            if cov > 0.002 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

/// Rasterizes an analytical Gaussian-falloff CSS inset `box-shadow` in physical pixel coordinates,
/// strictly clipped to the inner padding box (`inset(rect, border_width)`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_inset_box_shadow(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    border_width: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !offset_x.is_finite()
        || !offset_y.is_finite()
        || !blur_radius.is_finite()
        || !spread_radius.is_finite()
        || !opacity.is_finite()
        || color.a == 0
        || opacity <= 0.0
        || clip.is_empty()
    {
        return;
    }

    let bw = if border_width.is_finite() {
        border_width.max(0.0)
    } else {
        0.0
    };
    let inner_w = rect.width - 2.0 * bw;
    let inner_h = rect.height - 2.0 * bw;
    if !inner_w.is_finite() || !inner_h.is_finite() || inner_w <= 0.0 || inner_h <= 0.0 {
        return;
    }
    let inner_rect = LayoutRect {
        x: rect.x + bw,
        y: rect.y + bw,
        width: inner_w,
        height: inner_h,
    };
    let safe_r = if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    };
    let inner_radius = (safe_r - bw)
        .max(0.0)
        .min(inner_w * 0.5)
        .min(inner_h * 0.5);

    let hole_w = inner_w - 2.0 * spread_radius;
    let hole_h = inner_h - 2.0 * spread_radius;
    if !hole_w.is_finite() || !hole_h.is_finite() || hole_w <= 0.0 || hole_h <= 0.0 {
        fill_rounded_rect(buf, clip, inner_rect, inner_radius, color, opacity);
        return;
    }

    let hole_rect = LayoutRect {
        x: inner_rect.x + offset_x + spread_radius,
        y: inner_rect.y + offset_y + spread_radius,
        width: hole_w,
        height: hole_h,
    };
    let base_hole_r = if inner_radius > 0.01 {
        (inner_radius - spread_radius).max(0.0)
    } else {
        0.0
    };

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (inner_rect.x.floor() as i32).clamp(cx0, cx1);
    let max_x = ((inner_rect.x + inner_w).ceil() as i32).clamp(cx0, cx1);
    let min_y = (inner_rect.y.floor() as i32).clamp(cy0, cy1);
    let max_y = ((inner_rect.y + inner_h).ceil() as i32).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let crisp = blur_radius <= 0.5;
    let sigma = (blur_radius * 0.5).max(0.5);
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let pad_hx = inner_w * 0.5;
    let pad_hy = inner_h * 0.5;
    let pad_cx = inner_rect.x + pad_hx;
    let pad_cy = inner_rect.y + pad_hy;
    let hole_hx = hole_w * 0.5;
    let hole_hy = hole_h * 0.5;
    let hole_cx = hole_rect.x + hole_hx;
    let hole_cy = hole_rect.y + hole_hy;
    let hole_r = if crisp {
        base_hole_r.min(hole_hx.min(hole_hy))
    } else {
        (base_hole_r + sigma * 0.35).clamp(0.0, hole_hx.min(hole_hy))
    };
    let rounded_clip = clip.has_rounded();

    // O(1) interior-hole skip: where `hole_sdf <= -skip_pad`, shadow coverage is `< 0.002` (or
    // exact `0.0` in crisp mode), so we skip the entire unshadowed center span per scanline.
    let skip_pad = if crisp { 1.0 } else { (sigma * 3.0).ceil() };
    let corner_inset = hole_r + skip_pad;
    let core_x0 = ((hole_rect.x + corner_inset).ceil() as i32).max(min_x);
    let core_x1 = ((hole_rect.x + hole_w - corner_inset).floor() as i32).min(max_x);
    let core_y0 = ((hole_rect.y + skip_pad).ceil() as i32).max(min_y);
    let core_y1 = ((hole_rect.y + hole_h - skip_pad).floor() as i32).min(max_y);
    let band_y0 = ((hole_rect.y + corner_inset).ceil() as i32).max(min_y);
    let band_y1 = ((hole_rect.y + hole_h - corner_inset).floor() as i32).min(max_y);
    let band_x0 = ((hole_rect.x + skip_pad).ceil() as i32).max(min_x);
    let band_x1 = ((hole_rect.x + hole_w - skip_pad).floor() as i32).min(max_x);

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let (skip_l, skip_r) = if py >= band_y0 && py < band_y1 && band_x0 < band_x1 {
            (band_x0, band_x1)
        } else if py >= core_y0 && py < core_y1 && core_x0 < core_x1 {
            (core_x0, core_x1)
        } else {
            (max_x, max_x)
        };

        let left_end = skip_l.min(max_x);
        let right_start = if skip_l < skip_r { skip_r } else { left_end };

        let mut paint_span = |x_start: i32, x_end: i32| {
            for px in x_start..x_end {
                let fx = px as f32 + 0.5;
                let pad_sdf = rounded_rect_sdf(fx, fy, pad_cx, pad_cy, pad_hx, pad_hy, inner_radius);
                let pad_cov = (0.5 - pad_sdf).clamp(0.0, 1.0);
                if pad_cov <= 0.0 {
                    continue;
                }
                let clip_cov = if rounded_clip {
                    clip.rounded_coverage(fx, fy)
                } else {
                    1.0
                };
                if clip_cov <= 0.0 {
                    continue;
                }
                let hole_sdf = rounded_rect_sdf(fx, fy, hole_cx, hole_cy, hole_hx, hole_hy, hole_r);
                let cov = if crisp {
                    let hole_cov = (0.5 - hole_sdf).clamp(0.0, 1.0);
                    (pad_cov - hole_cov).max(0.0) * clip_cov
                } else {
                    (1.0 - gaussian_edge_coverage(hole_sdf * inv_sigma_sqrt2)) * pad_cov * clip_cov
                };
                if cov > 0.002 {
                    buf.blend_pixel(px, py, color, cov * opacity);
                }
            }
        };

        paint_span(min_x, left_end);
        paint_span(right_start, max_x);
    }
}
