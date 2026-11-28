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

#[inline(always)]
fn sanitize_dim(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

/// Post-sanitization [`CanvasSink`] emitting native `skia_safe::Canvas` vector draw calls onto an
/// isolated canvas surface (`NativeSkiaSurface` layer or dedicated offscreen `CpuSkiaSurface`).
pub struct SkiaNativeSink<'a> {
    canvas: &'a Canvas,
    width: f32,
    height: f32,
    paint: Paint,
    path_builder: PathBuilder,
}

impl<'a> SkiaNativeSink<'a> {
    #[inline]
    pub fn new(canvas: &'a Canvas, width: f32, height: f32) -> Self {
        let mut paint = Paint::default();
        paint.set_stroke_cap(PaintCap::Round);
        paint.set_stroke_join(PaintJoin::Round);
        Self {
            canvas,
            width: sanitize_dim(width),
            height: sanitize_dim(height),
            paint,
            path_builder: PathBuilder::new(),
        }
    }
}

impl CanvasSink for SkiaNativeSink<'_> {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        if self.width <= 0.0 || self.height <= 0.0 {
            return;
        }
        // Always runs on an isolated surface (`NativeSkiaSurface` save_layer or dedicated surface),
        // so `Clear` / `Src` replaces only this canvas's own pixels across `a == 0`, `0 < a < 255`,
        // and `a == 255` (matching `WebCanvas2dSink` and isolated RGBA `SkiaPixelSink`).
        let bounds = Rect::from_xywh(0.0, 0.0, self.width, self.height);
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_anti_alias(false);
        self.paint.set_blend_mode(if color.a == 0 {
            BlendMode::Clear
        } else {
            BlendMode::Src
        });
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_rect(bounds, &self.paint);
        self.paint.set_blend_mode(BlendMode::SrcOver);
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        if color.a == 0 {
            return;
        }
        self.paint.set_style(PaintStyle::Fill);
        self.paint.set_anti_alias(true);
        self.paint.set_color(to_skia_color(color));
        self.canvas.draw_rect(Rect::from_xywh(x, y, w, h), &self.paint);
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
        self.paint.set_anti_alias(true);
        self.paint.set_color(to_skia_color(color));
        let rect = Rect::from_xywh(x, y, w, h);
        if radius <= 0.0 {
            self.canvas.draw_rect(rect, &self.paint);
        } else {
            let rrect = RRect::new_rect_xy(rect, radius, radius);
            self.canvas.draw_rrect(rrect, &self.paint);
        }
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
            }
            return;
        }
        self.paint.set_style(PaintStyle::Stroke);
        self.paint.set_anti_alias(true);
        self.paint.set_stroke_width(width);
        self.paint.set_color(to_skia_color(color));

        // Opaque polylines emit zero-alloc round-capped segments directly onto `skia_safe::Canvas`;
        // translucent polylines reuse `path_builder` to avoid double-blending at interior vertices.
        if color.a == 255 {
            let mut iter = points.iter();
            if let Some(mut prev) = iter.next() {
                for cur in iter {
                    self.canvas.draw_line(prev, cur, &self.paint);
                    prev = cur;
                }
            }
        } else {
            let mut iter = points.iter();
            if let Some(first) = iter.next() {
                self.path_builder.reset();
                self.path_builder.move_to(first);
                for pt in iter {
                    self.path_builder.line_to(pt);
                }
                let path = self.path_builder.snapshot();
                self.canvas.draw_path(&path, &self.paint);
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
        let w = sanitize_dim(width);
        let h = sanitize_dim(height);
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let size = CanvasPaintSize::new(w, h, scale);
        (
            Self {
                inner: GuardedPainter::new(SkiaNativeSink::new(canvas, w, h)),
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
