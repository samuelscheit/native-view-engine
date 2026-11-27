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

pub use nve_abi::ColorRgba;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use nve_store::wire::{WireDecoder, WireEncoder};
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore, ValidatedWords};

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
        assert_eq!(got.to_owned(), pic);

        let mut words = Vec::new();
        assert!(pic.write_words(&mut words));
        let vw = ValidatedWords::<SvgPicture>::try_new(&words).expect("valid flat SvgPicture");
        assert_eq!(SvgPicture::view(vw).to_owned(), pic);
        assert!(!SvgPicture::validate(&[]));
        assert!(!SvgPicture::validate(&[0, 0, 0, 0, 0, 0, 99999]));
    }

    #[test]
    fn svg_flat_resource_roundtrips_all_variants_rejects_spoofed_hash_and_caps() {
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

        let mut words = Vec::new();
        assert!(rich.write_words(&mut words));
        let vw = ValidatedWords::<SvgPicture>::try_new(&words).expect("valid rich SvgPicture");
        assert_eq!(SvgPicture::view(vw).to_owned(), rich);

        // Per-word truncation across the entire flat buffer must fail cleanly.
        for cut in 0..words.len() {
            assert!(!SvgPicture::validate(&words[..cut]), "cut {cut} must fail");
        }
        // Trailing word rejected.
        let mut trailing = words.clone();
        trailing.push(0);
        assert!(!SvgPicture::validate(&trailing));

        // Spoofed `content_hash` MUST be rejected (prevents process-global `SvgCache` poisoning).
        let mut spoofed = words.clone();
        spoofed[0] ^= 1;
        assert!(!SvgPicture::validate(&spoofed), "spoofed content_hash lo must fail");
        let mut spoofed_hi = words.clone();
        spoofed_hi[1] ^= 0x8000_0000;
        assert!(!SvgPicture::validate(&spoofed_hi), "spoofed content_hash hi must fail");
        // Tampered coordinate with stale hash must also fail.
        let mut tampered_coord = words.clone();
        tampered_coord[2] = 999.0f32.to_bits();
        assert!(!SvgPicture::validate(&tampered_coord), "tampered viewBox with stale hash must fail");

        // End-to-end wire test: legitimate SvgPicture passes, tampered wire payload is rejected on replica.
        let mut prod = NodeStore::new();
        let mut tx = prod.tx();
        let node = tx.create();
        tx.set_children(NodeId::ROOT, &[node]);
        tx.edit::<Svg>(node).picture(Some(rich.clone()));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);
        let mut wire = Vec::new();
        WireEncoder::default().encode(&prod, &changes, &mut wire);

        let mut replica = NodeStore::replica();
        let mut dec = WireDecoder::default();
        let mut rtx = replica.tx();
        dec.apply(&mut rtx, &wire).unwrap();
        rtx.commit_into(&mut ChangeSet::default());
        let rid = replica.try_row::<Svg>(node).unwrap().picture();
        assert_eq!(replica.resource::<SvgPicture>(rid).unwrap().to_owned(), rich);

        // Tamper with the encoded content_hash bytes inside the wire frame -> replica refuses to view it.
        let hash_bytes = rich.content_hash.to_le_bytes();
        let pos = wire
            .windows(8)
            .position(|w| w == hash_bytes)
            .expect("content_hash present in wire frame");
        let mut poisoned_wire = wire.clone();
        poisoned_wire[pos] ^= 0xFF;
        let mut poisoned_replica = NodeStore::replica();
        let mut pdec = WireDecoder::default();
        let mut prtx = poisoned_replica.tx();
        pdec.apply(&mut prtx, &poisoned_wire).unwrap();
        prtx.commit_into(&mut ChangeSet::default());
        let prid = poisoned_replica.try_row::<Svg>(node).unwrap().picture();
        assert!(
            poisoned_replica.resource::<SvgPicture>(prid).is_none(),
            "wire replica must reject SvgPicture with forged content_hash before cache lookup"
        );

        // Bad shape flags and bad command tags rejected.
        let mut bad_flags = words.clone();
        bad_flags[7] |= 1 << 5;
        assert!(!SvgPicture::validate(&bad_flags));

        // Encode-side caps reject MAX_SVG_SHAPES + 1 and MAX_SVG_CMDS + 1 without partial output.
        let dummy_shape = VectorShape::filled(vec![VectorPathCmd::Close], ColorRgba::BLACK);
        let too_many_shapes =
            SvgPicture::new((0.0, 0.0, 10.0, 10.0), vec![dummy_shape; MAX_SVG_SHAPES + 1]);
        let mut out_rollback = vec![0xDEAD_BEEFu32];
        assert!(!too_many_shapes.write_words(&mut out_rollback));
        assert_eq!(out_rollback, vec![0xDEAD_BEEFu32]);

        let too_many_cmds = SvgPicture::new(
            (0.0, 0.0, 10.0, 10.0),
            vec![VectorShape::filled(
                vec![VectorPathCmd::Close; MAX_SVG_CMDS + 1],
                ColorRgba::BLACK,
            )],
        );
        let mut out_cmds = vec![0xCAFE_BABEu32];
        assert!(!too_many_cmds.write_words(&mut out_cmds));
        assert_eq!(out_cmds, vec![0xCAFE_BABEu32]);
    }
}
