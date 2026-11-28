//! Core types, bounds constants, and zero-alloc stack-buffered sparkline painter for
//! `nve.res.draw_list` draw lists and [`CanvasPainter`] streams.

use std::fmt;

use nve_abi::ColorRgba;

use crate::draw_list::DrawListRecorder;
use crate::painter::CanvasPainter;

/// Strict upper bound on operations per draw list to guard against corrupt/hostile payloads.
pub const MAX_DRAW_OPS: usize = 65_536;

/// Strict upper bound on vertices inside a single `StrokePolyline` operation.
pub const MAX_POLYLINE_POINTS: usize = 16_384;

/// Maximum live nested `ClipRect` depth passed to renderers before overflow balancing absorbs
/// deeper clips (matches generated host decoders across C++, Java, Dart, Swift, TS, Web, and Skia).
pub const MAX_CLIP_DEPTH: usize = 64;

/// Stack vertex buffer capacity for zero-allocation [`paint_sparkline`] painting.
const SPARKLINE_STACK_POINTS: usize = 256;

/// Cross-platform image scaling mode for `ComponentKind::Image` (`resizeMode` prop).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, nve_codec::NveCodec)]
pub enum ImageResizeMode {
    #[default]
    Cover = 0,
    Contain = 1,
    Stretch = 2,
    Center = 3,
}

impl ImageResizeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Stretch => "stretch",
            Self::Center => "center",
        }
    }

    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("cover") {
            Some(Self::Cover)
        } else if s.eq_ignore_ascii_case("contain") {
            Some(Self::Contain)
        } else if s.eq_ignore_ascii_case("stretch") {
            Some(Self::Stretch)
        } else if s.eq_ignore_ascii_case("center") {
            Some(Self::Center)
        } else {
            None
        }
    }
}

impl fmt::Display for ImageResizeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed vector drawing operation encoded inside a flat `nve.res.draw_list` stream.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasOp {
    Clear {
        color: ColorRgba,
    },
    FillRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: ColorRgba,
    },
    FillRoundedRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    },
    FillCircle {
        cx: f32,
        cy: f32,
        radius: f32,
        color: ColorRgba,
    },
    StrokeLine {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: ColorRgba,
    },
    StrokePolyline {
        points: Vec<(f32, f32)>,
        width: f32,
        color: ColorRgba,
    },
    ClipRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    RestoreClip,
}

impl CanvasOp {
    /// Replays this owned operation into any [`CanvasPainter`] with zero allocations.
    #[inline]
    pub fn replay(&self, painter: &mut (impl CanvasPainter + ?Sized)) {
        match self {
            Self::Clear { color } => painter.clear(*color),
            Self::FillRect { x, y, w, h, color } => painter.fill_rect(*x, *y, *w, *h, *color),
            Self::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => painter.fill_rounded_rect(*x, *y, *w, *h, *radius, *color),
            Self::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => painter.fill_circle(*cx, *cy, *radius, *color),
            Self::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => painter.stroke_line(*x0, *y0, *x1, *y1, *width, *color),
            Self::StrokePolyline {
                points,
                width,
                color,
            } => painter.stroke_polyline(points, *width, *color),
            Self::ClipRect { x, y, w, h } => painter.clip_rect(*x, *y, *w, *h),
            Self::RestoreClip => painter.restore_clip(),
        }
    }

    /// Generates a crisp telemetry sparkline sequence (background rounded frame, clip region,
    /// subtle horizontal reference guides, vector polyline curve, and highlighted terminal dot).
    ///
    /// Non-finite sample values are safely skipped so the resulting stream always passes
    /// strict [`nve_store::FlatResource::validate`] checks.
    pub fn sparkline(
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> Vec<CanvasOp> {
        let mut rec = DrawListRecorder::with_capacity(12);
        paint_sparkline(&mut rec, samples, width, height, stroke_color, background_color);
        rec.into_ops()
    }
}

/// Paints a crisp telemetry sparkline directly into any [`CanvasPainter`].
///
/// Allocates zero heap bytes for up to 256 finite samples (falling back to a `Vec` only when
/// `samples` contains more than 256 finite values).
pub fn paint_sparkline<P: CanvasPainter + ?Sized>(
    p: &mut P,
    samples: &[f32],
    width: f32,
    height: f32,
    stroke_color: ColorRgba,
    background_color: ColorRgba,
) {
    let w = sanitize_positive(width, 16.0);
    let h = sanitize_positive(height, 16.0);
    let pad_x = (w * 0.04).clamp(4.0, 12.0);
    let pad_y = (h * 0.14).clamp(4.0, 12.0);
    let plot_w = (w - pad_x * 2.0).max(1.0);
    let plot_h = (h - pad_y * 2.0).max(1.0);

    let grid_color = ColorRgba::rgba(148, 163, 184, 52);
    let baseline_color = ColorRgba::rgba(148, 163, 184, 90);

    p.clear(ColorRgba::TRANSPARENT);
    p.fill_rounded_rect(0.0, 0.0, w, h, 8.0_f32.min(h * 0.25), background_color);
    p.clip_rect(1.0, 1.0, (w - 2.0).max(1.0), (h - 2.0).max(1.0));

    // Subtle horizontal telemetry guide rails (top / mid / bottom).
    let mid_y = pad_y + plot_h * 0.5;
    p.stroke_line(pad_x, pad_y, pad_x + plot_w, pad_y, 1.0, grid_color);
    p.stroke_line(pad_x, mid_y, pad_x + plot_w, mid_y, 1.0, grid_color);
    p.stroke_line(
        pad_x,
        pad_y + plot_h,
        pad_x + plot_w,
        pad_y + plot_h,
        1.0,
        baseline_color,
    );

    // Pass 1: count finite samples and compute [min, max] with zero heap allocations.
    let mut count = 0usize;
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for &v in samples {
        if !v.is_finite() {
            continue;
        }
        if v < min_v {
            min_v = v;
        }
        if v > max_v {
            max_v = v;
        }
        count += 1;
        if count == MAX_POLYLINE_POINTS {
            break;
        }
    }

    if count == 0 {
        p.restore_clip();
        return;
    }

    if count == 1 {
        let cy = pad_y + plot_h * 0.5;
        p.stroke_line(pad_x, cy, pad_x + plot_w, cy, 2.0, stroke_color);
        p.fill_circle(pad_x + plot_w, cy, 3.5, stroke_color);
        p.restore_clip();
        return;
    }

    let min_d = f64::from(min_v);
    let span = f64::from(max_v) - min_d;
    let denom = (count - 1) as f32;
    let map_point = |idx: usize, v: f32| -> (f32, f32) {
        let t = idx as f32 / denom;
        let norm = if !span.is_finite() || span.abs() <= f64::from(f32::EPSILON) {
            0.5
        } else {
            ((f64::from(v) - min_d) / span).clamp(0.0, 1.0) as f32
        };
        (pad_x + t * plot_w, pad_y + (1.0 - norm) * plot_h)
    };

    let emit_series = |p: &mut P, points: &[(f32, f32)]| {
        if let Some(&(last_x, last_y)) = points.last() {
            let bar_w = (plot_w / count as f32).clamp(3.0, 10.0);
            let bar_h = (pad_y + plot_h - last_y).max(2.0);
            p.fill_rect(
                (last_x - bar_w * 0.5).max(pad_x),
                last_y,
                bar_w,
                bar_h,
                ColorRgba::rgba(stroke_color.r, stroke_color.g, stroke_color.b, 48),
            );
            p.stroke_polyline(points, 2.0, stroke_color);
            p.fill_circle(last_x, last_y, 3.5, stroke_color);
        }
    };

    if count <= SPARKLINE_STACK_POINTS {
        let mut stack_pts = [(0.0f32, 0.0f32); SPARKLINE_STACK_POINTS];
        let mut idx = 0usize;
        for &v in samples {
            if !v.is_finite() {
                continue;
            }
            stack_pts[idx] = map_point(idx, v);
            idx += 1;
            if idx == count {
                break;
            }
        }
        emit_series(p, &stack_pts[..count]);
    } else {
        let mut heap_pts = Vec::with_capacity(count);
        let mut idx = 0usize;
        for &v in samples {
            if !v.is_finite() {
                continue;
            }
            heap_pts.push(map_point(idx, v));
            idx += 1;
            if idx == count {
                break;
            }
        }
        emit_series(p, &heap_pts);
    }

    p.restore_clip();
}

#[inline]
fn sanitize_positive(v: f32, fallback: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        fallback
    }
}
