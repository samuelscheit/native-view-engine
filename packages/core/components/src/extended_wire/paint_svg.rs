//! Binary wire codecs for [`BackgroundFill`] and [`SvgPicture`] (cross-boundary `BatchSink` only).

use std::sync::Arc;

use crate::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use crate::svg::{FillRule, SvgPicture, VectorPathCmd, VectorShape};

use super::{read_color, write_color, WireReader};

pub(crate) fn write_background_fill(out: &mut Vec<u8>, fill: &BackgroundFill) {
    match fill {
        BackgroundFill::Solid(c) => {
            out.push(0);
            write_color(out, *c);
        }
        BackgroundFill::Linear(lg) => {
            out.push(1);
            out.extend_from_slice(&lg.angle_deg.to_le_bytes());
            out.extend_from_slice(&(lg.stops.len() as u32).to_le_bytes());
            for stop in lg.stops.iter() {
                out.extend_from_slice(&stop.offset.to_le_bytes());
                write_color(out, stop.color);
            }
        }
        BackgroundFill::Radial(rg) => {
            out.push(2);
            out.extend_from_slice(&rg.center_x.to_le_bytes());
            out.extend_from_slice(&rg.center_y.to_le_bytes());
            out.extend_from_slice(&rg.radius.to_le_bytes());
            out.extend_from_slice(&(rg.stops.len() as u32).to_le_bytes());
            for stop in rg.stops.iter() {
                out.extend_from_slice(&stop.offset.to_le_bytes());
                write_color(out, stop.color);
            }
        }
    }
}

pub(crate) fn read_background_fill(r: &mut WireReader<'_>) -> Option<BackgroundFill> {
    match r.read_u8()? {
        0 => Some(BackgroundFill::Solid(read_color(r)?)),
        1 => {
            let angle_deg = r.read_f32()?;
            let n = r.read_count()?;
            let mut stops = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                stops.push(GradientStop::new(r.read_f32()?, read_color(r)?));
            }
            Some(BackgroundFill::Linear(LinearGradient::new(angle_deg, stops)))
        }
        2 => {
            let cx = r.read_f32()?;
            let cy = r.read_f32()?;
            let rad = r.read_f32()?;
            let n = r.read_count()?;
            let mut stops = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                stops.push(GradientStop::new(r.read_f32()?, read_color(r)?));
            }
            Some(BackgroundFill::Radial(RadialGradient::new(cx, cy, rad, stops)))
        }
        _ => None,
    }
}

/// Encodes [`BackgroundFill`] into `out`.
pub fn encode_background_fill(fill: &BackgroundFill, out: &mut Vec<u8>) {
    write_background_fill(out, fill);
}

/// Decodes [`BackgroundFill`] from wire bytes.
pub fn decode_background_fill(bytes: &[u8]) -> Option<BackgroundFill> {
    let mut r = WireReader::new(bytes);
    let fill = read_background_fill(&mut r)?;
    r.is_empty().then_some(fill)
}

/// Encodes [`SvgPicture`] into `out`.
pub fn encode_svg_picture(pic: &SvgPicture, out: &mut Vec<u8>) {
    out.extend_from_slice(&pic.view_box.0.to_le_bytes());
    out.extend_from_slice(&pic.view_box.1.to_le_bytes());
    out.extend_from_slice(&pic.view_box.2.to_le_bytes());
    out.extend_from_slice(&pic.view_box.3.to_le_bytes());
    out.extend_from_slice(&(pic.shapes.len() as u32).to_le_bytes());
    for shape in pic.shapes.iter() {
        match shape.fill {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                write_color(out, c);
            }
        }
        match shape.stroke {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                write_color(out, c);
            }
        }
        out.extend_from_slice(&shape.stroke_width.to_le_bytes());
        out.push(shape.fill_rule as u8);
        out.extend_from_slice(&(shape.path.len() as u32).to_le_bytes());
        for &cmd in shape.path.iter() {
            match cmd {
                VectorPathCmd::MoveTo(x, y) => {
                    out.push(0);
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                }
                VectorPathCmd::LineTo(x, y) => {
                    out.push(1);
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                }
                VectorPathCmd::QuadTo(cx, cy, x, y) => {
                    out.push(2);
                    out.extend_from_slice(&cx.to_le_bytes());
                    out.extend_from_slice(&cy.to_le_bytes());
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                }
                VectorPathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                    out.push(3);
                    out.extend_from_slice(&c1x.to_le_bytes());
                    out.extend_from_slice(&c1y.to_le_bytes());
                    out.extend_from_slice(&c2x.to_le_bytes());
                    out.extend_from_slice(&c2y.to_le_bytes());
                    out.extend_from_slice(&x.to_le_bytes());
                    out.extend_from_slice(&y.to_le_bytes());
                }
                VectorPathCmd::Close => out.push(4),
            }
        }
    }
}

/// Decodes [`SvgPicture`] from wire bytes.
pub fn decode_svg_picture(bytes: &[u8]) -> Option<SvgPicture> {
    let mut r = WireReader::new(bytes);
    let view_box = (r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?);
    let shape_count = r.read_count()?;
    let mut shapes = Vec::with_capacity(shape_count.min(64));
    for _ in 0..shape_count {
        let fill = match r.read_u8()? {
            0 => None,
            1 => Some(read_color(&mut r)?),
            _ => return None,
        };
        let stroke = match r.read_u8()? {
            0 => None,
            1 => Some(read_color(&mut r)?),
            _ => return None,
        };
        let stroke_width = r.read_f32()?;
        let fill_rule = match r.read_u8()? {
            0 => FillRule::NonZero,
            1 => FillRule::EvenOdd,
            _ => return None,
        };
        let cmd_count = r.read_count()?;
        let mut path = Vec::with_capacity(cmd_count.min(256));
        for _ in 0..cmd_count {
            let cmd = match r.read_u8()? {
                0 => VectorPathCmd::MoveTo(r.read_f32()?, r.read_f32()?),
                1 => VectorPathCmd::LineTo(r.read_f32()?, r.read_f32()?),
                2 => VectorPathCmd::QuadTo(
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                ),
                3 => VectorPathCmd::CubicTo(
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                    r.read_f32()?,
                ),
                4 => VectorPathCmd::Close,
                _ => return None,
            };
            path.push(cmd);
        }
        shapes.push(VectorShape {
            path: Arc::from(path),
            fill,
            stroke,
            stroke_width,
            fill_rule,
        });
    }
    r.is_empty().then(|| SvgPicture::new(view_box, shapes))
}
