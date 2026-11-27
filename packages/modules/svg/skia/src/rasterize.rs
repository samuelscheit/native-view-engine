//! Analytical 4-subscanline antialiased polygon filler (`NonZero` & `EvenOdd`), adaptive
//! quadratic/cubic Bezier flattening, and per-shape SDF stroke rasterization for [`SvgPicture`].

use nve_abi::ColorRgba;
use nve_svg::{FillRule, SvgPicture, VectorPathCmd, VectorShape};

use super::{CachedSvgBitmap, MAX_SVG_RASTER_DIM};

const SUBSCANLINES: usize = 4;
const MAX_CURVE_SUBDIV_DEPTH: u8 = 8;
const CURVE_FLATNESS_TOL_SQ: f32 = 0.0625; // (0.25 px)^2

/// Rasterizes `picture` into a fresh premultiplied RGBA8888 [`CachedSvgBitmap`] of `(px_w, px_h)`.
pub fn rasterize_svg_picture(
    picture: &SvgPicture,
    px_w: u32,
    px_h: u32,
    tint: Option<ColorRgba>,
) -> CachedSvgBitmap {
    let w = px_w.clamp(1, MAX_SVG_RASTER_DIM);
    let h = px_h.clamp(1, MAX_SVG_RASTER_DIM);
    let mut premul_rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    let Some((scale, ox, oy)) = picture.view_box_transform(0.0, 0.0, w as f32, h as f32) else {
        return CachedSvgBitmap {
            width: w,
            height: h,
            premul_rgba,
        };
    };

    for shape in picture.shapes.iter() {
        rasterize_shape(shape, scale, ox, oy, w, h, tint, &mut premul_rgba);
    }

    CachedSvgBitmap {
        width: w,
        height: h,
        premul_rgba,
    }
}

#[derive(Clone, Copy)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dir: i32,
}

struct Subpath {
    points: Vec<(f32, f32)>,
    closed: bool,
}

#[allow(clippy::too_many_arguments)]
fn rasterize_shape(
    shape: &VectorShape,
    scale: f32,
    ox: f32,
    oy: f32,
    w: u32,
    h: u32,
    tint: Option<ColorRgba>,
    out: &mut [u8],
) {
    if shape.path.is_empty() {
        return;
    }
    let subpaths = flatten_subpaths(&shape.path, scale, ox, oy);
    let (fill, stroke) = shape.effective_colors(tint);

    if let Some(fill_color) = fill
        && fill_color.a > 0
    {
        rasterize_fill(&subpaths, shape.fill_rule, fill_color, w, h, out);
    }

    if let Some(stroke_color) = stroke
        && stroke_color.a > 0
        && shape.stroke_width.is_finite()
        && shape.stroke_width > 0.0
    {
        let half_w = (shape.stroke_width * scale * 0.5).max(0.5);
        rasterize_stroke(&subpaths, half_w, stroke_color, w, h, out);
    }
}

fn flatten_subpaths(cmds: &[VectorPathCmd], scale: f32, ox: f32, oy: f32) -> Vec<Subpath> {
    let tx = |x: f32| x * scale + ox;
    let ty = |y: f32| y * scale + oy;
    let mut subpaths: Vec<Subpath> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let mut last = (0.0f32, 0.0f32);

    for &cmd in cmds {
        match cmd {
            VectorPathCmd::MoveTo(x, y) => {
                if cur.len() >= 2 {
                    subpaths.push(Subpath {
                        points: std::mem::take(&mut cur),
                        closed: false,
                    });
                } else {
                    cur.clear();
                }
                let pt = (tx(x), ty(y));
                cur.push(pt);
                last = pt;
            }
            VectorPathCmd::LineTo(x, y) => {
                let pt = (tx(x), ty(y));
                if cur.is_empty() {
                    cur.push(last);
                }
                cur.push(pt);
                last = pt;
            }
            VectorPathCmd::QuadTo(cx, cy, x, y) => {
                let p0 = last;
                let p1 = (tx(cx), ty(cy));
                let p2 = (tx(x), ty(y));
                if cur.is_empty() {
                    cur.push(p0);
                }
                flatten_quad(p0, p1, p2, 0, &mut cur);
                last = p2;
            }
            VectorPathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                let p0 = last;
                let p1 = (tx(x1), ty(y1));
                let p2 = (tx(x2), ty(y2));
                let p3 = (tx(x), ty(y));
                if cur.is_empty() {
                    cur.push(p0);
                }
                flatten_cubic(p0, p1, p2, p3, 0, &mut cur);
                last = p3;
            }
            VectorPathCmd::Close => {
                if cur.len() >= 2 {
                    let start = cur[0];
                    if (last.0 - start.0).hypot(last.1 - start.1) > 1e-4 {
                        cur.push(start);
                    }
                    last = start;
                    subpaths.push(Subpath {
                        points: std::mem::take(&mut cur),
                        closed: true,
                    });
                } else {
                    cur.clear();
                }
            }
        }
    }
    if cur.len() >= 2 {
        subpaths.push(Subpath {
            points: cur,
            closed: false,
        });
    }
    subpaths
}

fn flatten_quad(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    depth: u8,
    out: &mut Vec<(f32, f32)>,
) {
    let mx = 0.5 * (p0.0 + p2.0);
    let my = 0.5 * (p0.1 + p2.1);
    let dx = p1.0 - mx;
    let dy = p1.1 - my;
    if depth >= MAX_CURVE_SUBDIV_DEPTH || (dx * dx + dy * dy) <= CURVE_FLATNESS_TOL_SQ {
        out.push(p2);
        return;
    }
    let q0 = (0.5 * (p0.0 + p1.0), 0.5 * (p0.1 + p1.1));
    let q1 = (0.5 * (p1.0 + p2.0), 0.5 * (p1.1 + p2.1));
    let mid = (0.5 * (q0.0 + q1.0), 0.5 * (q0.1 + q1.1));
    flatten_quad(p0, q0, mid, depth + 1, out);
    flatten_quad(mid, q1, p2, depth + 1, out);
}

fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    depth: u8,
    out: &mut Vec<(f32, f32)>,
) {
    let d1x = 3.0 * p1.0 - 2.0 * p0.0 - p3.0;
    let d1y = 3.0 * p1.1 - 2.0 * p0.1 - p3.1;
    let d2x = 3.0 * p2.0 - p0.0 - 2.0 * p3.0;
    let d2y = 3.0 * p2.1 - p0.1 - 2.0 * p3.1;
    let err_sq = (d1x * d1x + d1y * d1y).max(d2x * d2x + d2y * d2y);
    if depth >= MAX_CURVE_SUBDIV_DEPTH || err_sq <= CURVE_FLATNESS_TOL_SQ * 16.0 {
        out.push(p3);
        return;
    }
    let m01 = (0.5 * (p0.0 + p1.0), 0.5 * (p0.1 + p1.1));
    let m12 = (0.5 * (p1.0 + p2.0), 0.5 * (p1.1 + p2.1));
    let m23 = (0.5 * (p2.0 + p3.0), 0.5 * (p2.1 + p3.1));
    let m012 = (0.5 * (m01.0 + m12.0), 0.5 * (m01.1 + m12.1));
    let m123 = (0.5 * (m12.0 + m23.0), 0.5 * (m12.1 + m23.1));
    let mid = (0.5 * (m012.0 + m123.0), 0.5 * (m012.1 + m123.1));
    flatten_cubic(p0, m01, m012, mid, depth + 1, out);
    flatten_cubic(mid, m123, m23, p3, depth + 1, out);
}

fn rasterize_fill(
    subpaths: &[Subpath],
    fill_rule: FillRule,
    color: ColorRgba,
    w: u32,
    h: u32,
    out: &mut [u8],
) {
    let mut edges: Vec<Edge> = Vec::new();
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    let mut push_edge = |a: (f32, f32), b: (f32, f32)| {
        if (a.1 - b.1).abs() <= 1e-6 {
            return;
        }
        let (x0, y0, x1, y1, dir) = if a.1 < b.1 {
            (a.0, a.1, b.0, b.1, 1)
        } else {
            (b.0, b.1, a.0, a.1, -1)
        };
        min_y = min_y.min(y0);
        max_y = max_y.max(y1);
        edges.push(Edge { x0, y0, x1, y1, dir });
    };

    for sp in subpaths {
        if sp.points.len() < 2 {
            continue;
        }
        for pair in sp.points.windows(2) {
            push_edge(pair[0], pair[1]);
        }
        if !sp.closed {
            let first = sp.points[0];
            let last = sp.points[sp.points.len() - 1];
            push_edge(last, first);
        }
    }
    if edges.is_empty() {
        return;
    }

    let y_start = (min_y.floor() as i32).clamp(0, h as i32);
    let y_end = (max_y.ceil() as i32).clamp(0, h as i32);
    let mut row_cov = vec![0.0f32; w as usize];
    let mut hits: Vec<(f32, i32)> = Vec::with_capacity(32);
    let weight = 1.0 / (SUBSCANLINES as f32);

    for py in y_start..y_end {
        row_cov.fill(0.0);
        for s in 0..SUBSCANLINES {
            let sy = py as f32 + (s as f32 + 0.5) * weight;
            hits.clear();
            for e in &edges {
                if sy >= e.y0 && sy < e.y1 {
                    let t = (sy - e.y0) / (e.y1 - e.y0);
                    let x = e.x0 + (e.x1 - e.x0) * t;
                    hits.push((x, e.dir));
                }
            }
            if hits.len() < 2 {
                continue;
            }
            hits.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding = 0i32;
            let mut inside = false;
            let mut span_start = 0.0f32;
            for &(x, dir) in &hits {
                let was_inside = inside;
                winding += dir;
                inside = match fill_rule {
                    FillRule::NonZero => winding != 0,
                    FillRule::EvenOdd => (winding & 1) != 0,
                };
                if !was_inside && inside {
                    span_start = x;
                } else if was_inside && !inside {
                    accumulate_span(&mut row_cov, span_start, x, weight);
                }
            }
        }

        let row_base = (py as usize) * (w as usize);
        for (px, &cov) in row_cov.iter().enumerate() {
            if cov > 0.0 {
                blend_premul_rgba(out, (row_base + px) * 4, color, cov);
            }
        }
    }
}

#[inline]
fn accumulate_span(row_cov: &mut [f32], x0: f32, x1: f32, weight: f32) {
    let w = row_cov.len() as f32;
    let l = x0.clamp(0.0, w);
    let r = x1.clamp(0.0, w);
    if r <= l {
        return;
    }
    let px0 = (l.floor() as usize).min(row_cov.len().saturating_sub(1));
    let px1 = (r.ceil() as usize).min(row_cov.len());
    for (px, slot) in row_cov.iter_mut().enumerate().take(px1).skip(px0) {
        let p_l = px as f32;
        let p_r = p_l + 1.0;
        let overlap = (r.min(p_r) - l.max(p_l)).max(0.0);
        *slot += overlap * weight;
    }
}

fn rasterize_stroke(
    subpaths: &[Subpath],
    half_w: f32,
    color: ColorRgba,
    w: u32,
    h: u32,
    out: &mut [u8],
) {
    let mut cov_buf = vec![0.0f32; (w as usize) * (h as usize)];
    let pad = half_w.ceil() as i32 + 1;
    let wi = w as i32;
    let hi = h as i32;

    for sp in subpaths {
        for pair in sp.points.windows(2) {
            let (ax, ay) = pair[0];
            let (bx, by) = pair[1];
            let min_x = (ax.min(bx).floor() as i32 - pad).clamp(0, wi);
            let max_x = (ax.max(bx).ceil() as i32 + pad).clamp(0, wi);
            let min_y = (ay.min(by).floor() as i32 - pad).clamp(0, hi);
            let max_y = (ay.max(by).ceil() as i32 + pad).clamp(0, hi);
            let abx = bx - ax;
            let aby = by - ay;
            let len_sq = abx * abx + aby * aby;

            for py in min_y..max_y {
                let fy = py as f32 + 0.5;
                let row = (py as usize) * (w as usize);
                for px in min_x..max_x {
                    let fx = px as f32 + 0.5;
                    let dist = if len_sq <= 1e-6 {
                        (fx - ax).hypot(fy - ay)
                    } else {
                        let t = (((fx - ax) * abx + (fy - ay) * aby) / len_sq).clamp(0.0, 1.0);
                        let proj_x = ax + abx * t;
                        let proj_y = ay + aby * t;
                        (fx - proj_x).hypot(fy - proj_y)
                    };
                    let cov = (0.5 - (dist - half_w)).clamp(0.0, 1.0);
                    let idx = row + (px as usize);
                    if cov > cov_buf[idx] {
                        cov_buf[idx] = cov;
                    }
                }
            }
        }
    }

    for (idx, &cov) in cov_buf.iter().enumerate() {
        if cov > 0.0 {
            blend_premul_rgba(out, idx * 4, color, cov);
        }
    }
}

/// Src-over composites straight `color` * `cov` onto a premultiplied RGBA8888 pixel slot.
#[inline]
fn blend_premul_rgba(buf: &mut [u8], idx: usize, color: ColorRgba, cov: f32) {
    let sa = (f32::from(color.a) / 255.0) * cov.clamp(0.0, 1.0);
    if sa <= 1e-5 {
        return;
    }
    let sr_p = f32::from(color.r) * sa;
    let sg_p = f32::from(color.g) * sa;
    let sb_p = f32::from(color.b) * sa;
    let sa_p = 255.0 * sa;
    let inv_sa = 1.0 - sa;

    let dr = f32::from(buf[idx]);
    let dg = f32::from(buf[idx + 1]);
    let db = f32::from(buf[idx + 2]);
    let da = f32::from(buf[idx + 3]);

    buf[idx] = (sr_p + dr * inv_sa).clamp(0.0, 255.0).round() as u8;
    buf[idx + 1] = (sg_p + dg * inv_sa).clamp(0.0, 255.0).round() as u8;
    buf[idx + 2] = (sb_p + db * inv_sa).clamp(0.0, 255.0).round() as u8;
    buf[idx + 3] = (sa_p + da * inv_sa).clamp(0.0, 255.0).round() as u8;
}
