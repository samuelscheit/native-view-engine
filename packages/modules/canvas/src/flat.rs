//! Flat 4-byte-aligned [`FlatResource`] implementation for [`DrawList`] and zero-copy
//! [`DrawListView`] / [`CanvasOpRef`] / [`PolylinePoints`] slab readers.

use nve_abi::{ColorRgba, Key64};
use nve_codec::FloatConstraint;
use nve_store::{FlatResource, ValidatedWords};

use crate::desc::{
    DRAW_LIST_CODEC_DESC, OP_CLEAR, OP_CLIP_RECT, OP_FILL_CIRCLE, OP_FILL_RECT,
    OP_FILL_ROUNDED_RECT, OP_RESTORE_CLIP, OP_STROKE_LINE, OP_STROKE_POLYLINE,
};
use crate::{CanvasOp, DrawList, MAX_DRAW_OPS, MAX_POLYLINE_POINTS};

/// Zero-copy borrowed view over flat `(x, y)` `f32` vertex pairs inside a `StrokePolyline` op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolylinePoints<'a>(&'a [u32]);

impl<'a> PolylinePoints<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        self.0.len() / 2
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<(f32, f32)> {
        let b = index.checked_mul(2)?;
        let x = *self.0.get(b)?;
        let y = *self.0.get(b + 1)?;
        Some((f32::from_bits(x), f32::from_bits(y)))
    }

    #[inline]
    pub fn iter(self) -> PolylinePointsIter<'a> {
        PolylinePointsIter {
            view: self,
            idx: 0,
            end: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PolylinePointsIter<'a> {
    view: PolylinePoints<'a>,
    idx: usize,
    end: usize,
}

impl Iterator for PolylinePointsIter<'_> {
    type Item = (f32, f32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        let pt = self.view.get(self.idx);
        self.idx += 1;
        pt
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.end - self.idx;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for PolylinePointsIter<'_> {}

/// Zero-copy borrowed drawing operation yielded by [`DrawListView::ops`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CanvasOpRef<'a> {
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
        points: PolylinePoints<'a>,
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

impl CanvasOpRef<'_> {
    pub fn to_owned(self) -> CanvasOp {
        match self {
            Self::Clear { color } => CanvasOp::Clear { color },
            Self::FillRect { x, y, w, h, color } => CanvasOp::FillRect { x, y, w, h, color },
            Self::FillRoundedRect { x, y, w, h, radius, color } => {
                CanvasOp::FillRoundedRect { x, y, w, h, radius, color }
            }
            Self::FillCircle { cx, cy, radius, color } => {
                CanvasOp::FillCircle { cx, cy, radius, color }
            }
            Self::StrokeLine { x0, y0, x1, y1, width, color } => {
                CanvasOp::StrokeLine { x0, y0, x1, y1, width, color }
            }
            Self::StrokePolyline { points, width, color } => CanvasOp::StrokePolyline {
                points: points.iter().collect(),
                width,
                color,
            },
            Self::ClipRect { x, y, w, h } => CanvasOp::ClipRect { x, y, w, h },
            Self::RestoreClip => CanvasOp::RestoreClip,
        }
    }
}

/// Zero-copy borrowed view over flat [`DrawList`] words (`[op_count, (opcode, args...)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawListView<'a>(&'a [u32]);

impl<'a> DrawListView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() { 0 } else { self.0[0] as usize }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn ops(self) -> DrawListOpsIter<'a> {
        DrawListOpsIter {
            words: if self.0.len() > 1 { &self.0[1..] } else { &[] },
            pos: 0,
            remaining: self.len(),
        }
    }

    pub fn to_owned(self) -> DrawList {
        DrawList::from_ops(self.ops().map(CanvasOpRef::to_owned).collect())
    }
}

#[derive(Debug, Clone)]
pub struct DrawListOpsIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for DrawListOpsIter<'a> {
    type Item = CanvasOpRef<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let op = *self.words.get(self.pos)?;
        let p = self.pos + 1;
        let out = match op {
            OP_CLEAR => {
                let c = *self.words.get(p)?;
                self.pos = p + 1;
                CanvasOpRef::Clear {
                    color: ColorRgba::from_u32(c),
                }
            }
            OP_FILL_RECT => {
                let s = self.words.get(p..p + 5)?;
                self.pos = p + 5;
                CanvasOpRef::FillRect {
                    x: f32::from_bits(s[0]),
                    y: f32::from_bits(s[1]),
                    w: f32::from_bits(s[2]),
                    h: f32::from_bits(s[3]),
                    color: ColorRgba::from_u32(s[4]),
                }
            }
            OP_FILL_ROUNDED_RECT => {
                let s = self.words.get(p..p + 6)?;
                self.pos = p + 6;
                CanvasOpRef::FillRoundedRect {
                    x: f32::from_bits(s[0]),
                    y: f32::from_bits(s[1]),
                    w: f32::from_bits(s[2]),
                    h: f32::from_bits(s[3]),
                    radius: f32::from_bits(s[4]),
                    color: ColorRgba::from_u32(s[5]),
                }
            }
            OP_FILL_CIRCLE => {
                let s = self.words.get(p..p + 4)?;
                self.pos = p + 4;
                CanvasOpRef::FillCircle {
                    cx: f32::from_bits(s[0]),
                    cy: f32::from_bits(s[1]),
                    radius: f32::from_bits(s[2]),
                    color: ColorRgba::from_u32(s[3]),
                }
            }
            OP_STROKE_LINE => {
                let s = self.words.get(p..p + 6)?;
                self.pos = p + 6;
                CanvasOpRef::StrokeLine {
                    x0: f32::from_bits(s[0]),
                    y0: f32::from_bits(s[1]),
                    x1: f32::from_bits(s[2]),
                    y1: f32::from_bits(s[3]),
                    width: f32::from_bits(s[4]),
                    color: ColorRgba::from_u32(s[5]),
                }
            }
            OP_STROKE_POLYLINE => {
                let hdr = self.words.get(p..p + 3)?;
                let pt_count = hdr[2] as usize;
                let pts_start = p + 3;
                let pts_end = pts_start.checked_add(pt_count.checked_mul(2)?)?;
                let pts = self.words.get(pts_start..pts_end)?;
                self.pos = pts_end;
                CanvasOpRef::StrokePolyline {
                    points: PolylinePoints(pts),
                    width: f32::from_bits(hdr[0]),
                    color: ColorRgba::from_u32(hdr[1]),
                }
            }
            OP_CLIP_RECT => {
                let s = self.words.get(p..p + 4)?;
                self.pos = p + 4;
                CanvasOpRef::ClipRect {
                    x: f32::from_bits(s[0]),
                    y: f32::from_bits(s[1]),
                    w: f32::from_bits(s[2]),
                    h: f32::from_bits(s[3]),
                }
            }
            OP_RESTORE_CLIP => {
                self.pos = p;
                CanvasOpRef::RestoreClip
            }
            _ => return None,
        };
        self.remaining -= 1;
        Some(out)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for DrawListOpsIter<'_> {}

#[inline]
fn word_finite(word: u32) -> bool {
    FloatConstraint::Finite.check(f32::from_bits(word))
}

#[inline]
fn word_non_negative(word: u32) -> bool {
    FloatConstraint::NonNegative.check(f32::from_bits(word))
}

impl FlatResource for DrawList {
    const KIND: Key64 = Key64::of(DRAW_LIST_CODEC_DESC.kind_name);
    type View<'a> = DrawListView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.ops().len() > MAX_DRAW_OPS {
            return false;
        }
        let start = out.len();
        out.push(self.ops().len() as u32);
        for op in self.ops() {
            match op {
                CanvasOp::Clear { color } => out.extend_from_slice(&[OP_CLEAR, color.to_u32()]),
                CanvasOp::FillRect { x, y, w, h, color } => {
                    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
                        out.truncate(start);
                        return false;
                    }
                    out.extend_from_slice(&[
                        OP_FILL_RECT,
                        x.to_bits(),
                        y.to_bits(),
                        w.to_bits(),
                        h.to_bits(),
                        color.to_u32(),
                    ]);
                }
                CanvasOp::FillRoundedRect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => {
                    if !x.is_finite()
                        || !y.is_finite()
                        || !w.is_finite()
                        || !h.is_finite()
                        || !FloatConstraint::NonNegative.check(*radius)
                    {
                        out.truncate(start);
                        return false;
                    }
                    out.extend_from_slice(&[
                        OP_FILL_ROUNDED_RECT,
                        x.to_bits(),
                        y.to_bits(),
                        w.to_bits(),
                        h.to_bits(),
                        radius.to_bits(),
                        color.to_u32(),
                    ]);
                }
                CanvasOp::FillCircle {
                    cx,
                    cy,
                    radius,
                    color,
                } => {
                    if !cx.is_finite()
                        || !cy.is_finite()
                        || !FloatConstraint::NonNegative.check(*radius)
                    {
                        out.truncate(start);
                        return false;
                    }
                    out.extend_from_slice(&[
                        OP_FILL_CIRCLE,
                        cx.to_bits(),
                        cy.to_bits(),
                        radius.to_bits(),
                        color.to_u32(),
                    ]);
                }
                CanvasOp::StrokeLine {
                    x0,
                    y0,
                    x1,
                    y1,
                    width,
                    color,
                } => {
                    if !x0.is_finite()
                        || !y0.is_finite()
                        || !x1.is_finite()
                        || !y1.is_finite()
                        || !FloatConstraint::NonNegative.check(*width)
                    {
                        out.truncate(start);
                        return false;
                    }
                    out.extend_from_slice(&[
                        OP_STROKE_LINE,
                        x0.to_bits(),
                        y0.to_bits(),
                        x1.to_bits(),
                        y1.to_bits(),
                        width.to_bits(),
                        color.to_u32(),
                    ]);
                }
                CanvasOp::StrokePolyline {
                    points,
                    width,
                    color,
                } => {
                    if points.len() > MAX_POLYLINE_POINTS
                        || !FloatConstraint::NonNegative.check(*width)
                    {
                        out.truncate(start);
                        return false;
                    }
                    out.reserve(4 + points.len() * 2);
                    out.push(OP_STROKE_POLYLINE);
                    out.push(width.to_bits());
                    out.push(color.to_u32());
                    out.push(points.len() as u32);
                    for &(px, py) in points {
                        if !px.is_finite() || !py.is_finite() {
                            out.truncate(start);
                            return false;
                        }
                        out.push(px.to_bits());
                        out.push(py.to_bits());
                    }
                }
                CanvasOp::ClipRect { x, y, w, h } => {
                    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
                        out.truncate(start);
                        return false;
                    }
                    out.extend_from_slice(&[
                        OP_CLIP_RECT,
                        x.to_bits(),
                        y.to_bits(),
                        w.to_bits(),
                        h.to_bits(),
                    ]);
                }
                CanvasOp::RestoreClip => out.push(OP_RESTORE_CLIP),
            }
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count_u32) = words.first() else {
            return false;
        };
        let count = count_u32 as usize;
        if count > MAX_DRAW_OPS || words.len() - 1 < count {
            return false;
        }
        let mut pos = 1usize;
        for _ in 0..count {
            let Some(&op) = words.get(pos) else {
                return false;
            };
            pos += 1;
            match op {
                OP_CLEAR => {
                    if words.get(pos).is_none() {
                        return false;
                    }
                    pos += 1;
                }
                OP_FILL_RECT => {
                    let Some(s) = words.get(pos..pos + 5) else {
                        return false;
                    };
                    if !word_finite(s[0])
                        || !word_finite(s[1])
                        || !word_finite(s[2])
                        || !word_finite(s[3])
                    {
                        return false;
                    }
                    pos += 5;
                }
                OP_FILL_ROUNDED_RECT => {
                    let Some(s) = words.get(pos..pos + 6) else {
                        return false;
                    };
                    if !word_finite(s[0])
                        || !word_finite(s[1])
                        || !word_finite(s[2])
                        || !word_finite(s[3])
                        || !word_non_negative(s[4])
                    {
                        return false;
                    }
                    pos += 6;
                }
                OP_FILL_CIRCLE => {
                    let Some(s) = words.get(pos..pos + 4) else {
                        return false;
                    };
                    if !word_finite(s[0]) || !word_finite(s[1]) || !word_non_negative(s[2]) {
                        return false;
                    }
                    pos += 4;
                }
                OP_STROKE_LINE => {
                    let Some(s) = words.get(pos..pos + 6) else {
                        return false;
                    };
                    if !word_finite(s[0])
                        || !word_finite(s[1])
                        || !word_finite(s[2])
                        || !word_finite(s[3])
                        || !word_non_negative(s[4])
                    {
                        return false;
                    }
                    pos += 6;
                }
                OP_STROKE_POLYLINE => {
                    let Some(hdr) = words.get(pos..pos + 3) else {
                        return false;
                    };
                    if !word_non_negative(hdr[0]) {
                        return false;
                    }
                    let pt_count = hdr[2] as usize;
                    if pt_count > MAX_POLYLINE_POINTS {
                        return false;
                    }
                    let Some(coord_words) = pt_count.checked_mul(2) else {
                        return false;
                    };
                    let pts_start = pos + 3;
                    let Some(pts_end) = pts_start.checked_add(coord_words) else {
                        return false;
                    };
                    let Some(pts) = words.get(pts_start..pts_end) else {
                        return false;
                    };
                    if pts.iter().any(|&w| !word_finite(w)) {
                        return false;
                    }
                    pos = pts_end;
                }
                OP_CLIP_RECT => {
                    let Some(s) = words.get(pos..pos + 4) else {
                        return false;
                    };
                    if !word_finite(s[0])
                        || !word_finite(s[1])
                        || !word_finite(s[2])
                        || !word_finite(s[3])
                    {
                        return false;
                    }
                    pos += 4;
                }
                OP_RESTORE_CLIP => {}
                _ => return false,
            }
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: ValidatedWords<'_, Self>) -> DrawListView<'_> {
        DrawListView(words.as_slice())
    }
}
