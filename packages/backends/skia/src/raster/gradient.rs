//! Anti-aliased SDF-clipped linear and radial gradient fills (axis-aligned and affine-transformed)
//! for the pure-Rust NVE software rasterizer.

use std::borrow::Cow;

use nve_abi::ColorRgba;
use nve_components::paint::{
    normalize_stops, sample_normalized_stops, stops_are_normalized, BackgroundFill, GradientStop,
};
use nve_components::transform::affine_inverse_map_point;
use nve_engine::LayoutRect;

use super::clip::{rounded_rect_sdf, ClipStack};
use super::transform::{fill_rounded_rect_transformed, transformed_aabb};
use super::{fill_rounded_rect, PixelBuffer};

enum PreparedGradient<'a> {
    Solid(ColorRgba),
    Linear {
        p0: (f32, f32),
        dx: f32,
        dy: f32,
        inv_len_sq: f32,
        stops: Cow<'a, [GradientStop]>,
    },
    Radial {
        cx: f32,
        cy: f32,
        inv_rx: f32,
        inv_ry: f32,
        degenerate: bool,
        stops: Cow<'a, [GradientStop]>,
    },
}

impl<'a> PreparedGradient<'a> {
    fn prepare(rect: LayoutRect, fill: &'a BackgroundFill) -> Option<Self> {
        match fill {
            BackgroundFill::Solid(c) => (c.a > 0).then_some(Self::Solid(*c)),
            BackgroundFill::Linear(lg) => {
                if lg.stops.is_empty() {
                    return None;
                }
                if lg.stops.len() == 1 {
                    let c = lg.stops[0].color;
                    return (c.a > 0).then_some(Self::Solid(c));
                }
                let stops = prepare_stops(&lg.stops);
                let (p0, p1) = lg.endpoints(rect.x, rect.y, rect.width, rect.height);
                let dx = p1.0 - p0.0;
                let dy = p1.1 - p0.1;
                let len_sq = dx * dx + dy * dy;
                let inv_len_sq = if len_sq > 1e-6 { 1.0 / len_sq } else { 0.0 };
                Some(Self::Linear {
                    p0,
                    dx,
                    dy,
                    inv_len_sq,
                    stops,
                })
            }
            BackgroundFill::Radial(rg) => {
                if rg.stops.is_empty() {
                    return None;
                }
                if rg.stops.len() == 1 {
                    let c = rg.stops[0].color;
                    return (c.a > 0).then_some(Self::Solid(c));
                }
                let stops = prepare_stops(&rg.stops);
                let (cx, cy, rx, ry) = rg.geometry(rect.x, rect.y, rect.width, rect.height);
                let degenerate = rx <= 1e-6 || ry <= 1e-6;
                Some(Self::Radial {
                    cx,
                    cy,
                    inv_rx: if degenerate { 0.0 } else { 1.0 / rx },
                    inv_ry: if degenerate { 0.0 } else { 1.0 / ry },
                    degenerate,
                    stops,
                })
            }
        }
    }

    #[inline]
    fn sample(&self, lx: f32, ly: f32) -> ColorRgba {
        match self {
            Self::Solid(c) => *c,
            Self::Linear {
                p0,
                dx,
                dy,
                inv_len_sq,
                stops,
            } => {
                let t = if *inv_len_sq > 0.0 {
                    ((lx - p0.0) * *dx + (ly - p0.1) * *dy) * *inv_len_sq
                } else {
                    0.0
                };
                sample_normalized_stops(stops, t)
            }
            Self::Radial {
                cx,
                cy,
                inv_rx,
                inv_ry,
                degenerate,
                stops,
            } => {
                let t = if *degenerate {
                    if (lx - *cx).hypot(ly - *cy) <= 1e-6 {
                        0.0
                    } else {
                        1.0
                    }
                } else {
                    let nx = (lx - *cx) * *inv_rx;
                    let ny = (ly - *cy) * *inv_ry;
                    (nx * nx + ny * ny).sqrt()
                };
                sample_normalized_stops(stops, t)
            }
        }
    }
}

#[inline]
fn prepare_stops(stops: &[GradientStop]) -> Cow<'_, [GradientStop]> {
    if stops_are_normalized(stops) {
        Cow::Borrowed(stops)
    } else {
        Cow::Owned(normalize_stops(stops.to_vec()))
    }
}

/// Fills an axis-aligned rounded rectangle with a linear or radial gradient, SDF-clipped with
/// smooth anti-aliased edges and active `ClipStack` support.
pub(crate) fn fill_rounded_rect_gradient(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    rect: LayoutRect,
    radius: f32,
    gradient: &BackgroundFill,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let Some(prepared) = PreparedGradient::prepare(rect, gradient) else {
        return;
    };
    if let PreparedGradient::Solid(color) = prepared {
        fill_rounded_rect(buf, clip, rect, radius, color, opacity);
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

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
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
            let sdf = rounded_rect_sdf(fx, fy, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                let color = prepared.sample(fx, fy);
                if color.a > 0 {
                    buf.blend_pixel(px, py, color, cov * opacity);
                }
            }
        }
    }
}

/// Fills an affine-transformed rounded rectangle with a linear or radial gradient by evaluating
/// both SDF coverage and gradient coordinates in inverse-mapped local box space.
pub(crate) fn fill_rounded_rect_gradient_transformed(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    m: &[f32; 6],
    rect: LayoutRect,
    radius: f32,
    gradient: &BackgroundFill,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || opacity <= 0.0 || clip.is_empty() {
        return;
    }
    let Some(prepared) = PreparedGradient::prepare(rect, gradient) else {
        return;
    };
    if let PreparedGradient::Solid(color) = prepared {
        fill_rounded_rect_transformed(buf, clip, m, rect, radius, color, opacity);
        return;
    }

    let (bx0, by0, bx1, by1) = transformed_aabb(m, rect, 1.0);
    let [cx0, cy0, cx1, cy1] = clip.bounds();
    let min_x = (bx0.floor() as i32).clamp(cx0, cx1);
    let max_x = (bx1.ceil() as i32).clamp(cx0, cx1);
    let min_y = (by0.floor() as i32).clamp(cy0, cy1);
    let max_y = (by1.ceil() as i32).clamp(cy0, cy1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let hx = rect.width * 0.5;
    let hy = rect.height * 0.5;
    let cx = rect.x + hx;
    let cy = rect.y + hy;
    let r = radius.clamp(0.0, hx.min(hy));
    let rounded_clip = clip.has_rounded();

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
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
            let Some((lx, ly)) = affine_inverse_map_point(m, fx, fy) else {
                return;
            };
            let sdf = rounded_rect_sdf(lx, ly, cx, cy, hx, hy, r);
            let cov = (0.5 - sdf).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                let color = prepared.sample(lx, ly);
                if color.a > 0 {
                    buf.blend_pixel(px, py, color, cov * opacity);
                }
            }
        }
    }
}
