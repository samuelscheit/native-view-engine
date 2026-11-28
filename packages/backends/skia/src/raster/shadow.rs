//! Analytical Gaussian-falloff rounded-rect shadow rasterizer (axis-aligned fast path plus shared
//! physical-pixel blur sigma & inset geometry helpers). Affine-transformed outer and inset shadow
//! rasterizers live in [`super::shadow_transformed`].

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::clip::{rounded_rect_sdf, ClipStack};
use super::{fill_rounded_rect, scale_rect, PixelBuffer};

/// Maximum physical-pixel Gaussian blur sigma shared by CPU and GPU shadow rasterizers.
pub(crate) const MAX_BLUR_SIGMA: f32 = 512.0;
/// Crisp/blurred cutoff in physical device pixels (`phys_blur <= 0.5` renders crisp).
pub(crate) const BLUR_CRISP_THRESHOLD_PX: f32 = 0.5;
/// Minimum physical-pixel blur sigma once `phys_blur > BLUR_CRISP_THRESHOLD_PX`.
pub(crate) const MIN_BLUR_SIGMA_PX: f32 = 0.5;

/// Returns `(geom_scale, min_axis_scale)` of a 2D affine matrix `[a, b, c, d, ..]`, matching
/// Skia's `SkMatrix::mapRadius(1.0)` identically via `sx.sqrt() * sy.sqrt()` so huge scales never
/// overflow intermediate `sx * sy`.
#[inline]
pub(crate) fn linear_ctm_scales(m: &[f32; 6]) -> (f32, f32) {
    let sx = m[0].hypot(m[1]);
    let sy = m[2].hypot(m[3]);
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return (0.0, 0.0);
    }
    (sx.sqrt() * sy.sqrt(), sx.min(sy))
}

/// Evaluates the unified physical-pixel blur sigma from `blur_radius` and `ctm_scale` in `f64`
/// so finite extreme blurs (e.g. `2e38 @ 2x`) clamp cleanly to `MAX_BLUR_SIGMA` instead of
/// overflowing to `+Inf`. Returns `None` when crisp (`phys_blur <= 0.5 px`) or degenerate.
#[inline]
pub(crate) fn physical_blur_sigma(blur_radius: f32, ctm_scale: f32) -> Option<f32> {
    if !blur_radius.is_finite() || !ctm_scale.is_finite() || ctm_scale <= 0.0 || blur_radius <= 0.0
    {
        return None;
    }
    let phys_blur = (blur_radius as f64) * (ctm_scale as f64);
    if phys_blur <= BLUR_CRISP_THRESHOLD_PX as f64 {
        return None;
    }
    Some((phys_blur * 0.5).clamp(MIN_BLUR_SIGMA_PX as f64, MAX_BLUR_SIGMA as f64) as f32)
}

/// Fast Abramowitz & Stegun 7.1.26 approximation of `0.5 * erfc(x)` accurate to `< 1e-6`.
/// Maps signed distance normalized by `sigma * sqrt(2)` into smooth Gaussian coverage `0.0..=1.0`.
#[inline]
pub(super) fn gaussian_edge_coverage(x: f32) -> f32 {
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

#[inline]
pub(super) fn valid_outer_shadow_rect(
    rect: LayoutRect,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    opacity: f32,
) -> Option<LayoutRect> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !offset_x.is_finite()
        || !offset_y.is_finite()
        || !blur_radius.is_finite()
        || !spread_radius.is_finite()
        || !opacity.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
        || opacity <= 0.0
    {
        return None;
    }
    let shadow_w = rect.width + spread_radius * 2.0;
    let shadow_h = rect.height + spread_radius * 2.0;
    let shadow_x = rect.x + offset_x - spread_radius;
    let shadow_y = rect.y + offset_y - spread_radius;
    if !shadow_w.is_finite()
        || !shadow_h.is_finite()
        || !shadow_x.is_finite()
        || !shadow_y.is_finite()
        || shadow_w <= 0.0
        || shadow_h <= 0.0
    {
        return None;
    }
    Some(LayoutRect {
        x: shadow_x,
        y: shadow_y,
        width: shadow_w,
        height: shadow_h,
    })
}

/// Shared inset padding-box & cutout-hole geometry resolution used by BOTH CPU and GPU rasterizers.
pub(crate) enum InsetGeometry {
    /// Zero shadow visible (degenerate rect, infinite negative-spread hole, or full cover past blur).
    Empty,
    /// Cutout hole collapsed or shifted past the padding box + blur tail: solid-fill padding box.
    SolidFill {
        inner_rect: LayoutRect,
        inner_radius: f32,
    },
    /// Standard blurred or crisp inset ring inside `inner_rect` outside `hole_rect`.
    Ring {
        inner_rect: LayoutRect,
        inner_radius: f32,
        hole_rect: LayoutRect,
        base_hole_r: f32,
    },
}

/// Resolves CSS inset `box-shadow` padding-box and cutout-hole geometry in `rect`'s coordinate
/// space, sharing exact overflow/disjoint/cover rules between CPU and GPU rasterizers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_inset_geometry(
    rect: LayoutRect,
    radius: f32,
    border_width: f32,
    offset_x: f32,
    offset_y: f32,
    blur_radius: f32,
    spread_radius: f32,
    opacity: f32,
    blur_margin: f32,
) -> InsetGeometry {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || !offset_x.is_finite()
        || !offset_y.is_finite()
        || !blur_radius.is_finite()
        || !spread_radius.is_finite()
        || !opacity.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
        || opacity <= 0.0
    {
        return InsetGeometry::Empty;
    }
    let bw = if border_width.is_finite() {
        border_width.max(0.0)
    } else {
        0.0
    };
    let inner_w = rect.width - 2.0 * bw;
    let inner_h = rect.height - 2.0 * bw;
    let inner_x = rect.x + bw;
    let inner_y = rect.y + bw;
    if !inner_w.is_finite()
        || !inner_h.is_finite()
        || !inner_x.is_finite()
        || !inner_y.is_finite()
        || inner_w <= 0.0
        || inner_h <= 0.0
    {
        return InsetGeometry::Empty;
    }
    let inner_rect = LayoutRect {
        x: inner_x,
        y: inner_y,
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
    // Huge negative spread expands the cutout hole itself to infinity -> casts zero shadow.
    if spread_radius < 0.0 && (!hole_w.is_finite() || !hole_h.is_finite()) {
        return InsetGeometry::Empty;
    }
    let hole_x = inner_x + offset_x + spread_radius;
    let hole_y = inner_y + offset_y + spread_radius;
    // When `hole_w`/`hole_h` are finite and positive while `hole_x`/`hole_y` overflowed to `±Inf`
    // (extreme `offset_x/y`), the finite hole was shifted infinitely far away from the padding
    // box, leaving the entire padding box outside the hole -> `SolidFill` (NOT `Empty`).
    if !hole_w.is_finite()
        || !hole_h.is_finite()
        || !hole_x.is_finite()
        || !hole_y.is_finite()
        || hole_w <= 0.0
        || hole_h <= 0.0
    {
        return InsetGeometry::SolidFill {
            inner_rect,
            inner_radius,
        };
    }

    let safe_margin = if blur_margin.is_finite() {
        blur_margin.max(0.0) as f64
    } else {
        0.0
    };
    let ix0 = inner_x as f64;
    let iy0 = inner_y as f64;
    let ix1 = ix0 + inner_w as f64;
    let iy1 = iy0 + inner_h as f64;
    let hx0 = hole_x as f64;
    let hy0 = hole_y as f64;
    let hx1 = hx0 + hole_w as f64;
    let hy1 = hy0 + hole_h as f64;

    if hx0 >= ix1 + safe_margin
        || hx1 <= ix0 - safe_margin
        || hy0 >= iy1 + safe_margin
        || hy1 <= iy0 - safe_margin
    {
        return InsetGeometry::SolidFill {
            inner_rect,
            inner_radius,
        };
    }

    let base_hole_r = if inner_radius > 0.01 {
        (inner_radius - spread_radius).max(0.0)
    } else {
        0.0
    };
    let clamped_hole_r = base_hole_r.min(hole_w * 0.5).min(hole_h * 0.5);
    let cover_margin = (ix0 - hx0)
        .min(iy0 - hy0)
        .min(hx1 - ix1)
        .min(hy1 - iy1);
    if cover_margin >= safe_margin
        && (clamped_hole_r as f64) <= (inner_radius as f64) + cover_margin + 1e-4
    {
        return InsetGeometry::Empty;
    }

    InsetGeometry::Ring {
        inner_rect,
        inner_radius,
        hole_rect: LayoutRect {
            x: hole_x,
            y: hole_y,
            width: hole_w,
            height: hole_h,
        },
        base_hole_r,
    }
}

/// Rasterizes an axis-aligned soft rounded box shadow from logical inputs scaled by `scale`.
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
    scale: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if color.a == 0 || clip.is_empty() {
        return;
    }
    let Some(logical_rect) =
        valid_outer_shadow_rect(rect, offset_x, offset_y, blur_radius, spread_radius, opacity)
    else {
        return;
    };
    let safe_r = if radius.is_finite() {
        radius.max(0.0)
    } else {
        0.0
    };
    let logical_base_r = if safe_r > 0.01 {
        (safe_r + spread_radius)
            .max(0.0)
            .min(logical_rect.width * 0.5)
            .min(logical_rect.height * 0.5)
    } else {
        0.0
    };
    let shadow_rect = scale_rect(logical_rect, scale);
    if !shadow_rect.x.is_finite()
        || !shadow_rect.y.is_finite()
        || !shadow_rect.width.is_finite()
        || !shadow_rect.height.is_finite()
        || shadow_rect.width <= 0.0
        || shadow_rect.height <= 0.0
    {
        return;
    }
    let base_r = (logical_base_r * scale)
        .min(shadow_rect.width * 0.5)
        .min(shadow_rect.height * 0.5);

    let Some(sigma) = physical_blur_sigma(blur_radius, scale) else {
        fill_rounded_rect(buf, clip, shadow_rect, base_r, color, opacity);
        return;
    };

    // Standard CSS blur-radius ~= 2 * sigma; 3*sigma (~1.5*blur) covers >99.7% of Gaussian energy.
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let pad = (sigma * 3.0).ceil();
    let pad_i = pad as i32;

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (shadow_rect.x.floor() as i32)
        .saturating_sub(pad_i)
        .clamp(cx0, cx1);
    let max_x = ((shadow_rect.x + shadow_rect.width).ceil() as i32)
        .saturating_add(pad_i)
        .clamp(cx0, cx1);
    let min_y = (shadow_rect.y.floor() as i32)
        .saturating_sub(pad_i)
        .clamp(cy0, cy1);
    let max_y = ((shadow_rect.y + shadow_rect.height).ceil() as i32)
        .saturating_add(pad_i)
        .clamp(cy0, cy1);
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

/// Rasterizes an analytical Gaussian-falloff CSS inset `box-shadow` from logical inputs scaled by
/// `scale`, strictly clipped to the inner padding box (`inset(rect, border_width)`).
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
    scale: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if color.a == 0 || clip.is_empty() {
        return;
    }
    let phys_sigma = physical_blur_sigma(blur_radius, scale);
    let blur_margin = phys_sigma.map_or(1.0, |s| s * 4.0 + 2.0) / scale.max(1e-3);
    let (inner_rect, inner_radius, hole_rect, base_hole_r) = match resolve_inset_geometry(
        rect,
        radius,
        border_width,
        offset_x,
        offset_y,
        blur_radius,
        spread_radius,
        opacity,
        blur_margin,
    ) {
        InsetGeometry::Empty => return,
        InsetGeometry::SolidFill {
            inner_rect,
            inner_radius,
        } => {
            fill_rounded_rect(
                buf,
                clip,
                scale_rect(inner_rect, scale),
                inner_radius * scale,
                color,
                opacity,
            );
            return;
        }
        InsetGeometry::Ring {
            inner_rect,
            inner_radius,
            hole_rect,
            base_hole_r,
        } => (
            scale_rect(inner_rect, scale),
            inner_radius * scale,
            scale_rect(hole_rect, scale),
            base_hole_r
                .min(hole_rect.width * 0.5)
                .min(hole_rect.height * 0.5)
                * scale,
        ),
    };
    if !inner_rect.x.is_finite()
        || !inner_rect.y.is_finite()
        || !inner_rect.width.is_finite()
        || !inner_rect.height.is_finite()
        || !hole_rect.x.is_finite()
        || !hole_rect.y.is_finite()
        || !hole_rect.width.is_finite()
        || !hole_rect.height.is_finite()
    {
        return;
    }

    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (inner_rect.x.floor() as i32).clamp(cx0, cx1);
    let max_x = ((inner_rect.x + inner_rect.width).ceil() as i32).clamp(cx0, cx1);
    let min_y = (inner_rect.y.floor() as i32).clamp(cy0, cy1);
    let max_y = ((inner_rect.y + inner_rect.height).ceil() as i32).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let crisp = phys_sigma.is_none();
    let sigma = phys_sigma.unwrap_or(MIN_BLUR_SIGMA_PX);
    let inv_sigma_sqrt2 = 1.0 / (sigma * std::f32::consts::SQRT_2);
    let pad_hx = inner_rect.width * 0.5;
    let pad_hy = inner_rect.height * 0.5;
    let pad_cx = inner_rect.x + pad_hx;
    let pad_cy = inner_rect.y + pad_hy;
    let hole_hx = hole_rect.width * 0.5;
    let hole_hy = hole_rect.height * 0.5;
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
    let core_x1 = ((hole_rect.x + hole_rect.width - corner_inset).floor() as i32).min(max_x);
    let core_y0 = ((hole_rect.y + skip_pad).ceil() as i32).max(min_y);
    let core_y1 = ((hole_rect.y + hole_rect.height - skip_pad).floor() as i32).min(max_y);
    let band_y0 = ((hole_rect.y + corner_inset).ceil() as i32).max(min_y);
    let band_y1 = ((hole_rect.y + hole_rect.height - corner_inset).floor() as i32).min(max_y);
    let band_x0 = ((hole_rect.x + skip_pad).ceil() as i32).max(min_x);
    let band_x1 = ((hole_rect.x + hole_rect.width - skip_pad).floor() as i32).min(max_x);

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
                let pad_sdf =
                    rounded_rect_sdf(fx, fy, pad_cx, pad_cy, pad_hx, pad_hy, inner_radius);
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
                let hole_sdf =
                    rounded_rect_sdf(fx, fy, hole_cx, hole_cy, hole_hx, hole_hy, hole_r);
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
