//! [`SkiaPixelSink`] (`CanvasSink`) and [`SkiaCanvasPainter`] (`GuardedPainter`) unifying
//! software rasterization, GPU plugin texture caching, recorded replay, and direct painting.

use nve_abi::ColorRgba;
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx};
use nve_canvas::{
    CanvasPaintSize, CanvasPainter, CanvasSink, GuardedPainter, PolylinePoints, MAX_CLIP_DEPTH,
};
use nve_engine::LayoutRect;

use crate::raster::{clear_rect_overwrite, fill_circle, fill_rect, stroke_line, ClipRectPx};

/// Post-sanitization `PixelBuffer` [`CanvasSink`] with an inline stack clip array (0 heap allocs).
pub struct SkiaPixelSink<'a> {
    buffer: &'a mut PixelBuffer,
    ox: f32,
    oy: f32,
    width_px: f32,
    height_px: f32,
    scale: f32,
    opacity: f32,
    bw: i32,
    bh: i32,
    clip_stack: [ClipRectPx; MAX_CLIP_DEPTH + 1],
    clip_depth: usize,
}

impl<'a> SkiaPixelSink<'a> {
    fn try_new(
        buffer: &'a mut PixelBuffer,
        rect: LayoutRect,
        clip_rect: Option<LayoutRect>,
        scale_factor: f32,
        opacity: f32,
    ) -> Option<(Self, CanvasPaintSize)> {
        if !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
            || !opacity.is_finite()
            || opacity <= 0.0
        {
            return None;
        }
        let bw = buffer.width() as i32;
        let bh = buffer.height() as i32;
        let mut base_clip = ClipRectPx {
            x0: (rect.x.floor() as i32).clamp(0, bw),
            y0: (rect.y.floor() as i32).clamp(0, bh),
            x1: ((rect.x + rect.width).ceil() as i32).clamp(0, bw),
            y1: ((rect.y + rect.height).ceil() as i32).clamp(0, bh),
        };
        if let Some(cr) = clip_rect
            && cr.x.is_finite()
            && cr.y.is_finite()
            && cr.width.is_finite()
            && cr.height.is_finite()
        {
            let outer = ClipRectPx {
                x0: (cr.x.floor() as i32).clamp(0, bw),
                y0: (cr.y.floor() as i32).clamp(0, bh),
                x1: ((cr.x + cr.width).ceil() as i32).clamp(0, bw),
                y1: ((cr.y + cr.height).ceil() as i32).clamp(0, bh),
            };
            base_clip = base_clip.intersect(outer);
        }
        if base_clip.is_empty() {
            return None;
        }
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let mut clip_stack = [ClipRectPx::default(); MAX_CLIP_DEPTH + 1];
        clip_stack[0] = base_clip;
        let size = CanvasPaintSize::new(rect.width / scale, rect.height / scale, scale);
        Some((
            Self {
                buffer,
                ox: rect.x,
                oy: rect.y,
                width_px: rect.width,
                height_px: rect.height,
                scale,
                opacity,
                bw,
                bh,
                clip_stack,
                clip_depth: 0,
            },
            size,
        ))
    }

    #[inline(always)]
    fn active_clip(&self) -> ClipRectPx {
        self.clip_stack[self.clip_depth]
    }
}

impl CanvasSink for SkiaPixelSink<'_> {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        let clip = self.active_clip();
        if color.a == 0 {
            clear_rect_overwrite(self.buffer, clip, color);
        } else {
            fill_rect(
                self.buffer,
                clip,
                self.ox,
                self.oy,
                self.width_px,
                self.height_px,
                0.0,
                color,
                self.opacity,
            );
        }
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        fill_rect(
            self.buffer,
            self.active_clip(),
            self.ox + x * self.scale,
            self.oy + y * self.scale,
            w * self.scale,
            h * self.scale,
            0.0,
            color,
            self.opacity,
        );
    }

    #[inline]
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) {
        fill_rect(
            self.buffer,
            self.active_clip(),
            self.ox + x * self.scale,
            self.oy + y * self.scale,
            w * self.scale,
            h * self.scale,
            radius * self.scale,
            color,
            self.opacity,
        );
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        fill_circle(
            self.buffer,
            self.active_clip(),
            self.ox + cx * self.scale,
            self.oy + cy * self.scale,
            radius * self.scale,
            color,
            self.opacity,
        );
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        stroke_line(
            self.buffer,
            self.active_clip(),
            self.ox + x0 * self.scale,
            self.oy + y0 * self.scale,
            self.ox + x1 * self.scale,
            self.oy + y1 * self.scale,
            width * self.scale,
            color,
            self.opacity,
        );
    }

    #[inline]
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        let clip = self.active_clip();
        if points.len() == 1 {
            if let Some((px, py)) = points.get(0) {
                fill_circle(
                    self.buffer,
                    clip,
                    self.ox + px * self.scale,
                    self.oy + py * self.scale,
                    (width * self.scale * 0.5).max(0.5),
                    color,
                    self.opacity,
                );
            }
            return;
        }
        let mut iter = points.iter();
        if let Some(mut prev) = iter.next() {
            for cur in iter {
                stroke_line(
                    self.buffer,
                    clip,
                    self.ox + prev.0 * self.scale,
                    self.oy + prev.1 * self.scale,
                    self.ox + cur.0 * self.scale,
                    self.oy + cur.1 * self.scale,
                    width * self.scale,
                    color,
                    self.opacity,
                );
                prev = cur;
            }
        }
    }

    #[inline]
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.clip_depth >= MAX_CLIP_DEPTH {
            return;
        }
        let next = if w > 0.0 && h > 0.0 {
            ClipRectPx {
                x0: ((self.ox + x * self.scale).floor() as i32).clamp(0, self.bw),
                y0: ((self.oy + y * self.scale).floor() as i32).clamp(0, self.bh),
                x1: ((self.ox + (x + w) * self.scale).ceil() as i32).clamp(0, self.bw),
                y1: ((self.oy + (y + h) * self.scale).ceil() as i32).clamp(0, self.bh),
            }
        } else {
            ClipRectPx::default()
        };
        let intersected = self.clip_stack[self.clip_depth].intersect(next);
        self.clip_depth += 1;
        self.clip_stack[self.clip_depth] = intersected;
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.clip_depth = self.clip_depth.saturating_sub(1);
    }
}

/// Guarded [`CanvasPainter`] painting directly onto a Skia / software [`PixelBuffer`].
pub struct SkiaCanvasPainter<'a> {
    inner: GuardedPainter<SkiaPixelSink<'a>>,
}

impl<'a> SkiaCanvasPainter<'a> {
    /// Creates a guarded painter targeting `buffer` within `rect` (physical pixel coordinates).
    /// Returns `None` when `rect`, `clip_rect`, or `opacity` produces an empty or invisible surface.
    #[inline]
    pub fn new(
        buffer: &'a mut PixelBuffer,
        rect: LayoutRect,
        clip_rect: Option<LayoutRect>,
        scale_factor: f32,
        opacity: f32,
    ) -> Option<(Self, CanvasPaintSize)> {
        let (sink, size) = SkiaPixelSink::try_new(buffer, rect, clip_rect, scale_factor, opacity)?;
        Some((
            Self {
                inner: GuardedPainter::new(sink),
            },
            size,
        ))
    }

    /// Creates a guarded painter directly from a [`SkiaSystemPaintCtx`].
    #[inline]
    pub fn try_from_ctx(ctx: &'a mut SkiaSystemPaintCtx<'_>) -> Option<(Self, CanvasPaintSize)> {
        Self::new(
            ctx.buffer,
            ctx.rect,
            ctx.clip_rect,
            ctx.scale_factor,
            ctx.opacity,
        )
    }
}

impl CanvasPainter for SkiaCanvasPainter<'_> {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        self.inner.clear(color);
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        self.inner.fill_rect(x, y, w, h, color);
    }

    #[inline]
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) {
        self.inner.fill_rounded_rect(x, y, w, h, radius, color);
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        self.inner.fill_circle(cx, cy, radius, color);
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        self.inner.stroke_line(x0, y0, x1, y1, width, color);
    }

    #[inline]
    fn stroke_polyline_points(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        self.inner.stroke_polyline_points(points, width, color);
    }

    #[inline]
    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32, color: ColorRgba) {
        self.inner.stroke_polyline(points, width, color);
    }

    #[inline]
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.inner.clip_rect(x, y, w, h);
    }

    #[inline]
    fn restore_clip(&mut self) {
        self.inner.restore_clip();
    }
}
