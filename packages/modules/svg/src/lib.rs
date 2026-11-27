//! Standalone vector SVG capability (`packages/modules/svg`).
//!
//! Pure-spec root crate with zero backend dependencies: defines [`Svg`] (`#[derive(NveTable)]`),
//! [`SvgPicture`] (`nve_store::FlatResource`), path parser, and deterministic inline SVG formatter.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

mod flat;
pub mod path_parser;
mod picture;

pub use flat::{
    SvgPictureView, VectorPathCmdsIter, VectorShapeView, VectorShapesIter,
};
pub use picture::{
    compute_svg_content_hash, parse_svg_path_data, write_path_data_into, FillRule, SvgPicture,
    VectorPathCmd, VectorShape, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES,
};

use std::sync::Arc;

pub use nve_abi::ColorRgba;
use nve_codec::{ByteReader, ByteWriter, CodecError, NveDecode, NveEncode};
use nve_store::{NveTable, Res, Rgba};

/// Vector SVG node table (`nve.svg`).
#[derive(NveTable)]
#[nve_table(name = "nve.svg", storage = "sparse", flags = "publish")]
pub struct Svg {
    #[effect(realize, paint)]
    pub picture: Res<SvgPicture>,
    /// Optional override fill/stroke tint (`None` = inherit typography color / picture colors).
    #[effect(paint)]
    pub tint: Option<Rgba>,
    /// Mirror horizontally under RTL layouts (`[dir=rtl]`).
    #[effect(paint)]
    pub mirror_directional: bool,
}

pub const MAX_SVG_SHAPES: usize = 4096;
pub const MAX_SVG_CMDS: usize = 65536;

/// Fallibly encodes an [`SvgPicture`] enforcing the exact decoder limits (`MAX_SVG_SHAPES` and
/// `MAX_SVG_CMDS`) at encode time.
pub fn encode_svg_picture(pic: &SvgPicture, w: &mut ByteWriter) -> Result<(), CodecError> {
    w.write_f32(pic.view_box.0);
    w.write_f32(pic.view_box.1);
    w.write_f32(pic.view_box.2);
    w.write_f32(pic.view_box.3);
    w.write_len_max(pic.shapes.len(), MAX_SVG_SHAPES)?;
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
        w.write_len_max(shape.path.len(), MAX_SVG_CMDS)?;
        for cmd in shape.path.iter() {
            cmd.encode(w)?;
        }
    }
    Ok(())
}

/// Decodes an [`SvgPicture`] with full bounds, tag, and trailing-byte validation.
pub fn decode_svg_picture(bytes: &[u8]) -> Result<SvgPicture, CodecError> {
    let mut r = ByteReader::new(bytes);
    let vb = (r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?);
    let shapes = r.read_vec(MAX_SVG_SHAPES, 11, |r| {
        let fill = r.read_opt(|r| Ok(ColorRgba::from_u32(r.read_u32()?)))?;
        let stroke = r.read_opt(|r| Ok(ColorRgba::from_u32(r.read_u32()?)))?;
        let stroke_width = r.read_f32()?;
        let fill_rule = FillRule::decode(r)?;
        let path = r.read_vec(
            MAX_SVG_CMDS,
            VectorPathCmd::MIN_ENCODED_BYTES,
            VectorPathCmd::decode,
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore};

    #[test]
    fn svg_table_and_picture_resource_roundtrip() {
        let pic = SvgPicture::from_paths(24.0, 24.0, &["M0 0 L24 24 Z"]);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Svg>(n)
            .picture(Some(pic.clone()))
            .tint(Some(Rgba::rgb(10, 20, 30)))
            .mirror_directional(true);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let row = store.try_row::<Svg>(n).unwrap();
        assert_eq!(row.tint(), Some(Rgba::rgb(10, 20, 30)));
        assert!(row.mirror_directional());
        let got = store.resource::<SvgPicture>(row.picture()).unwrap();
        assert_eq!(got.content_hash(), pic.content_hash);
        assert_eq!(got.view_box(), pic.view_box);
        assert_eq!(got.shape_count(), 1);

        let mut words = Vec::new();
        assert!(pic.write_words(&mut words));
        assert!(SvgPicture::validate(&words));
        assert_eq!(SvgPicture::view(&words).content_hash(), pic.content_hash);
        assert!(!SvgPicture::validate(&[]));
        assert!(!SvgPicture::validate(&[0, 0, 0, 0, 0, 0, 99999]));
    }

    #[test]
    fn svg_codec_roundtrips_all_variants_and_enforces_caps_and_adversarial_guards() {
        let rich = SvgPicture::new(
            (1.0, 2.0, 64.0, 48.0),
            vec![
                VectorShape {
                    path: Arc::from([
                        VectorPathCmd::MoveTo(0.0, 0.0),
                        VectorPathCmd::LineTo(12.5, 4.0),
                        VectorPathCmd::QuadTo(16.0, 8.0, 24.0, 16.0),
                        VectorPathCmd::CubicTo(28.0, 20.0, 32.0, 12.0, 48.0, 36.0),
                        VectorPathCmd::Close,
                    ]),
                    fill: Some(ColorRgba::rgba(10, 20, 30, 200)),
                    stroke: Some(ColorRgba::rgb(255, 128, 0)),
                    stroke_width: 2.5,
                    fill_rule: FillRule::EvenOdd,
                },
                VectorShape {
                    path: Arc::from([VectorPathCmd::MoveTo(4.0, 4.0), VectorPathCmd::Close]),
                    fill: None,
                    stroke: None,
                    stroke_width: 0.0,
                    fill_rule: FillRule::NonZero,
                },
            ],
        );

        let mut w = ByteWriter::new();
        encode_svg_picture(&rich, &mut w).unwrap();
        let bytes = w.into_bytes();
        let decoded = decode_svg_picture(&bytes).unwrap();
        assert_eq!(decoded, rich);

        // Per-byte truncation across entire buffer must fail cleanly.
        for cut in 0..bytes.len() {
            assert!(
                matches!(decode_svg_picture(&bytes[..cut]), Err(CodecError::Truncated { .. })),
                "truncation at {cut} must return Truncated"
            );
        }

        // Trailing bytes must be rejected.
        let mut trailing = bytes.clone();
        trailing.push(0xAA);
        assert_eq!(decode_svg_picture(&trailing), Err(CodecError::TrailingBytes(1)));

        // Bad fill_rule tag (offset: 16 viewBox + 4 shape_count + 5 fill + 5 stroke + 4 stroke_width = 34).
        let mut bad_rule = bytes.clone();
        bad_rule[34] = 9;
        assert_eq!(
            decode_svg_picture(&bad_rule),
            Err(CodecError::BadEnum { ty: "FillRule", raw: 9 })
        );

        // Bad bool tag on fill Option (offset 20).
        let mut bad_bool = bytes.clone();
        bad_bool[20] = 2;
        assert_eq!(decode_svg_picture(&bad_bool), Err(CodecError::BadBool(2)));

        // Bad VectorPathCmd tag (offset 39 = 34 + 1 fill_rule + 4 cmd_count).
        let mut bad_cmd = bytes.clone();
        bad_cmd[39] = 99;
        assert_eq!(
            decode_svg_picture(&bad_cmd),
            Err(CodecError::BadEnum { ty: "VectorPathCmd", raw: 99 })
        );

        // Lying shape count and lying cmd count must be rejected before allocating.
        let mut lying_shapes = [0u8; 20];
        lying_shapes[16..20].copy_from_slice(&((MAX_SVG_SHAPES + 1) as u32).to_le_bytes());
        assert_eq!(
            decode_svg_picture(&lying_shapes),
            Err(CodecError::LenOverflow {
                len: MAX_SVG_SHAPES + 1,
                max: MAX_SVG_SHAPES,
            })
        );

        // Encode-side caps reject MAX_SVG_SHAPES + 1 and MAX_SVG_CMDS + 1 without partial output.
        let dummy_shape = VectorShape::filled(vec![VectorPathCmd::Close], ColorRgba::BLACK);
        let too_many_shapes = SvgPicture::new((0.0, 0.0, 10.0, 10.0), vec![dummy_shape; MAX_SVG_SHAPES + 1]);
        let mut w_cap = ByteWriter::new();
        assert_eq!(
            encode_svg_picture(&too_many_shapes, &mut w_cap),
            Err(CodecError::LenOverflow {
                len: MAX_SVG_SHAPES + 1,
                max: MAX_SVG_SHAPES,
            })
        );
        let mut out_rollback = vec![0xDEAD_BEEFu32];
        assert!(!too_many_shapes.write_words(&mut out_rollback));
        assert_eq!(out_rollback, vec![0xDEAD_BEEFu32], "failed FlatResource::write_words must leave zero partial words");

        let too_many_cmds = SvgPicture::new(
            (0.0, 0.0, 10.0, 10.0),
            vec![VectorShape::filled(vec![VectorPathCmd::Close; MAX_SVG_CMDS + 1], ColorRgba::BLACK)],
        );
        let mut w_cmds = ByteWriter::new();
        assert_eq!(
            encode_svg_picture(&too_many_cmds, &mut w_cmds),
            Err(CodecError::LenOverflow {
                len: MAX_SVG_CMDS + 1,
                max: MAX_SVG_CMDS,
            })
        );
    }
}
