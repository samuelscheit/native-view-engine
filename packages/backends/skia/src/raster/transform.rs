//! 2D affine transform stack and exact inverse-mapped anti-aliased rounded-rect rasterization
//! for the software rasterizer.

use nve_abi::ColorRgba;
use nve_components::transform::{affine_concat, affine_inverse_map_point, affine_map_point};
use nve_engine::LayoutRect;

use crate::raster::clip::{rounded_rect_sdf, ClipStack};
use crate::raster::PixelBuffer;

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone)]
pub(crate) struct TransformStack {
    entries: Vec<[f32; 6]>,
}

impl TransformStack {
    pub(crate) fn new() -> Self {
        let mut entries = Vec::with_capacity(8);
        entries.push(IDENTITY);
        Self { entries }
    }

    #[inline]
    pub(crate) fn current(&self) -> &[f32; 6] {
        self.entries.last().unwrap_or(&IDENTITY)
    }

    #[inline]
    pub(crate) fn is_identity(&self) -> bool {
        let m = self.current();
        (m[0] - 1.0).abs() <= 1e-6
            && m[1].abs() <= 1e-6
            && m[2].abs() <= 1e-6
            && (m[3] - 1.0).abs() <= 1e-6
            && m[4].abs() <= 1e-6
            && m[5].abs() <= 1e-6
    }

    /// Pushes a logical-space affine matrix `[a, b, c, d, tx, ty]` scaled into physical pixels.
    pub(crate) fn push_logical(&mut self, logical: [f32; 6], scale: f32) {
        let phys = [
            logical[0],
            logical[1],
            logical[2],
            logical[3],
            logical[4] * scale,
            logical[5] * scale,
        ];
        let next = affine_concat(self.current(), &phys);
        self.entries.push(next);
    }

    pub(crate) fn pop(&mut self) {
        if self.entries.len() > 1 {
            self.entries.pop();
        }
    }
}

/// Maps a physical-pixel rect to its transformed axis-aligned bounding box.
pub(crate) fn transformed_aabb(m: &[f32; 6], rect: LayoutRect, pad: f32) -> (f32, f32, f32, f32) {
    let x0 = rect.x - pad;
    let y0 = rect.y - pad;
    let x1 = rect.x + rect.width + pad;
    let y1 = rect.y + rect.height + pad;
    let corners = [
        affine_map_point(m, x0, y0),
        affine_map_point(m, x1, y0),
        affine_map_point(m, x1, y1),
        affine_map_point(m, x0, y1),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (cx, cy) in corners {
        min_x = min_x.min(cx);
        min_y = min_y.min(cy);
        max_x = max_x.max(cx);
        max_y = max_y.max(cy);
    }
    (min_x, min_y, max_x, max_y)
}

/// Maps a logical rect through the current physical transform's AABB (used for text/image/shadow).
pub(crate) fn map_logical_rect_approx(m: &[f32; 6], rect: LayoutRect, scale: f32) -> LayoutRect {
    let phys = LayoutRect {
        x: rect.x * scale,
        y: rect.y * scale,
        width: rect.width * scale,
        height: rect.height * scale,
    };
    let (x0, y0, x1, y1) = transformed_aabb(m, phys, 0.0);
    LayoutRect {
        x: x0 / scale,
        y: y0 / scale,
        width: (x1 - x0).max(0.0) / scale,
        height: (y1 - y0).max(0.0) / scale,
    }
}

/// Anti-aliased transformed rounded-rect fill via exact inverse-affine SDF evaluation.
pub(crate) fn fill_rounded_rect_transformed(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    m: &[f32; 6],
    rect: LayoutRect,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || color.a == 0 || opacity <= 0.0 || clip.is_empty() {
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
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

/// Parameters for an anti-aliased transformed rounded-rectangle stroke.
pub(crate) struct TransformedStroke {
    pub rect: LayoutRect,
    pub radius: f32,
    pub width: f32,
    pub color: ColorRgba,
    pub opacity: f32,
}

/// Anti-aliased transformed rounded-rect stroke via exact inverse-affine SDF evaluation.
pub(crate) fn stroke_rounded_rect_transformed(
    buf: &mut PixelBuffer,
    clip: &ClipStack,
    m: &[f32; 6],
    stroke: TransformedStroke,
) {
    let TransformedStroke {
        rect,
        radius,
        width: stroke_width,
        color,
        opacity,
    } = stroke;
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
    let (bx0, by0, bx1, by1) = transformed_aabb(m, rect, hw + 1.5);
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
            let band_dist = sdf.abs() - hw;
            let cov = (0.5 - band_dist).clamp(0.0, 1.0) * clip_cov;
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}
