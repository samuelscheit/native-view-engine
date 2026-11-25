//! Zero-dependency typed vector SVG primitives (`SvgPicture`, `VectorShape`, `VectorPathCmd`,
//! `FillRule`), deterministic content hashing for O(1) GPU/raster cache lookup, and XSS-safe
//! inline `<svg>` HTML emission.

mod path_parser;

use std::sync::Arc;

use nve_abi::num::write_f32;
use nve_abi::ColorRgba;

use crate::paint::write_css_color;

use path_parser::MAX_SVG_COORD_ABS;
pub use path_parser::{parse_svg_path_data, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES};

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Polygon fill rule matching SVG `fill-rule` (`nonzero` / `evenodd`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FillRule {
    #[default]
    NonZero = 0,
    EvenOdd = 1,
}

impl FillRule {
    #[inline]
    pub const fn as_svg_attr(self) -> &'static str {
        match self {
            Self::NonZero => "nonzero",
            Self::EvenOdd => "evenodd",
        }
    }
}

/// Typed vector path segment command in `SvgPicture` viewBox coordinate space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VectorPathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// One vector path shape with optional fill, optional stroke, stroke width, and fill rule.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorShape {
    pub path: Arc<[VectorPathCmd]>,
    pub fill: Option<ColorRgba>,
    pub stroke: Option<ColorRgba>,
    pub stroke_width: f32,
    pub fill_rule: FillRule,
}

impl VectorShape {
    /// Creates a filled vector shape with `FillRule::NonZero`.
    pub fn filled(path: impl Into<Vec<VectorPathCmd>>, color: ColorRgba) -> Self {
        Self {
            path: Arc::from(path.into()),
            fill: Some(color),
            stroke: None,
            stroke_width: 0.0,
            fill_rule: FillRule::NonZero,
        }
    }

    /// Creates a stroked vector shape with no fill.
    pub fn stroked(
        path: impl Into<Vec<VectorPathCmd>>,
        color: ColorRgba,
        stroke_width: f32,
    ) -> Self {
        Self {
            path: Arc::from(path.into()),
            fill: None,
            stroke: Some(color),
            stroke_width: if stroke_width.is_finite() {
                stroke_width.max(0.0)
            } else {
                0.0
            },
            fill_rule: FillRule::NonZero,
        }
    }

    /// Sets the [`FillRule`] (`NonZero` or `EvenOdd`).
    #[inline]
    pub fn with_fill_rule(mut self, fill_rule: FillRule) -> Self {
        self.fill_rule = fill_rule;
        self
    }

    /// Sets an optional stroke color and width.
    #[inline]
    pub fn with_stroke(mut self, color: ColorRgba, width: f32) -> Self {
        self.stroke = Some(color);
        self.stroke_width = if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        };
        self
    }

    /// Resolves effective `(fill, stroke)` when an optional icon `tint` is applied.
    #[inline]
    pub fn effective_colors(&self, tint: Option<ColorRgba>) -> (Option<ColorRgba>, Option<ColorRgba>) {
        match tint {
            Some(t) => (self.fill.map(|_| t), self.stroke.map(|_| t)),
            None => (self.fill, self.stroke),
        }
    }
}

/// Immutable, content-hashed typed vector picture ready for zero-serialization native Skia
/// caching and injection-free inline Web `<svg>` emission.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgPicture {
    pub content_hash: u64,
    pub view_box: (f32, f32, f32, f32),
    pub shapes: Arc<[VectorShape]>,
}

impl SvgPicture {
    /// Constructs a content-hashed [`SvgPicture`] from a `viewBox` `(min_x, min_y, width, height)`
    /// and a list of [`VectorShape`]s.
    pub fn new(view_box: (f32, f32, f32, f32), shapes: impl Into<Vec<VectorShape>>) -> Self {
        let vb = sanitize_view_box(view_box);
        let shapes_arc: Arc<[VectorShape]> = Arc::from(shapes.into());
        let content_hash = compute_svg_content_hash(vb, &shapes_arc);
        Self {
            content_hash,
            view_box: vb,
            shapes: shapes_arc,
        }
    }

    /// Compact helper parsing one or more SVG `d="..."` path strings into filled shapes
    /// within `viewBox="0 0 width height"`.
    pub fn from_paths(width: f32, height: f32, paths: &[&str]) -> Self {
        let shapes: Vec<VectorShape> = paths
            .iter()
            .filter_map(|d| {
                let cmds = parse_svg_path_data(d);
                (!cmds.is_empty()).then(|| VectorShape::filled(cmds, ColorRgba::BLACK))
            })
            .collect();
        Self::new((0.0, 0.0, width, height), shapes)
    }

    /// Compact helper parsing one or more SVG `d="..."` path strings into stroked shapes
    /// within `viewBox="0 0 width height"`.
    pub fn from_stroked_paths(
        width: f32,
        height: f32,
        stroke_width: f32,
        paths: &[&str],
    ) -> Self {
        let shapes: Vec<VectorShape> = paths
            .iter()
            .filter_map(|d| {
                let cmds = parse_svg_path_data(d);
                (!cmds.is_empty()).then(|| VectorShape::stroked(cmds, ColorRgba::BLACK, stroke_width))
            })
            .collect();
        Self::new((0.0, 0.0, width, height), shapes)
    }

    /// Returns `true` if two `SvgPicture` handles share the same underlying `Arc` allocation.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.content_hash == other.content_hash
            && self.view_box == other.view_box
            && Arc::ptr_eq(&self.shapes, &other.shapes)
    }

    /// Writes canonical `viewBox` attribute value (`"vx vy vw vh"`) into `out`.
    pub fn write_view_box_into(&self, out: &mut String) {
        let (vx, vy, vw, vh) = sanitize_view_box(self.view_box);
        write_f32(out, vx);
        out.push(' ');
        write_f32(out, vy);
        out.push(' ');
        write_f32(out, vw);
        out.push(' ');
        write_f32(out, vh);
    }

    /// Writes `<path .../>` child elements for all shapes directly into `out` (zero XSS because
    /// every coordinate and color is formatted from strictly typed numeric values).
    pub fn write_svg_children_into(&self, tint: Option<ColorRgba>, out: &mut String) {
        for shape in self.shapes.iter() {
            if shape.path.is_empty() {
                continue;
            }
            let (fill, stroke) = shape.effective_colors(tint);
            out.push_str("<path d=\"");
            write_path_data_into(&shape.path, out);
            out.push('"');
            match fill {
                Some(c) => {
                    out.push_str(" fill=\"");
                    write_css_color(c, out);
                    out.push('"');
                }
                None => out.push_str(" fill=\"none\""),
            }
            if shape.fill_rule == FillRule::EvenOdd {
                out.push_str(" fill-rule=\"evenodd\"");
            }
            if let Some(sc) = stroke
                && shape.stroke_width.is_finite()
                && shape.stroke_width > 0.0
            {
                out.push_str(" stroke=\"");
                write_css_color(sc, out);
                out.push_str("\" stroke-width=\"");
                write_f32(out, shape.stroke_width);
                out.push_str("\" stroke-linecap=\"round\" stroke-linejoin=\"round\"");
            }
            out.push_str("/>");
        }
    }

    /// Emits a self-contained, XSS-safe inline `<svg viewBox="...">...</svg>` HTML fragment.
    pub fn to_inline_svg_html(&self, width: f32, height: f32, tint: Option<ColorRgba>) -> String {
        let mut out = String::with_capacity(128 + self.shapes.len() * 64);
        out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"");
        self.write_view_box_into(&mut out);
        out.push('"');
        if width.is_finite() && width > 0.0 {
            out.push_str(" width=\"");
            write_f32(&mut out, width);
            out.push('"');
        }
        if height.is_finite() && height > 0.0 {
            out.push_str(" height=\"");
            write_f32(&mut out, height);
            out.push('"');
        }
        out.push('>');
        self.write_svg_children_into(tint, &mut out);
        out.push_str("</svg>");
        out
    }

    /// Computes uniform `xMidYMid meet` transform `(scale, offset_x, offset_y)` mapping `viewBox`
    /// coordinates into a target rectangle `(dst_x, dst_y, dst_w, dst_h)`.
    #[inline]
    pub fn view_box_transform(
        &self,
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
    ) -> Option<(f32, f32, f32)> {
        let (vx, vy, vw, vh) = sanitize_view_box(self.view_box);
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

/// Formats typed [`VectorPathCmd`]s as canonical SVG `d="..."` string (injection-free numeric only).
pub fn write_path_data_into(cmds: &[VectorPathCmd], out: &mut String) {
    for (idx, cmd) in cmds.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        match *cmd {
            VectorPathCmd::MoveTo(x, y) => {
                out.push('M');
                write_f32(out, sanitize_coord(x));
                out.push(' ');
                write_f32(out, sanitize_coord(y));
            }
            VectorPathCmd::LineTo(x, y) => {
                out.push('L');
                write_f32(out, sanitize_coord(x));
                out.push(' ');
                write_f32(out, sanitize_coord(y));
            }
            VectorPathCmd::QuadTo(cx, cy, x, y) => {
                out.push('Q');
                write_f32(out, sanitize_coord(cx));
                out.push(' ');
                write_f32(out, sanitize_coord(cy));
                out.push(' ');
                write_f32(out, sanitize_coord(x));
                out.push(' ');
                write_f32(out, sanitize_coord(y));
            }
            VectorPathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                out.push('C');
                write_f32(out, sanitize_coord(c1x));
                out.push(' ');
                write_f32(out, sanitize_coord(c1y));
                out.push(' ');
                write_f32(out, sanitize_coord(c2x));
                out.push(' ');
                write_f32(out, sanitize_coord(c2y));
                out.push(' ');
                write_f32(out, sanitize_coord(x));
                out.push(' ');
                write_f32(out, sanitize_coord(y));
            }
            VectorPathCmd::Close => out.push('Z'),
        }
    }
}

#[inline]
fn sanitize_coord(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-MAX_SVG_COORD_ABS, MAX_SVG_COORD_ABS)
    } else {
        0.0
    }
}

#[inline]
fn sanitize_view_box(vb: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let vx = if vb.0.is_finite() { vb.0 } else { 0.0 };
    let vy = if vb.1.is_finite() { vb.1 } else { 0.0 };
    let vw = if vb.2.is_finite() && vb.2 > 0.0 { vb.2 } else { 24.0 };
    let vh = if vb.3.is_finite() && vb.3 > 0.0 { vb.3 } else { 24.0 };
    (vx, vy, vw, vh)
}

#[inline]
fn fnv_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV1A_PRIME);
    }
    h
}

#[inline]
fn fnv_f32(h: u64, v: f32) -> u64 {
    let bits = if !v.is_finite() || v == 0.0 {
        0u32
    } else {
        v.to_bits()
    };
    fnv_bytes(h, &bits.to_le_bytes())
}

#[inline]
fn fnv_opt_color(h: u64, c: Option<ColorRgba>) -> u64 {
    match c {
        Some(col) => fnv_bytes(h, &[1, col.r, col.g, col.b, col.a]),
        None => fnv_bytes(h, &[0]),
    }
}

/// Computes a deterministic 64-bit FNV-1a content hash over `view_box` and `shapes`.
pub fn compute_svg_content_hash(view_box: (f32, f32, f32, f32), shapes: &[VectorShape]) -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv_f32(h, view_box.0);
    h = fnv_f32(h, view_box.1);
    h = fnv_f32(h, view_box.2);
    h = fnv_f32(h, view_box.3);
    h = fnv_bytes(h, &(shapes.len() as u32).to_le_bytes());
    for s in shapes {
        h = fnv_opt_color(h, s.fill);
        h = fnv_opt_color(h, s.stroke);
        h = fnv_f32(h, s.stroke_width);
        h = fnv_bytes(h, &[s.fill_rule as u8]);
        h = fnv_bytes(h, &(s.path.len() as u32).to_le_bytes());
        for &cmd in s.path.iter() {
            match cmd {
                VectorPathCmd::MoveTo(x, y) => {
                    h = fnv_f32(fnv_f32(fnv_bytes(h, &[1]), x), y);
                }
                VectorPathCmd::LineTo(x, y) => {
                    h = fnv_f32(fnv_f32(fnv_bytes(h, &[2]), x), y);
                }
                VectorPathCmd::QuadTo(cx, cy, x, y) => {
                    h = fnv_f32(fnv_f32(fnv_f32(fnv_f32(fnv_bytes(h, &[3]), cx), cy), x), y);
                }
                VectorPathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                    let h0 = fnv_f32(fnv_f32(fnv_bytes(h, &[4]), x1), y1);
                    let h1 = fnv_f32(fnv_f32(h0, x2), y2);
                    h = fnv_f32(fnv_f32(h1, x), y);
                }
                VectorPathCmd::Close => {
                    h = fnv_bytes(h, &[5]);
                }
            }
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_svg_html_is_deterministic_and_injection_free() {
        let pic = SvgPicture::from_paths(24.0, 24.0, &["M2 2L22 2L12 22Z"]);
        assert_ne!(pic.content_hash, 0);
        let html = pic.to_inline_svg_html(24.0, 24.0, Some(ColorRgba::rgb(56, 189, 248)));
        assert!(html.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" width=\"24\" height=\"24\">"
        ));
        assert!(html.contains("d=\"M2 2 L22 2 L12 22 Z\""));
        assert!(html.contains("fill=\"#38bdf8\""));
        assert!(html.ends_with("</svg>"));
    }
}
