//! Standalone vector SVG capability (`packages/modules/svg`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Svg`] (`#[derive(NveTable)]`),
//! [`SvgPicture`] (`nve_store::Resource`), path parser, and deterministic inline SVG formatter.

pub mod path_parser;
mod picture;

pub use picture::{
    compute_svg_content_hash, parse_svg_path_data, write_path_data_into, FillRule, SvgPicture,
    VectorPathCmd, VectorShape, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES,
};

use std::sync::Arc;

pub use nve_abi::ColorRgba;
use nve_abi::Key64;
use nve_store::{NveTable, Res, Resource, Rgba};

/// Vector SVG node table (`nve.svg`).
#[derive(NveTable)]
#[nve_table(name = "nve.svg", storage = "sparse", flags = "publish")]
pub struct Svg {
    #[effect(realize, paint)]
    pub picture: Res<SvgPicture>,
    /// Override fill/stroke tint (`Rgba::TRANSPARENT` = inherit typography color / picture colors).
    #[effect(paint)]
    pub tint: Rgba,
    /// Mirror horizontally under RTL layouts (`[dir=rtl]`).
    #[effect(paint)]
    #[bits(1)]
    pub mirror_directional: bool,
}

const MAX_SHAPES: usize = 4096;
const MAX_CMDS: usize = 65536;

impl Resource for SvgPicture {
    const KIND: Key64 = Key64::of("nve.res.svg_picture");

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.view_box.0.to_le_bytes());
        out.extend_from_slice(&self.view_box.1.to_le_bytes());
        out.extend_from_slice(&self.view_box.2.to_le_bytes());
        out.extend_from_slice(&self.view_box.3.to_le_bytes());
        out.extend_from_slice(&(self.shapes.len() as u32).to_le_bytes());
        for shape in self.shapes.iter() {
            match shape.fill {
                None => out.push(0),
                Some(c) => {
                    out.push(1);
                    out.extend_from_slice(&c.to_u32().to_le_bytes());
                }
            }
            match shape.stroke {
                None => out.push(0),
                Some(c) => {
                    out.push(1);
                    out.extend_from_slice(&c.to_u32().to_le_bytes());
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

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut pos = 0usize;
        let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
            let end = pos.checked_add(n)?;
            let s = bytes.get(*pos..end)?;
            *pos = end;
            Some(s)
        };
        let u8v = |pos: &mut usize| -> Option<u8> { Some(take(pos, 1)?[0]) };
        let u32v = |pos: &mut usize| -> Option<u32> {
            Some(u32::from_le_bytes(take(pos, 4)?.try_into().ok()?))
        };
        let f32v = |pos: &mut usize| -> Option<f32> {
            Some(f32::from_le_bytes(take(pos, 4)?.try_into().ok()?))
        };

        let vb = (
            f32v(&mut pos)?,
            f32v(&mut pos)?,
            f32v(&mut pos)?,
            f32v(&mut pos)?,
        );
        let shape_count = u32v(&mut pos)? as usize;
        if shape_count > MAX_SHAPES {
            return None;
        }
        let mut shapes = Vec::with_capacity(shape_count.min(64));
        for _ in 0..shape_count {
            let fill = match u8v(&mut pos)? {
                0 => None,
                1 => Some(ColorRgba::from_u32(u32v(&mut pos)?)),
                _ => return None,
            };
            let stroke = match u8v(&mut pos)? {
                0 => None,
                1 => Some(ColorRgba::from_u32(u32v(&mut pos)?)),
                _ => return None,
            };
            let stroke_width = f32v(&mut pos)?;
            let fill_rule = match u8v(&mut pos)? {
                0 => FillRule::NonZero,
                1 => FillRule::EvenOdd,
                _ => return None,
            };
            let cmd_count = u32v(&mut pos)? as usize;
            if cmd_count > MAX_CMDS {
                return None;
            }
            let mut path = Vec::with_capacity(cmd_count.min(256));
            for _ in 0..cmd_count {
                let cmd = match u8v(&mut pos)? {
                    0 => VectorPathCmd::MoveTo(f32v(&mut pos)?, f32v(&mut pos)?),
                    1 => VectorPathCmd::LineTo(f32v(&mut pos)?, f32v(&mut pos)?),
                    2 => VectorPathCmd::QuadTo(
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                    ),
                    3 => VectorPathCmd::CubicTo(
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
                        f32v(&mut pos)?,
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
        (pos == bytes.len()).then(|| SvgPicture::new(vb, shapes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    #[test]
    fn svg_table_and_picture_resource_roundtrip() {
        let pic = SvgPicture::from_paths(24.0, 24.0, &["M0 0 L24 24 Z"]);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Svg>(n)
            .picture(Some(pic.clone()))
            .tint(Rgba::rgb(10, 20, 30))
            .mirror_directional(true);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<Svg>(n).unwrap();
        assert_eq!(row.tint(), Rgba::rgb(10, 20, 30));
        assert!(row.mirror_directional());
        let got = store.resource::<SvgPicture>(row.picture()).unwrap();
        assert_eq!(got.content_hash, pic.content_hash);

        // Wire round-trip via Resource encode/decode
        let mut buf = Vec::new();
        pic.encode(&mut buf);
        assert_eq!(SvgPicture::decode(&buf).unwrap().content_hash, pic.content_hash);
        assert!(SvgPicture::decode(&[]).is_none());
    }
}
