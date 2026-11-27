//! Flat 4-byte-aligned [`FlatResource`] implementation for [`SvgPicture`] and zero-copy
//! [`SvgPictureView`] / [`VectorShapeView`] slab readers.

use nve_abi::num::write_f32;
use nve_abi::{ColorRgba, Key64};
use nve_store::FlatResource;

use crate::picture::{
    sanitize_view_box, write_css_color, write_path_cmd_into, FillRule, SvgPicture, VectorPathCmd,
};
use crate::{MAX_SVG_CMDS, MAX_SVG_SHAPES};

const FLAG_HAS_FILL: u32 = 1 << 0;
const FLAG_HAS_STROKE: u32 = 1 << 1;
const FLAG_EVEN_ODD: u32 = 1 << 2;
const ALL_SHAPE_FLAGS: u32 = FLAG_HAS_FILL | FLAG_HAS_STROKE | FLAG_EVEN_ODD;

const CMD_MOVE_TO: u32 = 0;
const CMD_LINE_TO: u32 = 1;
const CMD_QUAD_TO: u32 = 2;
const CMD_CUBIC_TO: u32 = 3;
const CMD_CLOSE: u32 = 4;

/// Zero-copy borrowed view over flat [`SvgPicture`] words:
/// `[hash_lo, hash_hi, vx, vy, vw, vh, shape_count, (flags, fill_rgba, stroke_rgba, stroke_w, cmd_count, (cmd_tag, coords...)*)*]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgPictureView<'a>(&'a [u32]);

impl<'a> SvgPictureView<'a> {
    #[inline(always)]
    pub fn content_hash(self) -> u64 {
        let lo = u64::from(self.0.first().copied().unwrap_or(0));
        let hi = u64::from(self.0.get(1).copied().unwrap_or(0));
        lo | (hi << 32)
    }

    #[inline(always)]
    pub fn view_box(self) -> (f32, f32, f32, f32) {
        if self.0.len() < 6 {
            return (0.0, 0.0, 24.0, 24.0);
        }
        (
            f32::from_bits(self.0[2]),
            f32::from_bits(self.0[3]),
            f32::from_bits(self.0[4]),
            f32::from_bits(self.0[5]),
        )
    }

    #[inline(always)]
    pub const fn shape_count(self) -> usize {
        if self.0.len() < 7 { 0 } else { self.0[6] as usize }
    }

    #[inline]
    pub fn shapes(self) -> VectorShapesIter<'a> {
        VectorShapesIter {
            words: if self.0.len() > 7 { &self.0[7..] } else { &[] },
            pos: 0,
            remaining: self.shape_count(),
        }
    }

    /// Writes canonical `viewBox` attribute value (`"vx vy vw vh"`) into `out`.
    pub fn write_view_box_into(self, out: &mut String) {
        let (vx, vy, vw, vh) = sanitize_view_box(self.view_box());
        write_f32(out, vx);
        out.push(' ');
        write_f32(out, vy);
        out.push(' ');
        write_f32(out, vw);
        out.push(' ');
        write_f32(out, vh);
    }

    /// Writes `<path .../>` child elements directly from the flat slab without allocating.
    pub fn write_svg_children_into(self, tint: Option<ColorRgba>, out: &mut String) {
        for shape in self.shapes() {
            if shape.cmd_count() == 0 {
                continue;
            }
            let (fill, stroke) = shape.effective_colors(tint);
            out.push_str("<path d=\"");
            for (idx, cmd) in shape.cmds().enumerate() {
                if idx > 0 {
                    out.push(' ');
                }
                write_path_cmd_into(cmd, out);
            }
            out.push('"');
            match fill {
                Some(c) => {
                    out.push_str(" fill=\"");
                    write_css_color(c, out);
                    out.push('"');
                }
                None => out.push_str(" fill=\"none\""),
            }
            if shape.fill_rule() == FillRule::EvenOdd {
                out.push_str(" fill-rule=\"evenodd\"");
            }
            if let Some(sc) = stroke
                && shape.stroke_width().is_finite()
                && shape.stroke_width() > 0.0
            {
                out.push_str(" stroke=\"");
                write_css_color(sc, out);
                out.push_str("\" stroke-width=\"");
                write_f32(out, shape.stroke_width());
                out.push_str("\" stroke-linecap=\"round\" stroke-linejoin=\"round\"");
            }
            out.push_str("/>");
        }
    }

    /// Computes uniform `xMidYMid meet` transform `(scale, offset_x, offset_y)`.
    #[inline]
    pub fn view_box_transform(
        self,
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
    ) -> Option<(f32, f32, f32)> {
        let (vx, vy, vw, vh) = sanitize_view_box(self.view_box());
        if !dst_w.is_finite() || !dst_h.is_finite() || dst_w <= 0.0 || dst_h <= 0.0 {
            return None;
        }
        if vw <= 1e-6 || vh <= 1e-6 {
            return None;
        }
        let scale = (dst_w / vw).min(dst_h / vh);
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let ox = dst_x + (dst_w - vw * scale) * 0.5 - vx * scale;
        let oy = dst_y + (dst_h - vh * scale) * 0.5 - vy * scale;
        Some((scale, ox, oy))
    }
}

/// Zero-copy view of one [`crate::VectorShape`] inside an [`SvgPictureView`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VectorShapeView<'a> {
    fill: Option<ColorRgba>,
    stroke: Option<ColorRgba>,
    stroke_width: f32,
    fill_rule: FillRule,
    cmd_count: usize,
    cmd_words: &'a [u32],
}

impl<'a> VectorShapeView<'a> {
    #[inline(always)]
    pub const fn fill(self) -> Option<ColorRgba> {
        self.fill
    }

    #[inline(always)]
    pub const fn stroke(self) -> Option<ColorRgba> {
        self.stroke
    }

    #[inline(always)]
    pub const fn stroke_width(self) -> f32 {
        self.stroke_width
    }

    #[inline(always)]
    pub const fn fill_rule(self) -> FillRule {
        self.fill_rule
    }

    #[inline(always)]
    pub const fn cmd_count(self) -> usize {
        self.cmd_count
    }

    #[inline(always)]
    pub const fn effective_colors(
        self,
        tint: Option<ColorRgba>,
    ) -> (Option<ColorRgba>, Option<ColorRgba>) {
        match tint {
            Some(t) => {
                let f = if self.fill.is_some() { Some(t) } else { None };
                let s = if self.stroke.is_some() { Some(t) } else { None };
                (f, s)
            }
            None => (self.fill, self.stroke),
        }
    }

    #[inline]
    pub fn cmds(self) -> VectorPathCmdsIter<'a> {
        VectorPathCmdsIter {
            words: self.cmd_words,
            pos: 0,
            remaining: self.cmd_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VectorShapesIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl<'a> Iterator for VectorShapesIter<'a> {
    type Item = VectorShapeView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let hdr = self.words.get(self.pos..self.pos + 5)?;
        let flags = hdr[0];
        let fill = ((flags & FLAG_HAS_FILL) != 0).then(|| ColorRgba::from_u32(hdr[1]));
        let stroke = ((flags & FLAG_HAS_STROKE) != 0).then(|| ColorRgba::from_u32(hdr[2]));
        let stroke_width = f32::from_bits(hdr[3]);
        let fill_rule = if (flags & FLAG_EVEN_ODD) != 0 {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        };
        let cmd_count = hdr[4] as usize;
        let cmds_start = self.pos + 5;
        let mut cursor = cmds_start;
        for _ in 0..cmd_count {
            let tag = *self.words.get(cursor)?;
            cursor = cursor.checked_add(cmd_word_len(tag)?)?;
        }
        let cmd_words = self.words.get(cmds_start..cursor)?;
        self.pos = cursor;
        self.remaining -= 1;
        Some(VectorShapeView {
            fill,
            stroke,
            stroke_width,
            fill_rule,
            cmd_count,
            cmd_words,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VectorPathCmdsIter<'a> {
    words: &'a [u32],
    pos: usize,
    remaining: usize,
}

impl Iterator for VectorPathCmdsIter<'_> {
    type Item = VectorPathCmd;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let tag = *self.words.get(self.pos)?;
        let p = self.pos + 1;
        let cmd = match tag {
            CMD_MOVE_TO => {
                let s = self.words.get(p..p + 2)?;
                self.pos = p + 2;
                VectorPathCmd::MoveTo(f32::from_bits(s[0]), f32::from_bits(s[1]))
            }
            CMD_LINE_TO => {
                let s = self.words.get(p..p + 2)?;
                self.pos = p + 2;
                VectorPathCmd::LineTo(f32::from_bits(s[0]), f32::from_bits(s[1]))
            }
            CMD_QUAD_TO => {
                let s = self.words.get(p..p + 4)?;
                self.pos = p + 4;
                VectorPathCmd::QuadTo(
                    f32::from_bits(s[0]),
                    f32::from_bits(s[1]),
                    f32::from_bits(s[2]),
                    f32::from_bits(s[3]),
                )
            }
            CMD_CUBIC_TO => {
                let s = self.words.get(p..p + 6)?;
                self.pos = p + 6;
                VectorPathCmd::CubicTo(
                    f32::from_bits(s[0]),
                    f32::from_bits(s[1]),
                    f32::from_bits(s[2]),
                    f32::from_bits(s[3]),
                    f32::from_bits(s[4]),
                    f32::from_bits(s[5]),
                )
            }
            CMD_CLOSE => {
                self.pos = p;
                VectorPathCmd::Close
            }
            _ => return None,
        };
        self.remaining -= 1;
        Some(cmd)
    }
}

#[inline(always)]
const fn cmd_word_len(tag: u32) -> Option<usize> {
    match tag {
        CMD_MOVE_TO | CMD_LINE_TO => Some(3),
        CMD_QUAD_TO => Some(5),
        CMD_CUBIC_TO => Some(7),
        CMD_CLOSE => Some(1),
        _ => None,
    }
}

impl FlatResource for SvgPicture {
    const KIND: Key64 = Key64::of("nve.res.svg_picture");
    type View<'a> = SvgPictureView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.shapes.len() > MAX_SVG_SHAPES {
            return false;
        }
        let start = out.len();
        out.extend_from_slice(&[
            self.content_hash as u32,
            (self.content_hash >> 32) as u32,
            self.view_box.0.to_bits(),
            self.view_box.1.to_bits(),
            self.view_box.2.to_bits(),
            self.view_box.3.to_bits(),
            self.shapes.len() as u32,
        ]);
        for shape in self.shapes.iter() {
            if shape.path.len() > MAX_SVG_CMDS {
                out.truncate(start);
                return false;
            }
            let mut flags = 0u32;
            let fill_u32 = match shape.fill {
                Some(c) => {
                    flags |= FLAG_HAS_FILL;
                    c.to_u32()
                }
                None => 0,
            };
            let stroke_u32 = match shape.stroke {
                Some(c) => {
                    flags |= FLAG_HAS_STROKE;
                    c.to_u32()
                }
                None => 0,
            };
            if shape.fill_rule == FillRule::EvenOdd {
                flags |= FLAG_EVEN_ODD;
            }
            out.extend_from_slice(&[
                flags,
                fill_u32,
                stroke_u32,
                shape.stroke_width.to_bits(),
                shape.path.len() as u32,
            ]);
            for &cmd in shape.path.iter() {
                match cmd {
                    VectorPathCmd::MoveTo(x, y) => {
                        out.extend_from_slice(&[CMD_MOVE_TO, x.to_bits(), y.to_bits()])
                    }
                    VectorPathCmd::LineTo(x, y) => {
                        out.extend_from_slice(&[CMD_LINE_TO, x.to_bits(), y.to_bits()])
                    }
                    VectorPathCmd::QuadTo(cx, cy, x, y) => out.extend_from_slice(&[
                        CMD_QUAD_TO,
                        cx.to_bits(),
                        cy.to_bits(),
                        x.to_bits(),
                        y.to_bits(),
                    ]),
                    VectorPathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => out.extend_from_slice(&[
                        CMD_CUBIC_TO,
                        c1x.to_bits(),
                        c1y.to_bits(),
                        c2x.to_bits(),
                        c2y.to_bits(),
                        x.to_bits(),
                        y.to_bits(),
                    ]),
                    VectorPathCmd::Close => out.push(CMD_CLOSE),
                }
            }
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        if words.len() < 7 {
            return false;
        }
        let shape_count = words[6] as usize;
        if shape_count > MAX_SVG_SHAPES {
            return false;
        }
        let mut pos = 7usize;
        for _ in 0..shape_count {
            let Some(hdr) = words.get(pos..pos + 5) else {
                return false;
            };
            let flags = hdr[0];
            if (flags & !ALL_SHAPE_FLAGS) != 0 {
                return false;
            }
            if (flags & FLAG_HAS_FILL) == 0 && hdr[1] != 0 {
                return false;
            }
            if (flags & FLAG_HAS_STROKE) == 0 && hdr[2] != 0 {
                return false;
            }
            let cmd_count = hdr[4] as usize;
            if cmd_count > MAX_SVG_CMDS {
                return false;
            }
            pos += 5;
            for _ in 0..cmd_count {
                let Some(&tag) = words.get(pos) else {
                    return false;
                };
                let Some(step) = cmd_word_len(tag) else {
                    return false;
                };
                let Some(next) = pos.checked_add(step) else {
                    return false;
                };
                if next > words.len() {
                    return false;
                }
                pos = next;
            }
        }
        pos == words.len()
    }

    #[inline(always)]
    fn view(words: &[u32]) -> SvgPictureView<'_> {
        SvgPictureView(words)
    }
}
