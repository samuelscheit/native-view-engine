//! Single-pass flat-word immutable vector draw list (`DrawList`, backed directly by `Arc<[u32]>`)
//! and fluent [`DrawListRecorder`] backed by [`GuardedPainter`].
//!
//! `DrawListSink` writes canonical 4-byte-aligned `nve.res.draw_list` words (`[op_count,
//! (opcode, args...)*]`) directly with zero intermediate `Vec<CanvasOp>` or per-polyline heap
//! allocations, so [`nve_store::FlatResource::write_words`] is a single `memcpy` and
//! [`DrawList::record_into_words`] writes straight into `ResourceStore`'s reused scratch buffer.

use std::sync::{Arc, OnceLock};

use nve_abi::ColorRgba;
use nve_store::FlatResource;

use crate::desc::{
    OP_CLEAR, OP_CLIP_RECT, OP_FILL_CIRCLE, OP_FILL_RECT, OP_FILL_ROUNDED_RECT, OP_RESTORE_CLIP,
    OP_STROKE_LINE, OP_STROKE_POLYLINE,
};
use crate::flat::{encode_ops_into, CanvasOpRef, DrawListOpsIter, DrawListView};
use crate::painter::{CanvasPainter, CanvasSink, GuardedPainter};
use crate::polyline::{PolylineBacking, PolylinePoints};
use crate::types::{paint_sparkline, CanvasOp, MAX_DRAW_OPS};

/// Average flat `u32` words per draw op used to pre-size `DrawListRecorder` word buffers.
const WORDS_PER_OP_ESTIMATE: usize = 6;

/// Shared zero-allocation backing slice for [`DrawList::empty`].
static EMPTY_DRAW_LIST_WORDS: OnceLock<Arc<[u32]>> = OnceLock::new();

/// Immutable, `O(1)`-clonable flat 4-byte-aligned vector draw list (`Arc<[u32]>`).
///
/// Every non-empty `words` slice is sealed-valid canonical `nve.res.draw_list` wire format
/// (`[op_count, (opcode, args...)*]`); an empty 0-word slice represents an invalid raw
/// [`Self::from_ops`] payload rejected by [`FlatResource::write_words`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawList {
    words: Arc<[u32]>,
}

impl Default for DrawList {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl From<Vec<CanvasOp>> for DrawList {
    #[inline]
    fn from(ops: Vec<CanvasOp>) -> Self {
        Self::from_ops(ops)
    }
}

impl DrawList {
    #[inline]
    pub fn empty() -> Self {
        Self {
            words: Arc::clone(EMPTY_DRAW_LIST_WORDS.get_or_init(|| Arc::from([0u32]))),
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

    /// Records a [`DrawList`] directly into flat `u32` words with canonical sanitization and
    /// automatic clip unwinding (zero intermediate `Vec<CanvasOp>`).
    #[inline]
    pub fn record(f: impl FnOnce(&mut DrawListRecorder)) -> Self {
        let mut rec = DrawListRecorder::new();
        f(&mut rec);
        rec.finish()
    }

    /// Like [`Self::record`] with a pre-sized flat word buffer.
    #[inline]
    pub fn record_with_capacity(op_capacity: usize, f: impl FnOnce(&mut DrawListRecorder)) -> Self {
        let mut rec = DrawListRecorder::with_capacity(op_capacity);
        f(&mut rec);
        rec.finish()
    }

    /// Records flat `nve.res.draw_list` `u32` words directly into `out`, reusing `out`'s existing
    /// capacity with zero `Arc` or intermediate `DrawList` allocation.
    ///
    /// Designed for `tx.edit::<Canvas>(node).draw_list_ephemeral_with(|out| { ...; true })`.
    #[inline]
    pub fn record_into_words(out: &mut Vec<u32>, f: impl FnOnce(&mut DrawListRecorder)) {
        struct RestoreGuard<'a> {
            out: &'a mut Vec<u32>,
            base: usize,
            rec: Option<DrawListRecorder>,
        }
        impl Drop for RestoreGuard<'_> {
            fn drop(&mut self) {
                if let Some(mut rec) = self.rec.take() {
                    rec.inner.unwind_clips();
                    let mut words = std::mem::take(&mut rec.inner.sink_mut().words);
                    words.truncate(self.base);
                    *self.out = words;
                }
            }
        }

        let base = out.len();
        let buf = std::mem::take(out);
        let mut guard = RestoreGuard {
            out,
            base,
            rec: Some(DrawListRecorder::from_word_buf(buf, base)),
        };
        if let Some(rec) = guard.rec.as_mut() {
            f(rec);
        }
        if let Some(rec) = guard.rec.take() {
            *guard.out = rec.finish_words_vec();
        }
    }

    /// Encodes a raw vector of typed [`CanvasOp`]s directly into flat `u32` words.
    ///
    /// Invalid raw ops (e.g. non-finite floats or negative radius/width) produce an empty 0-word
    /// sentinel that [`FlatResource::write_words`] rejects cleanly without modifying its output.
    pub fn from_ops(ops: Vec<CanvasOp>) -> Self {
        let mut words = Vec::with_capacity(1 + ops.len().min(MAX_DRAW_OPS) * WORDS_PER_OP_ESTIMATE);
        if encode_ops_into(&ops, &mut words) {
            Self {
                words: Arc::from(words),
            }
        } else {
            Self {
                words: Arc::from([] as [u32; 0]),
            }
        }
    }

    /// Validates a flat `nve.res.draw_list` word slice and copies it into an immutable [`DrawList`].
    pub fn try_from_words(words: &[u32]) -> Option<Self> {
        <Self as FlatResource>::validate(words).then(|| Self::from_validated_words(words))
    }

    #[inline]
    pub(crate) fn from_validated_words(words: &[u32]) -> Self {
        Self {
            words: Arc::from(words),
        }
    }

    /// Underlying flat `u32` word slice (`[op_count, (opcode, args...)*]`).
    #[inline]
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// Byte length of the encoded flat binary IR stream (`words().len() * 4`).
    #[inline]
    pub fn byte_len(&self) -> usize {
        self.words.len() * 4
    }

    /// Zero-copy borrowed [`DrawListView`] over this draw list's flat `u32` words.
    #[inline]
    pub fn as_view(&self) -> DrawListView<'_> {
        DrawListView::from_validated_slice(&self.words)
    }

    /// Zero-allocation iterator yielding borrowed [`CanvasOpRef`] operations.
    #[inline]
    pub fn ops(&self) -> DrawListOpsIter<'_> {
        self.as_view().ops()
    }

    /// Materializes owned [`CanvasOp`]s (for diagnostics and tests).
    pub fn to_ops(&self) -> Vec<CanvasOp> {
        self.ops().map(CanvasOpRef::to_owned).collect()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.as_view().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn op_count(&self) -> u32 {
        self.words.first().copied().unwrap_or(0)
    }

    /// `O(1)` pointer identity check for fast change-detection between frames.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.words, &other.words)
    }

    /// Replays every operation in this draw list into `painter` with zero allocations.
    #[inline]
    pub fn replay(&self, painter: &mut (impl CanvasPainter + ?Sized)) {
        self.as_view().replay(painter);
    }

    /// Builds a telemetry sparkline directly into a flat-word [`DrawList`].
    pub fn sparkline(
        samples: &[f32],
        width: f32,
        height: f32,
        stroke_color: ColorRgba,
        background_color: ColorRgba,
    ) -> Self {
        Self::record_with_capacity(12, |rec| {
            paint_sparkline(rec, samples, width, height, stroke_color, background_color);
        })
    }
}

#[derive(Debug, Clone)]
struct DrawListSink {
    words: Vec<u32>,
    base: usize,
    op_count: usize,
    open_clips: usize,
    skipped_clips: usize,
}

impl Default for DrawListSink {
    #[inline]
    fn default() -> Self {
        Self::with_capacity(16)
    }
}

impl DrawListSink {
    #[inline]
    fn with_capacity(op_capacity: usize) -> Self {
        let cap = 1 + op_capacity
            .min(MAX_DRAW_OPS)
            .saturating_mul(WORDS_PER_OP_ESTIMATE);
        let mut words = Vec::with_capacity(cap);
        words.push(0); // reserve slot 0 for `op_count`
        Self {
            words,
            base: 0,
            op_count: 0,
            open_clips: 0,
            skipped_clips: 0,
        }
    }

    #[inline]
    fn from_word_buf(mut words: Vec<u32>, base: usize) -> Self {
        words.push(0); // reserve slot `base` for `op_count`
        Self {
            words,
            base,
            op_count: 0,
            open_clips: 0,
            skipped_clips: 0,
        }
    }

    #[inline(always)]
    fn can_push_draw(&self) -> bool {
        // Never let draw ops escape a `MAX_DRAW_OPS`-skipped clip into the parent clip, and reserve
        // one op slot per currently-open clip so unwind `RestoreClip`s never exceed `MAX_DRAW_OPS`.
        self.skipped_clips == 0 && self.op_count + self.open_clips < MAX_DRAW_OPS
    }
}

impl CanvasSink for DrawListSink {
    #[inline]
    fn clear(&mut self, color: ColorRgba) {
        if self.can_push_draw() {
            self.words.extend_from_slice(&[OP_CLEAR, color.to_u32()]);
            self.op_count += 1;
        }
    }

    #[inline]
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) {
        if self.can_push_draw() {
            self.words.extend_from_slice(&[
                OP_FILL_RECT,
                x.to_bits(),
                y.to_bits(),
                w.to_bits(),
                h.to_bits(),
                color.to_u32(),
            ]);
            self.op_count += 1;
        }
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
        if self.can_push_draw() {
            self.words.extend_from_slice(&[
                OP_FILL_ROUNDED_RECT,
                x.to_bits(),
                y.to_bits(),
                w.to_bits(),
                h.to_bits(),
                radius.to_bits(),
                color.to_u32(),
            ]);
            self.op_count += 1;
        }
    }

    #[inline]
    fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) {
        if self.can_push_draw() {
            self.words.extend_from_slice(&[
                OP_FILL_CIRCLE,
                cx.to_bits(),
                cy.to_bits(),
                radius.to_bits(),
                color.to_u32(),
            ]);
            self.op_count += 1;
        }
    }

    #[inline]
    fn stroke_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: ColorRgba) {
        if self.can_push_draw() {
            self.words.extend_from_slice(&[
                OP_STROKE_LINE,
                x0.to_bits(),
                y0.to_bits(),
                x1.to_bits(),
                y1.to_bits(),
                width.to_bits(),
                color.to_u32(),
            ]);
            self.op_count += 1;
        }
    }

    #[inline]
    fn stroke_polyline(&mut self, points: PolylinePoints<'_>, width: f32, color: ColorRgba) {
        if !self.can_push_draw() {
            return;
        }
        let pt_count = points.len();
        self.words.reserve(4 + pt_count * 2);
        self.words.extend_from_slice(&[
            OP_STROKE_POLYLINE,
            width.to_bits(),
            color.to_u32(),
            pt_count as u32,
        ]);
        match points.backing() {
            PolylineBacking::Words(w) => {
                self.words.extend_from_slice(&w[..(w.len() & !1)]);
            }
            PolylineBacking::Slice(s) => {
                for &(px, py) in s {
                    self.words.push(px.to_bits());
                    self.words.push(py.to_bits());
                }
            }
        }
        self.op_count += 1;
    }

    #[inline]
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.skipped_clips == 0 && self.op_count + self.open_clips + 2 <= MAX_DRAW_OPS {
            self.words.extend_from_slice(&[
                OP_CLIP_RECT,
                x.to_bits(),
                y.to_bits(),
                w.to_bits(),
                h.to_bits(),
            ]);
            self.op_count += 1;
            self.open_clips += 1;
        } else {
            self.skipped_clips += 1;
        }
    }

    #[inline]
    fn pop_clip(&mut self) {
        if self.skipped_clips > 0 {
            self.skipped_clips -= 1;
        } else if self.open_clips > 0 {
            self.open_clips -= 1;
            self.words.push(OP_RESTORE_CLIP);
            self.op_count += 1;
        }
    }
}

/// Fluent single-pass flat-word recorder constructing a [`DrawList`] (`Arc<[u32]>`) through
/// [`GuardedPainter`].
///
/// Implements [`CanvasPainter`] (object-safe `-> ()`) and retains inherent fluent `-> &mut Self`
/// convenience methods forwarding to the canonical guarded contract.
#[derive(Debug, Clone, Default)]
pub struct DrawListRecorder {
    inner: GuardedPainter<DrawListSink>,
}

impl DrawListRecorder {
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    #[inline]
    pub fn with_capacity(op_capacity: usize) -> Self {
        Self {
            inner: GuardedPainter::new(DrawListSink::with_capacity(op_capacity)),
        }
    }

    #[inline]
    fn from_word_buf(buf: Vec<u32>, base: usize) -> Self {
        Self {
            inner: GuardedPainter::new(DrawListSink::from_word_buf(buf, base)),
        }
    }

    #[inline]
    pub fn op_count(&self) -> u32 {
        self.inner.sink().op_count.min(u32::MAX as usize) as u32
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.sink().op_count == 0
    }

    #[inline]
    pub fn clear(&mut self, color: ColorRgba) -> &mut Self {
        CanvasPainter::clear(self, color);
        self
    }

    #[inline]
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: ColorRgba) -> &mut Self {
        CanvasPainter::fill_rect(self, x, y, w, h, color);
        self
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
        CanvasPainter::fill_rounded_rect(self, x, y, w, h, radius, color);
        self
    }

    #[inline]
    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: ColorRgba) -> &mut Self {
        CanvasPainter::fill_circle(self, cx, cy, radius, color);
        self
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
        CanvasPainter::stroke_line(self, x0, y0, x1, y1, width, color);
        self
    }

    #[inline]
    pub fn stroke_polyline(
        &mut self,
        points: &[(f32, f32)],
        width: f32,
        color: ColorRgba,
    ) -> &mut Self {
        CanvasPainter::stroke_polyline(self, points, width, color);
        self
    }

    #[inline]
    pub fn clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) -> &mut Self {
        CanvasPainter::clip_rect(self, x, y, w, h);
        self
    }

    #[inline]
    pub fn restore_clip(&mut self) -> &mut Self {
        CanvasPainter::restore_clip(self);
        self
    }

    #[inline]
    pub fn push_op(&mut self, op: CanvasOp) -> &mut Self {
        op.replay(self);
        self
    }

    pub fn extend_ops(&mut self, ops: impl IntoIterator<Item = CanvasOp>) -> &mut Self {
        for op in ops {
            op.replay(self);
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
        paint_sparkline(self, samples, width, height, stroke_color, background_color);
        self
    }

    #[inline]
    fn finish_words_vec(mut self) -> Vec<u32> {
        self.inner.unwind_clips();
        let sink = self.inner.sink_mut();
        let base = sink.base;
        let op_count = sink.op_count as u32;
        let mut words = std::mem::take(&mut sink.words);
        if let Some(slot) = words.get_mut(base) {
            *slot = op_count;
        } else {
            words.push(0);
        }
        words
    }

    /// Unwinds any remaining open clips and returns owned [`CanvasOp`]s.
    pub fn into_ops(self) -> Vec<CanvasOp> {
        self.finish().to_ops()
    }

    /// Unwinds any remaining open clips, patches `words[0] = op_count`, and consumes this
    /// recorder into an immutable flat-word [`DrawList`].
    #[inline]
    pub fn finish(self) -> DrawList {
        DrawList {
            words: Arc::from(self.finish_words_vec()),
        }
    }
}

impl CanvasPainter for DrawListRecorder {
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
