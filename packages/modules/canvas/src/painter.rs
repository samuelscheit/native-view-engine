//! Object-safe [`CanvasPainter`] trait, low-level post-sanitization [`CanvasSink`], and
//! [`GuardedPainter`] enforcing one canonical sanitization + `MAX_CLIP_DEPTH = 64` + RAII `Drop`
//! clip-unwind contract across `DrawListRecorder`, Skia, and Web.

use nve_abi::ColorRgba;

use crate::desc::ClipDepthTracker;
use crate::polyline::{PolylineBacking, PolylinePoints};
use crate::MAX_POLYLINE_POINTS;

/// Logical canvas dimensions and backing scale factor supplied to direct canvas painters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasPaintSize {
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
}

impl CanvasPaintSize {
    #[inline]
    pub const fn new(width: f32, height: f32, scale_factor: f32) -> Self {
        Self {
            width,
            height,
            scale_factor,
        }
    }
}

/// Object-safe vector drawing surface implemented by [`crate::DrawListRecorder`],
/// [`GuardedPainter`], and backend painters (`SkiaCanvasPainter`, `WebCanvas2dPainter`).
///
/// Write one `paint_scene(p: &mut (impl CanvasPainter + ?Sized), size: CanvasPaintSize, ...)`
/// function and drive it either directly in-process (zero `DrawList` encode/decode) or into a
/// recorded [`crate::DrawList`] for SSR, `NVE2` wire, `Frame0File`, or out-of-process hosts.
pub trait CanvasPainter {
    fn clear(&mut self, color: ColorRgba);
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba);
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    );
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba);
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba);
    fn stroke_polyline_points(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba);
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn restore_clip(&mut self);

    /// Ergonomic object-safe slice entrypoint; forwards zero-copy to [`Self::stroke_polyline_points`].
    #[inline]
    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32, color: ColorRgba) {
        self.stroke_polyline_points(PolylinePoints::from_slice(points), width, color);
    }
}

/// Lets `&mut dyn CanvasPainter` satisfy generic `&mut impl CanvasPainter` / `impl CanvasPainter` bounds.
impl<P: CanvasPainter + ?Sized> CanvasPainter for &mut P {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        (**self).clear(color);
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        (**self).fill_rect(x, y, w, h, color);
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
        (**self).fill_rounded_rect(x, y, w, h, radius, color);
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        (**self).fill_circle(cx, cy, radius, color);
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        (**self).stroke_line(x0, y0, x1, y1, width, color);
    }

    #[inline]
    fn stroke_polyline_points(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        (**self).stroke_polyline_points(points, width, color);
    }

    #[inline]
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        (**self).clip_rect(x, y, w, h);
    }

    #[inline]
    fn restore_clip(&mut self) {
        (**self).restore_clip();
    }

    #[inline]
    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32, color: ColorRgba) {
        (**self).stroke_polyline(points, width, color);
    }
}

// Compile-time object-safety lock.
const _: fn(&mut dyn CanvasPainter) = |_| {};

/// Low-level post-sanitization, post-clip-balanced backend sink wrapped by [`GuardedPainter`].
///
/// Every call delivered by [`GuardedPainter`] upholds:
/// - `fill_rect` / `fill_rounded_rect`: finite `x, y, w, h`, `w > 0.0`, `h > 0.0`, and finite
///   `0.0 <= radius <= min(w, h) * 0.5`.
/// - `fill_circle`: finite `cx, cy, radius` with `radius > 0.0`.
/// - `stroke_line`: finite `x0, y0, x1, y1, width` with `width > 0.0`.
/// - `stroke_polyline`: finite `width > 0.0`, `1 <= points.len() <= MAX_POLYLINE_POINTS`, and
///   every vertex finite.
/// - `push_clip_rect` / `pop_clip`: strictly balanced with live depth `<= MAX_CLIP_DEPTH`,
///   finite `(x, y)` and finite non-negative `(w, h)`, and guaranteed RAII unwind on [`GuardedPainter::drop`].
pub trait CanvasSink {
    fn clear(&mut self, color: ColorRgba);
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba);
    fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    );
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba);
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba);
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba);
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn pop_clip(&mut self);
}

impl<S: CanvasSink + ?Sized> CanvasSink for &mut S {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        (**self).clear(color);
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        (**self).fill_rect(x, y, w, h, color);
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
        (**self).fill_rounded_rect(x, y, w, h, radius, color);
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        (**self).fill_circle(cx, cy, radius, color);
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        (**self).stroke_line(x0, y0, x1, y1, width, color);
    }

    #[inline]
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        (**self).stroke_polyline(points, width, color);
    }

    #[inline]
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        (**self).push_clip_rect(x, y, w, h);
    }

    #[inline]
    fn pop_clip(&mut self) {
        (**self).pop_clip();
    }
}

/// Zero-allocation, zero-`unsafe` canonical guard wrapping any [`CanvasSink`].
///
/// Enforces identical geometry sanitization, NaN polyline splitting, `MAX_CLIP_DEPTH = 64`
/// overflow-balanced clipping, and RAII `Drop` clip unwinding across every backend and recorder.
#[derive(Debug, Default)]
pub struct GuardedPainter<S: CanvasSink> {
    sink: S,
    clips: ClipDepthTracker,
}

impl<S: CanvasSink + Clone> Clone for GuardedPainter<S> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            sink: self.sink.clone(),
            clips: self.clips,
        }
    }
}

impl<S: CanvasSink> GuardedPainter<S> {
    #[inline]
    pub const fn new(sink: S) -> Self {
        Self {
            sink,
            clips: ClipDepthTracker::new(),
        }
    }

    #[inline]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    #[inline]
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    #[inline]
    pub const fn clip_depth(&self) -> usize {
        self.clips.depth()
    }

    #[inline]
    pub const fn clip_overflow(&self) -> usize {
        self.clips.overflow()
    }

    /// Explicitly drains all remaining live clips, emitting one [`CanvasSink::pop_clip`] per open
    /// clip. Idempotent; [`Drop`] calls this automatically so panics also leave the sink balanced.
    #[inline]
    pub fn unwind_clips(&mut self) {
        for _ in 0..self.clips.drain_open() {
            self.sink.pop_clip();
        }
    }
}

impl<S: CanvasSink> Drop for GuardedPainter<S> {
    #[inline]
    fn drop(&mut self) {
        self.unwind_clips();
    }
}

#[inline(always)]
fn all_finite_4(a: f32, b: f32, c: f32, d: f32) -> bool {
    a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite()
}

#[inline(always)]
pub(crate) fn sanitize_non_negative(v: f32) -> f32 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

impl<S: CanvasSink> CanvasPainter for GuardedPainter<S> {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        // Always forwarded — transparent clear (`color.a == 0`) overwrites prior pixels.
        self.sink.clear(color);
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        if !all_finite_4(x, y, w, h) || w <= 0.0 || h <= 0.0 {
            return;
        }
        self.sink.fill_rect(x, y, w, h, color);
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
        if !all_finite_4(x, y, w, h) || w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = sanitize_non_negative(radius).min(w * 0.5).min(h * 0.5);
        self.sink.fill_rounded_rect(x, y, w, h, r, color);
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        if !cx.is_finite() || !cy.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }
        self.sink.fill_circle(cx, cy, radius, color);
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        if !all_finite_4(x0, y0, x1, y1) || !width.is_finite() || width <= 0.0 {
            return;
        }
        self.sink.stroke_line(x0, y0, x1, y1, width, color);
    }

    #[inline]
    fn stroke_polyline_points(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        if !width.is_finite() || width <= 0.0 || points.is_empty() {
            return;
        }
        match points.backing() {
            PolylineBacking::Words(_) => {
                // Sealed at `ValidatedWords` ingress: already finite and `<= MAX_POLYLINE_POINTS`.
                self.sink.stroke_polyline(points, width, color);
            }
            PolylineBacking::Slice(raw) => {
                let slice = &raw[..raw.len().min(MAX_POLYLINE_POINTS)];
                if slice.iter().all(|&(px, py)| px.is_finite() && py.is_finite()) {
                    self.sink
                        .stroke_polyline(PolylinePoints::from_slice(slice), width, color);
                } else {
                    for segment in slice.split(|&(px, py)| !px.is_finite() || !py.is_finite()) {
                        if !segment.is_empty() {
                            self.sink.stroke_polyline(
                                PolylinePoints::from_slice(segment),
                                width,
                                color,
                            );
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32, color: ColorRgba) {
        self.stroke_polyline_points(PolylinePoints::from_slice(points), width, color);
    }

    #[inline]
    fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.clips.push() {
            if all_finite_4(x, y, w, h) && w > 0.0 && h > 0.0 {
                self.sink.push_clip_rect(x, y, w, h);
            } else {
                // Keep live clip stack pairing intact with an empty clip region.
                self.sink.push_clip_rect(0.0, 0.0, 0.0, 0.0);
            }
        }
    }

    #[inline]
    fn restore_clip(&mut self) {
        if self.clips.pop() {
            self.sink.pop_clip();
        }
    }
}
