//! Affine-transformed outer and inset box-shadow software rasterizers using exact inverse-affine
//! SDF evaluation with device-pixel gradient normalization (respecting rotation and non-uniform
//! scale cleanly without unrotated AABB bleed).

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::clip::{rounded_rect_sdf_with_normal, ClipStack};
use super::shadow::{
    gaussian_edge_coverage, linear_ctm_scales, physical_blur_sigma, resolve_inset_geometry,
    valid_outer_shadow_rect, InsetGeometry,
};
use super::transform::{fill_rounded_rect_transformed, transformed_aabb};
use super::{scale_rect, PixelBuffer};

#[inline]
fn invert_affine_2x2(m: &[f32; 6]) -> Option<[f32; 4]> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-8 {
        return None;
    }
    let inv_det = 1.0 / det;
    let inv = [
        m[3] * inv_det,
        -m[1] * inv_det,
        -m[2] * inv_det,
        m[0] * inv_det,
    ];
    inv.iter().all(|v| v.is_finite()).then_some(inv)
}

#[inline]
fn device_sdf_from_local(sdf: f32, nx: f32, ny: f32, inv: [f32; 4], m_scale: f32) -> f32 {
    let gx = nx * inv[0] + ny * inv[1];
    let gy = nx * inv[2] + ny * inv[3];
    let grad_len = gx.hypot(gy);
    if grad_len > 1e-6 {
        sdf / grad_len
    } else {
        sdf * m_scale
    }
}

/// Rasterizes an affine-transformed outer box shadow from logical inputs scaled by `scale`,
/// using exact inverse-affine SDF evaluation with device-pixel gradient normalization.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_box_shadow_transformed(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    m: &[f32; 6],
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
    let Some(inv) = invert_affine_2x2(m) else {
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

    let (m_scale, _) = linear_ctm_scales(m);
    if m_scale <= 0.0 {
        return;
    }
    let total_scale = ((m_scale as f64) * (scale as f64)).min(f32::MAX as f64) as f32;
    let phys_sigma = physical_blur_sigma(blur_radius, total_scale);
    let pad_dev = phys_sigma.map_or(1.0, |s| (s * 3.0).ceil());
    let (bx0, by0, bx1, by1) = transformed_aabb(m, shadow_rect, 0.0);
    if !bx0.is_finite() || !by0.is_finite() || !bx1.is_finite() || !by1.is_finite() {
        return;
    }
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let pad_i = pad_dev as i32;
    let min_x = (bx0.floor() as i32).saturating_sub(pad_i).clamp(cx0, cx1);
    let max_x = (bx1.ceil() as i32).saturating_add(pad_i).clamp(cx0, cx1);
    let min_y = (by0.floor() as i32).saturating_sub(pad_i).clamp(cy0, cy1);
    let max_y = (by1.ceil() as i32).saturating_add(pad_i).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = shadow_rect.width * 0.5;
    let hy = shadow_rect.height * 0.5;
    let cx = shadow_rect.x + hx;
    let cy = shadow_rect.y + hy;
    let r = match phys_sigma {
        Some(sigma) => {
            let sigma_local = sigma / m_scale.max(1e-4);
            (base_r + sigma_local * 0.35).clamp(0.0, hx.min(hy))
        }
        None => base_r.clamp(0.0, hx.min(hy)),
    };
    let inv_sigma_sqrt2 = phys_sigma.map(|s| 1.0 / (s * std::f32::consts::SQRT_2));
    let rounded_clip = clip.has_rounded();

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let dy = fy - m[5];
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let dx = fx - m[4];
            let lx = inv[0] * dx + inv[2] * dy;
            let ly = inv[1] * dx + inv[3] * dy;
            let (sdf, nx, ny) = rounded_rect_sdf_with_normal(lx, ly, cx, cy, hx, hy, r);
            let sdf_dev = device_sdf_from_local(sdf, nx, ny, inv, m_scale);
            let cov = match inv_sigma_sqrt2 {
                Some(inv_s) => gaussian_edge_coverage(sdf_dev * inv_s) * clip_cov,
                None => (0.5 - sdf_dev).clamp(0.0, 1.0) * clip_cov,
            };
            if cov > 0.002 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

/// Rasterizes an affine-transformed CSS inset `box-shadow` from logical inputs scaled by `scale`,
/// using exact inverse-affine SDF evaluation with device-pixel gradient normalization and strict
/// clipping to the transformed padding box so rotated/scaled inset shadows never bleed outside.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_inset_box_shadow_transformed(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    m: &[f32; 6],
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
    let Some(inv) = invert_affine_2x2(m) else {
        return;
    };
    let (m_scale, m_min_axis) = linear_ctm_scales(m);
    if m_scale <= 0.0 {
        return;
    }
    let total_scale = ((m_scale as f64) * (scale as f64)).min(f32::MAX as f64) as f32;
    let eff_min_axis = ((m_min_axis as f64) * (scale as f64)).clamp(1e-3, f32::MAX as f64) as f32;
    let phys_sigma = physical_blur_sigma(blur_radius, total_scale);
    let blur_margin = phys_sigma.map_or(1.0, |s| s * 4.0 + 2.0) / eff_min_axis;

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
            fill_rounded_rect_transformed(
                buf,
                clip,
                m,
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

    let (bx0, by0, bx1, by1) = transformed_aabb(m, inner_rect, 1.0);
    if !bx0.is_finite() || !by0.is_finite() || !bx1.is_finite() || !by1.is_finite() {
        return;
    }
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (bx0.floor() as i32).clamp(cx0, cx1);
    let max_x = (bx1.ceil() as i32).clamp(cx0, cx1);
    let min_y = (by0.floor() as i32).clamp(cy0, cy1);
    let max_y = (by1.ceil() as i32).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let inv_sigma_sqrt2 = phys_sigma.map(|s| 1.0 / (s * std::f32::consts::SQRT_2));
    let pad_hx = inner_rect.width * 0.5;
    let pad_hy = inner_rect.height * 0.5;
    let pad_cx = inner_rect.x + pad_hx;
    let pad_cy = inner_rect.y + pad_hy;
    let hole_hx = hole_rect.width * 0.5;
    let hole_hy = hole_rect.height * 0.5;
    let hole_cx = hole_rect.x + hole_hx;
    let hole_cy = hole_rect.y + hole_hy;
    let hole_r = match phys_sigma {
        Some(sigma) => {
            let sigma_local = sigma / m_scale.max(1e-4);
            (base_hole_r + sigma_local * 0.35).clamp(0.0, hole_hx.min(hole_hy))
        }
        None => base_hole_r.min(hole_hx.min(hole_hy)),
    };
    let rounded_clip = clip.has_rounded();

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        let dy = fy - m[5];
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let clip_cov = if rounded_clip {
                clip.rounded_coverage(fx, fy)
            } else {
                1.0
            };
            if clip_cov <= 0.0 {
                continue;
            }
            let dx = fx - m[4];
            let lx = inv[0] * dx + inv[2] * dy;
            let ly = inv[1] * dx + inv[3] * dy;
            let (pad_sdf, pnx, pny) =
                rounded_rect_sdf_with_normal(lx, ly, pad_cx, pad_cy, pad_hx, pad_hy, inner_radius);
            let pad_sdf_dev = device_sdf_from_local(pad_sdf, pnx, pny, inv, m_scale);
            let pad_cov = (0.5 - pad_sdf_dev).clamp(0.0, 1.0);
            if pad_cov <= 0.0 {
                continue;
            }
            let (hole_sdf, hnx, hny) =
                rounded_rect_sdf_with_normal(lx, ly, hole_cx, hole_cy, hole_hx, hole_hy, hole_r);
            let hole_sdf_dev = device_sdf_from_local(hole_sdf, hnx, hny, inv, m_scale);
            let cov = match inv_sigma_sqrt2 {
                Some(inv_s) => {
                    (1.0 - gaussian_edge_coverage(hole_sdf_dev * inv_s)) * pad_cov * clip_cov
                }
                None => {
                    let hole_cov = (0.5 - hole_sdf_dev).clamp(0.0, 1.0);
                    (pad_cov - hole_cov).max(0.0) * clip_cov
                }
            };
            if cov > 0.002 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}
