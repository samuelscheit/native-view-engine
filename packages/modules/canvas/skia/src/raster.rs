//! Pure `PixelBuffer` software rasterization primitives for `nve-canvas-skia`.

use nve_abi::ColorRgba;
use nve_backend_skia::PixelBuffer;

/// Clamped physical-pixel clip rectangle (`[x0, x1) x [y0, y1)`).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct ClipRectPx {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
}

impl ClipRectPx {
    #[inline]
    pub(crate) fn intersect(self, other: Self) -> Self {
        Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
    }

    #[inline]
    pub(crate) const fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

#[inline]
pub(crate) fn clear_rect_overwrite(buf: &mut PixelBuffer, clip: ClipRectPx, color: ColorRgba) {
    if !clip.is_empty() {
        buf.fill_rect_rgba(clip.x0, clip.y0, clip.x1, clip.y1, color);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rect(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if clip.is_empty()
        || !x.is_finite()
        || !y.is_finite()
        || !w.is_finite()
        || !h.is_finite()
        || w <= 0.0
        || h <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let x0 = (x.floor() as i32).max(clip.x0);
    let x1 = ((x + w).ceil() as i32).min(clip.x1);
    let y0 = (y.floor() as i32).max(clip.y0);
    let y1 = ((y + h).ceil() as i32).min(clip.y1);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let raw_r = if radius.is_finite() { radius } else { 0.0 };
    let r = raw_r.clamp(0.0, (w * 0.5).min(h * 0.5));
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let hx = w * 0.5 - r;
    let hy = h * 0.5 - r;

    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            let cov = if r <= 0.0 {
                1.0
            } else {
                let dx = (fx - cx).abs() - hx;
                let dy = (fy - cy).abs() - hy;
                let dist = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
                (0.5 - dist).clamp(0.0, 1.0)
            };
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

pub(crate) fn fill_circle(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    cx: f32,
    cy: f32,
    radius: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if clip.is_empty()
        || !cx.is_finite()
        || !cy.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let x0 = ((cx - radius - 1.0).floor() as i32).max(clip.x0);
    let x1 = ((cx + radius + 1.0).ceil() as i32).min(clip.x1);
    let y0 = ((cy - radius - 1.0).floor() as i32).max(clip.y0);
    let y1 = ((cy + radius + 1.0).ceil() as i32).min(clip.y1);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for py in y0..y1 {
        let dy = (py as f32 + 0.5) - cy;
        for px in x0..x1 {
            let dx = (px as f32 + 0.5) - cx;
            let dist = dx.hypot(dy) - radius;
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stroke_line(
    buf: &mut PixelBuffer,
    clip: ClipRectPx,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: ColorRgba,
    opacity: f32,
) {
    if clip.is_empty()
        || !x0.is_finite()
        || !y0.is_finite()
        || !x1.is_finite()
        || !y1.is_finite()
        || !width.is_finite()
        || width <= 0.0
        || color.a == 0
        || !opacity.is_finite()
        || opacity <= 0.0
    {
        return;
    }
    let half = (width * 0.5).max(0.5);
    let min_x = ((x0.min(x1) - half - 1.0).floor() as i32).max(clip.x0);
    let max_x = ((x0.max(x1) + half + 1.0).ceil() as i32).min(clip.x1);
    let min_y = ((y0.min(y1) - half - 1.0).floor() as i32).max(clip.y0);
    let max_y = ((y0.max(y1) + half + 1.0).ceil() as i32).min(clip.y1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let vx = x1 - x0;
    let vy = y1 - y0;
    let len2 = vx * vx + vy * vy;

    for py in min_y..max_y {
        let fy = py as f32 + 0.5;
        for px in min_x..max_x {
            let fx = px as f32 + 0.5;
            let t = if len2 > 1e-6 {
                (((fx - x0) * vx + (fy - y0) * vy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dist = (fx - (x0 + t * vx)).hypot(fy - (y0 + t * vy)) - half;
            let cov = (0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                buf.blend_pixel(px, py, color, cov * opacity);
            }
        }
    }
}
