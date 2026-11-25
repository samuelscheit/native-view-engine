//! Zero-serialization immutable vector draw list (`DrawList`, backed by `Arc<[CanvasOp]>`)
//! and fluent direct recorder (`DrawListRecorder`) for in-process Rust rendering.
//!
//! Eliminates both base64 attribute encoding/decoding AND binary `NVE_DRAW1` encode/decode
//! round-trips on the Rust-to-Rust render hot path (`nve-dioxus` / `nve-docs` /
//! `studio-workspace` -> `HeadlessBackend`, `SkiaBackend`, `WebBackend`), while still offering
//! on-demand [`DrawList::to_nve_draw1_bytes`] for cross-language FFI hosts.

use std::ops::Deref;
use std::sync::Arc;

use nve_abi::ColorRgba;

use super::codec::{DrawListBuilder, DrawListDecoder};
use super::types::{sanitize_finite, CanvasOp, DrawListError, MAX_DRAW_OPS, MAX_POLYLINE_POINTS};

/// Immutable, `O(1)`-clonable typed vector draw list (`Arc<[CanvasOp]>`).
#[derive(Debug, Clone, PartialEq)]
pub struct DrawList {
    ops: Arc<[CanvasOp]>,
}

impl Default for DrawList {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl Deref for DrawList {
    type Target = [CanvasOp];

    #[inline]
    fn deref(&self) -> &[CanvasOp] {
        &self.ops
    }
}

impl<'a> IntoIterator for &'a DrawList {
    type Item = &'a CanvasOp;
    type IntoIter = std::slice::Iter<'a, CanvasOp>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.ops.iter()
    }
}

impl AsRef<[CanvasOp]> for DrawList {
    #[inline]
    fn as_ref(&self) -> &[CanvasOp] {
        &self.ops
    }
}

impl From<Vec<CanvasOp>> for DrawList {
    #[inline]
    fn from(ops: Vec<CanvasOp>) -> Self {
        Self::from_ops(ops)
    }
}

impl From<Arc<[CanvasOp]>> for DrawList {
    #[inline]
    fn from(ops: Arc<[CanvasOp]>) -> Self {
        Self { ops }
    }
}

impl DrawList {
    #[inline]
    pub fn empty() -> Self {
        Self {
            ops: Arc::from([] as [CanvasOp; 0]),
        }
    }

    #[inline]
    pub fn builder() -> DrawListRecorder {
        DrawListRecorder::new()
    }

    #[inline]
    pub fn with_capacity(op_capacity: usize) -> DrawListRecorder {
        DrawListRecorder::with_capacity(op_capacity)
    }

    /// Creates a [`DrawList`] from a vector of typed [`CanvasOp`]s without any binary encoding.
    #[inline]
    pub fn from_ops(ops: Vec<CanvasOp>) -> Self {
        Self {
            ops: Arc::from(ops),
        }
    }

    #[inline]
    pub fn from_arc(ops: Arc<[CanvasOp]>) -> Self {
        Self { ops }
    }

    #[inline]
    pub fn as_arc(&self) -> &Arc<[CanvasOp]> {
        &self.ops
    }

    #[inline]
    pub fn ops(&self) -> &[CanvasOp] {
        &self.ops
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    #[inline]
    pub fn op_count(&self) -> u32 {
        self.ops.len().min(u32::MAX as usize) as u32
    }

    /// `O(1)` pointer identity check for fast change-detection between frames.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.ops, &other.ops)
    }

    /// Builds a telemetry sparkline directly into a zero-serialization [`DrawList`].
    pub fn sparkline(
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> Self {
        Self::from_ops(CanvasOp::sparkline(
            samples,
            width,
            height,
            stroke_color,
            background_color,
        ))
    }

    /// Encodes this [`DrawList`] into cross-language `NVE_DRAW1` binary bytes (only needed when
    /// crossing an FFI boundary to C++/Swift/Kotlin/Dart/JS hosts).
    pub fn to_nve_draw1_bytes(&self) -> Vec<u8> {
        let mut builder = DrawListBuilder::with_capacity(self.ops.len() * 24 + 16);
        builder.extend_ops(&self.ops);
        builder.finish()
    }

    /// Decodes a cross-language `NVE_DRAW1` binary byte buffer into a [`DrawList`].
    pub fn from_nve_draw1_bytes(bytes: &[u8]) -> Result<Self, DrawListError> {
        Ok(Self::from_ops(DrawListDecoder::decode(bytes)?))
    }
}

/// Fluent zero-serialization recorder constructing a [`DrawList`] (`Arc<[CanvasOp]>`) directly
/// without binary encoding or base64 formatting.
#[derive(Debug, Clone, Default)]
pub struct DrawListRecorder {
    ops: Vec<CanvasOp>,
}

impl DrawListRecorder {
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    #[inline]
    pub fn with_capacity(op_capacity: usize) -> Self {
        Self {
            ops: Vec::with_capacity(op_capacity.min(MAX_DRAW_OPS)),
        }
    }

    #[inline]
    pub fn op_count(&self) -> u32 {
        self.ops.len().min(u32::MAX as usize) as u32
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    #[inline]
    fn push_checked(&mut self, op: CanvasOp) -> &mut Self {
        if self.ops.len() < MAX_DRAW_OPS {
            self.ops.push(op);
        }
        self
    }

    #[inline]
    pub fn clear(&mut self, color: ColorRgba) -> &mut Self {
        self.push_checked(CanvasOp::Clear { color })
    }

    #[inline]
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) -> &mut Self {
        self.push_checked(CanvasOp::FillRect {
            x: sanitize_finite(x),
            y: sanitize_finite(y),
            w: sanitize_finite(w),
            h: sanitize_finite(h),
            color,
        })
    }

    #[inline]
    pub fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: ColorRgba,
    ) -> &mut Self {
        self.push_checked(CanvasOp::FillRoundedRect {
            x: sanitize_finite(x),
            y: sanitize_finite(y),
            w: sanitize_finite(w),
            h: sanitize_finite(h),
            radius: sanitize_finite(radius),
            color,
        })
    }

    #[inline]
    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) -> &mut Self {
        self.push_checked(CanvasOp::FillCircle {
            cx: sanitize_finite(cx),
            cy: sanitize_finite(cy),
            radius: sanitize_finite(radius),
            color,
        })
    }

    #[inline]
    pub fn stroke_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        color: ColorRgba,
    ) -> &mut Self {
        self.push_checked(CanvasOp::StrokeLine {
            x0: sanitize_finite(x0),
            y0: sanitize_finite(y0),
            x1: sanitize_finite(x1),
            y1: sanitize_finite(y1),
            width: sanitize_finite(width),
            color,
        })
    }

    pub fn stroke_polyline(
        &mut self,
        points: &[(f32, f32)],
        width: f32,
        color: ColorRgba,
    ) -> &mut Self {
        let clamped = &points[..points.len().min(MAX_POLYLINE_POINTS)];
        let sanitized: Vec<(f32, f32)> = clamped
            .iter()
            .map(|&(px, py)| (sanitize_finite(px), sanitize_finite(py)))
            .collect();
        self.push_checked(CanvasOp::StrokePolyline {
            points: sanitized,
            width: sanitize_finite(width),
            color,
        })
    }

    #[inline]
    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        self.push_checked(CanvasOp::ClipRect {
            x: sanitize_finite(x),
            y: sanitize_finite(y),
            w: sanitize_finite(w),
            h: sanitize_finite(h),
        })
    }

    #[inline]
    pub fn restore_clip(&mut self) -> &mut Self {
        self.push_checked(CanvasOp::RestoreClip)
    }

    pub fn push_op(&mut self, op: CanvasOp) -> &mut Self {
        match op {
            CanvasOp::Clear { color } => self.clear(color),
            CanvasOp::FillRect { x, y, w, h, color } => self.fill_rect(x, y, w, h, color),
            CanvasOp::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => self.fill_rounded_rect(x, y, w, h, radius, color),
            CanvasOp::FillCircle {
                cx,
                cy,
                radius,
                color,
            } => self.fill_circle(cx, cy, radius, color),
            CanvasOp::StrokeLine {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
            } => self.stroke_line(x0, y0, x1, y1, width, color),
            CanvasOp::StrokePolyline {
                points,
                width,
                color,
            } => self.stroke_polyline(&points, width, color),
            CanvasOp::ClipRect { x, y, w, h } => self.clip_rect(x, y, w, h),
            CanvasOp::RestoreClip => self.restore_clip(),
        }
    }

    pub fn extend_ops(&mut self, ops: impl IntoIterator<Item = CanvasOp>) -> &mut Self {
        for op in ops {
            self.push_op(op);
        }
        self
    }

    pub fn sparkline(
        &mut self,
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> &mut Self {
        let ops = CanvasOp::sparkline(samples, width, height, stroke_color, background_color);
        self.extend_ops(ops)
    }

    /// Consumes this recorder into an immutable, `O(1)`-clonable [`DrawList`].
    #[inline]
    pub fn finish(self) -> DrawList {
        DrawList::from_ops(self.ops)
    }
}
