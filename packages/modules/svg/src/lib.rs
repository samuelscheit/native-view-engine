//! Standalone vector SVG capability (`packages/modules/svg`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Svg`] (`#[derive(NveTable)]`),
//! [`SvgPicture`] (`nve_store::Resource`), path parser, and deterministic inline SVG formatter.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod path_parser;
mod picture;

pub use picture::{
    compute_svg_content_hash, parse_svg_path_data, write_path_data_into, FillRule, SvgPicture,
    VectorPathCmd, VectorShape, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES,
};

use std::sync::Arc;

pub use nve_abi::ColorRgba;
use nve_abi::Key64;
use nve_codec::{ByteReader, ByteWriter, CodecError, NveDecode, NveEncode};
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

fn encode_svg_picture(pic: &SvgPicture, w: &mut ByteWriter) -> Result<(), CodecError> {
    w.write_f32(pic.view_box.0);
    w.write_f32(pic.view_box.1);
    w.write_f32(pic.view_box.2);
    w.write_f32(pic.view_box.3);
    w.write_len(pic.shapes.len())?;
    for shape in pic.shapes.iter() {
        w.write_opt(shape.fill, |w, c| {
            w.write_u32(c.to_u32());
            Ok(())
        })?;
        w.write_opt(shape.stroke, |w, c| {
            w.write_u32(c.to_u32());
            Ok(())
        })?;
        w.write_f32(shape.stroke_width);
        shape.fill_rule.encode(w)?;
        w.write_len(shape.path.len())?;
        for cmd in shape.path.iter() {
            cmd.encode(w)?;
        }
    }
    Ok(())
}

fn decode_svg_picture(bytes: &[u8]) -> Result<SvgPicture, CodecError> {
    let mut r = ByteReader::new(bytes);
    let vb = (r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?);
    let shapes = r.read_vec(MAX_SHAPES, 11, |r| {
        let fill = r.read_opt(|r| Ok(ColorRgba::from_u32(r.read_u32()?)))?;
        let stroke = r.read_opt(|r| Ok(ColorRgba::from_u32(r.read_u32()?)))?;
        let stroke_width = r.read_f32()?;
        let fill_rule = FillRule::decode(r)?;
        let path = r.read_vec(MAX_CMDS, VectorPathCmd::MIN_ENCODED_BYTES, VectorPathCmd::decode)?;
        Ok(VectorShape {
            path: Arc::from(path),
            fill,
            stroke,
            stroke_width,
            fill_rule,
        })
    })?;
    r.finish()?;
    Ok(SvgPicture::new(vb, shapes))
}

impl Resource for SvgPicture {
    const KIND: Key64 = Key64::of("nve.res.svg_picture");

    fn encode(&self, out: &mut Vec<u8>) {
        let res = ByteWriter::with_buf(out, |w| encode_svg_picture(self, w));
        debug_assert!(res.is_ok());
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        decode_svg_picture(bytes).ok()
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
