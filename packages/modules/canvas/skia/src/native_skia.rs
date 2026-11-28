//! Native `skia_safe::Canvas` hardware/CPU Skia sink (`--features skia`).
//!
//! Invoked directly from `SkiaTableSystem::paint_skia_canvas` for in-process direct canvases so
//! 120 Hz animating vector graphics emit native Skia GPU/vector draw calls without CPU
//! `PixelBuffer` rasterization or per-frame texture uploads.

use nve_abi::ColorRgba;
use nve_canvas::{
    CanvasPaintSize, CanvasPainter, CanvasSink, GuardedPainter, PolylinePoints,
};
use skia_safe::{
    BlendMode, Canvas, ClipOp, Color, Paint, PaintCap, PaintJoin, PaintStyle, PathBuilder, RRect,
    Rect,
};

#[inline(always)]
fn to_skia_color(c: ColorRgba) -> Color {
    Color::from_argb(c.a, c.r, c.g, c.b)
}

/// Post-sanitization [`CanvasSink`] emitting native `skia_safe::Canvas` vector draw calls.
pub struct SkiaNativeSink<'a> {
    canvas: &'a Canvas,
    width: f32,
    height: f32,
    paint: Paint,
    has_drawn: bool,
}

impl<'a> SkiaNativeSink<'a> {
    #[inline]
    pub fn new(canvas: &'a Canvas, width: f32, height: f32) -> Self {
        let mut paint = Paint::default();
        paint.set_stroke_cap(PaintCap::Round);
        paint.set_stroke_join(PaintJoin::Round);
        Self {
            canvas,
            width,
            height,
            paint,
            has_drawn: false,
        }
    }
}

impl CanvasSink for SkiaNativeSink<'_> {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        let bounds = Rect::from_xywh(0.0, 0.0, self.width, self.height);
        if color.a == 0 {
            // An initial transparent clear on a fresh clipped surface leaves parent backdrop
            // intact (matching a transparent `PixelBuffer` composited with `SrcOver`); a later
            // transparent clear overwrites previously drawn pixels inside the active clip rect.
            if self.has_drawn {
                self.paint.set_style(PaintStyle::Fill);
                self.paint.set_anti_alias(false);
                self.paint.set_blend_mode(BlendMode::Clear);
                self.paint.set_color(Color::TRANSPARENT);
                self.canvas.draw_rect(bounds, &self.paint);
                self.paint.set_blend_mode(BlendMode::SrcOver);
            }
            return;
        }
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_anti_alias(false);
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_rect(bounds, &self.paint);
        self.has_drawn = true;
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        if color.a == 0 {
            return;
        }
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_anti_alias(false);
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_rect(Rect::from_xywh(x, y, w, h), &self.paint);
        self.has_drawn = true;
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
        if color.a == 0 {
            return;
        }
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_color(to_skia_color(color));
        let rect = Rect::from_xywh(x, y, w, h);
        if radius <= 0.0 {
            self.paint.set_anti_alias(false);
            self.canvas.draw_rect(rect, &self.paint);
        } else {
            self.paint.set_anti_alias(true);
            let rrect = RRect::new_rect_xy(rect, radius, radius);
            self.canvas.draw_rrect(rrect, &self.paint);
        }
        self.has_drawn = true;
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        if color.a == 0 {
            return;
        }
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_anti_alias(true);
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_circle((cx, cy), radius, &self.paint);
        self.has_drawn = true;
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        if color.a == 0 {
            return;
        }
        self.paint.set_style(PaintStyle::Stroke);
        self.paint.set_anti_alias(true);
        self.paint.set_stroke_width(width);
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_line((x0, y0), (x1, y1), &self.paint);
        self.has_drawn = true;
    }

    #[inline]
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        if color.a == 0 {
            return;
        }
        if points.len() == 1 {
            if let Some((px, py)) = points.get(0) {
                self.paint.set_style(PaintStyle::Fill);
                self.paint.set_anti_alias(true);
                self.paint.set_color(to_skia_color(color));
                self.canvas
                    .draw_circle((px, py), (width * 0.5).max(0.5), &self.paint);
                self.has_drawn = true;
            }
            return;
        }
        self.paint.set_style(PaintStyle::Stroke);
        self.paint.set_anti_alias(true);
        self.paint.set_stroke_width(width);
        self.paint.set_color(to_skia_color(color));

        // Opaque polylines emit zero-alloc round-capped segments directly onto `skia_safe::Canvas`;
        // translucent polylines use a `PathBuilder` to avoid double-blending at interior vertices.
        if color.a == 255 {
            let mut iter = points.iter();
            if let Some(mut prev) = iter.next() {
                for cur in iter {
                    self.canvas.draw_line(prev, cur, &self.paint);
                    prev = cur;
                }
                self.has_drawn = true;
            }
        } else {
            let mut iter = points.iter();
            if let Some(first) = iter.next() {
                let mut builder = PathBuilder::new();
                builder.move_to(first);
                for pt in iter {
                    builder.line_to(pt);
                }
                let path = builder.detach();
                self.canvas.draw_path(&path, &self.paint);
                self.has_drawn = true;
            }
        }
    }

    #[inline]
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.canvas.save();
        self.canvas.clip_rect(
            Rect::from_xywh(x, y, w, h),
            Some(ClipOp::Intersect),
            Some(false),
        );
    }

    #[inline]
    fn pop_clip(&mut self) {
        self.canvas.restore();
    }
}

/// Guarded [`CanvasPainter`] emitting native hardware/CPU Skia draw calls onto a
/// [`skia_safe::Canvas`].
pub struct SkiaNativeCanvasPainter<'a> {
    inner: GuardedPainter<SkiaNativeSink<'a>>,
}

impl<'a> SkiaNativeCanvasPainter<'a> {
    #[inline]
    pub fn new(
        canvas: &'a Canvas,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> (Self, CanvasPaintSize) {
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let size = CanvasPaintSize::new(width, height, scale);
        (
            Self {
                inner: GuardedPainter::new(SkiaNativeSink::new(canvas, width, height)),
            },
            size,
        )
    }
}

impl CanvasPainter for SkiaNativeCanvasPainter<'_> {
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
