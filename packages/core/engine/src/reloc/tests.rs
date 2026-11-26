use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_abi::{
    ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify, NVE_PRIMARY_SLOT_NONE,
};
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};

use crate::{LayoutRect, Viewport};

use super::*;

static RELOC_TEST_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_reloc_path(tag: &str) -> PathBuf {
    let seq = RELOC_TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nve_reloc_test_{tag}_{}_{seq}.nvereloc",
        std::process::id()
    ))
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SampleRecord {
    id: u64,
    score: u32,
    flags: u32,
    name: RelocStr,
}

// SAFETY: `#[repr(C)]` POD struct composed solely of `ArenaPod` fields.
unsafe impl crate::mmap_arena::ArenaPod for SampleRecord {}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SampleRoot {
    count: u64,
    records: RelocSlice<SampleRecord>,
    label: RelocStr,
}

// SAFETY: `#[repr(C)]` POD struct composed solely of `ArenaPod` fields.
unsafe impl crate::mmap_arena::ArenaPod for SampleRoot {}

#[test]
fn generic_reloc_roundtrip_all_verify_modes_and_mmap() {
    let mut w = RelocWriter::new(RELOC_KIND_GENERIC, 7, 0x0001_0002);
    let n0 = w.write_str("alpha").unwrap();
    let n1 = w.write_str("beta-🚀").unwrap();
    let recs = w
        .write_slice(&[
            SampleRecord {
                id: 101,
                score: 50,
                flags: 1,
                name: n0,
            },
            SampleRecord {
                id: 202,
                score: 99,
                flags: 3,
                name: n1,
            },
        ])
        .unwrap();
    let label = w.write_str("studio-reloc").unwrap();
    let root_ptr = w
        .write_val(&SampleRoot {
            count: 2,
            records: recs,
            label,
        })
        .unwrap();
    let buf = w.finish(root_ptr).unwrap();

    assert_eq!(buf.as_ptr() as usize % RELOC_BUF_ALIGN, 0);
    assert_eq!(&buf[0..8], &RELOC_MAGIC);

    // Full, HeaderOnly, and Unchecked all read identical data.
    let v_full = buf.view(VerifyMode::Full).unwrap();
    let v_hdr = buf.view(VerifyMode::HeaderOnly).unwrap();
    // SAFETY: Just produced by `RelocWriter::finish`.
    let v_unchk = unsafe { RelocView::from_bytes_unchecked(&buf) };

    for v in [v_full, v_hdr, v_unchk] {
        assert_eq!(v.header().version, RELOC_VERSION);
        assert_eq!(v.header().schema_version, 7);
        assert_eq!(v.header().flags, 0x0001_0002);
        let root = v.root::<SampleRoot>().unwrap();
        assert_eq!(root.count, 2);
        assert_eq!(v.str_at(root.label).unwrap(), "studio-reloc");
        let list = v.slice(root.records).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 101);
        assert_eq!(v.str_at(list[0].name).unwrap(), "alpha");
        assert_eq!(list[1].id, 202);
        assert_eq!(v.str_at(list[1].name).unwrap(), "beta-🚀");
    }

    // Atomic write + mmap reopen with schema check.
    let path = temp_reloc_path("generic");
    buf.write_atomic(&path).unwrap();

    let mapped = MmapReloc::open_with_schema(&path, 7, VerifyMode::Full).unwrap();
    assert_eq!(mapped.as_bytes(), buf.as_slice());
    let root = mapped.view().root::<SampleRoot>().unwrap();
    assert_eq!(mapped.view().str_at(root.label).unwrap(), "studio-reloc");

    // Wrong schema rejected.
    let wrong_schema = MmapReloc::open_with_schema(&path, 8, VerifyMode::Full);
    assert!(matches!(
        wrong_schema,
        Err(RelocError::SchemaVersionMismatch {
            expected: 8,
            actual: 7
        })
    ));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn rejects_corrupt_magic_version_reserved_checksum_truncated_oob_and_misaligned() {
    let bundle = encode_reloc_bundle(3, 9, b"FRAME0_BYTES", b"STATE_BYTES", b"META").unwrap();

    // Truncated at every prefix < 64.
    for cut in [0usize, 1, 8, 32, 63] {
        assert!(matches!(
            RelocView::from_bytes(&bundle[..cut], VerifyMode::HeaderOnly),
            Err(RelocError::Truncated)
        ));
    }

    // Payload length mismatch (truncated after header).
    assert!(matches!(
        RelocView::from_bytes(&bundle[..bundle.len() - 1], VerifyMode::HeaderOnly),
        Err(RelocError::PayloadLengthMismatch { .. })
    ));

    // Misaligned buffer (+1 byte).
    let mut shifted = vec![0u8; bundle.len() + 16];
    let align_pad = (8 - (shifted.as_ptr() as usize % 8)) % 8;
    let start = align_pad + 1;
    shifted[start..start + bundle.len()].copy_from_slice(&bundle);
    assert!(matches!(
        RelocView::from_bytes(&shifted[start..start + bundle.len()], VerifyMode::HeaderOnly),
        Err(RelocError::Misaligned { .. })
    ));

    // Bad magic.
    let mut bad_magic = bundle.as_slice().to_vec();
    bad_magic[0] = b'X';
    assert!(matches!(
        RelocView::from_bytes(&RelocBuf::from_bytes(&bad_magic), VerifyMode::HeaderOnly),
        Err(RelocError::InvalidMagic)
    ));

    // Unsupported version.
    let mut bad_ver = bundle.as_slice().to_vec();
    bad_ver[8..12].copy_from_slice(&99u32.to_le_bytes());
    assert!(matches!(
        RelocView::from_bytes(&RelocBuf::from_bytes(&bad_ver), VerifyMode::HeaderOnly),
        Err(RelocError::UnsupportedVersion(99))
    ));

    // Non-zero reserved bytes.
    let mut bad_res = bundle.as_slice().to_vec();
    bad_res[40] = 1;
    assert!(matches!(
        RelocView::from_bytes(&RelocBuf::from_bytes(&bad_res), VerifyMode::HeaderOnly),
        Err(RelocError::InvalidReservedHeader)
    ));

    // Root offset inside header (< 64) or past EOF.
    let mut bad_root = bundle.as_slice().to_vec();
    bad_root[20..24].copy_from_slice(&16u32.to_le_bytes());
    assert!(matches!(
        RelocView::from_bytes(&RelocBuf::from_bytes(&bad_root), VerifyMode::HeaderOnly),
        Err(RelocError::OutOfBounds("root_offset"))
    ));
    bad_root[20..24].copy_from_slice(&(bundle.len() as u32).to_le_bytes());
    assert!(matches!(
        RelocView::from_bytes(&RelocBuf::from_bytes(&bad_root), VerifyMode::HeaderOnly),
        Err(RelocError::OutOfBounds("root_offset"))
    ));

    // Bit flip in payload: HeaderOnly passes, Full catches ChecksumMismatch.
    let mut flipped = bundle.as_slice().to_vec();
    let last = flipped.len() - 1;
    flipped[last] ^= 0x01;
    let flipped_buf = RelocBuf::from_bytes(&flipped);
    assert!(RelocView::from_bytes(&flipped_buf, VerifyMode::HeaderOnly).is_ok());
    assert!(matches!(
        RelocView::from_bytes(&flipped_buf, VerifyMode::Full),
        Err(RelocError::ChecksumMismatch { .. })
    ));

    // Bundle view roundtrip check.
    let bv = RelocBundleView::from_bytes(&bundle, VerifyMode::Full).unwrap();
    assert_eq!(bv.header().schema_version, 3);
    assert_eq!(bv.header().flags, 9);
    assert_eq!(bv.frame0(), b"FRAME0_BYTES");
    assert_eq!(bv.state(), b"STATE_BYTES");
    assert_eq!(bv.meta(), b"META");
}

#[test]
fn scene_reloc_roundtrip_and_corruption_guards() {
    let lp_root = LayoutSnapshot {
        width: Dimension::Points(1280.0),
        height: Dimension::Points(800.0),
        min_width: Dimension::Points(320.0),
        max_height: Dimension::Percent(100.0),
        flex_direction: FlexDirection::Row,
        justify_content: FlexJustify::SpaceBetween,
        align_items: FlexAlign::Center,
        flex_grow: 1.0,
        flex_shrink: 0.0,
        gap: 12.0,
        padding: EdgeInsets::symmetric(8.0, 16.0),
        margin: EdgeInsets::all(4.0),
        ..Default::default()
    };
    let children_root = [1u32, 2u32];
    let draw_bytes = [0x4Eu8, 0x56, 0x45, 0x5F, 0x44, 0x52, 0x41, 0x57, 0x31];

    let nodes = [
        SceneRelocNodeInput {
            node_id: 1,
            parent_index: -1,
            primary_slot: NVE_PRIMARY_SLOT_NONE,
            flags: SCENE_NODE_CLIP_CHILDREN,
            text_align: 0,
            image_resize_mode: 0,
            font_weight: 400,
            event_mask: 0,
            computed_rect: LayoutRect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 800.0,
            },
            style: VisualSnapshot {
                background: ColorRgba::rgb(11, 17, 32),
                ..Default::default()
            },
            layout: lp_root,
            text: "",
            tag: "root",
            image_src: "",
            box_shadow: "",
            draw_list: &[],
            children: &children_root,
        },
        SceneRelocNodeInput {
            node_id: 2,
            parent_index: 0,
            primary_slot: 1,
            flags: SCENE_NODE_HAS_EXPLICIT_STYLE | SCENE_NODE_INTERACTIVE,
            text_align: 1,
            image_resize_mode: 0,
            font_weight: 700,
            event_mask: 1,
            computed_rect: LayoutRect {
                x: 16.0,
                y: 12.0,
                width: 240.0,
                height: 32.0,
            },
            style: VisualSnapshot {
                text_color: ColorRgba::WHITE,
                font_size: 22.0,
                border_radius: 8.0,
                ..Default::default()
            },
            layout: LayoutSnapshot::default(),
            text: "Aether Studio Instant Boot",
            tag: "hero-title",
            image_src: "",
            box_shadow: "0 4px 12px rgba(0,0,0,0.4)",
            draw_list: &[],
            children: &[],
        },
        SceneRelocNodeInput {
            node_id: 3,
            parent_index: 0,
            primary_slot: 2,
            flags: 0,
            text_align: 0,
            image_resize_mode: 1,
            font_weight: 400,
            event_mask: 0,
            computed_rect: LayoutRect {
                x: 16.0,
                y: 60.0,
                width: 320.0,
                height: 120.0,
            },
            style: VisualSnapshot::default(),
            layout: LayoutSnapshot::default(),
            text: "",
            tag: "sparkline",
            image_src: "asset://chart.png",
            box_shadow: "",
            draw_list: &draw_bytes,
            children: &[],
        },
    ];

    let buf = encode_scene_reloc(&SceneRelocInput {
        schema_version: 2,
        surface_id: 1,
        frame_id: 1,
        viewport: Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 2.0,
        },
        clear_color: ColorRgba::rgb(11, 17, 32),
        root_node_index: 0,
        app_tag: "studio",
        nodes: &nodes,
    })
    .unwrap();

    let scene = SceneRelocView::from_bytes(&buf, VerifyMode::Full).unwrap();
    assert_eq!(scene.schema_version(), 2);
    assert_eq!(scene.app_tag().unwrap(), "studio");
    assert_eq!(scene.viewport().width, 1280.0);
    assert_eq!(scene.viewport().scale_factor, 2.0);
    assert_eq!(scene.nodes().len(), 3);
    assert_eq!(scene.layout_of_index(0).unwrap(), lp_root);
    assert_eq!(scene.children_of(&scene.nodes()[0]).unwrap(), &[1, 2]);
    assert_eq!(
        scene.text_of(&scene.nodes()[1]).unwrap(),
        "Aether Studio Instant Boot"
    );
    assert_eq!(scene.tag_of(&scene.nodes()[1]).unwrap(), "hero-title");
    assert_eq!(
        scene.box_shadow_of(&scene.nodes()[1]).unwrap(),
        "0 4px 12px rgba(0,0,0,0.4)"
    );
    assert_eq!(scene.draw_list_of(&scene.nodes()[2]).unwrap(), &draw_bytes);
    assert_eq!(
        scene.image_src_of(&scene.nodes()[2]).unwrap(),
        "asset://chart.png"
    );

    // Corrupt UTF-8 with recomputed checksum is still caught by `VerifyMode::Full`.
    let mut bad_utf8 = buf.as_slice().to_vec();
    let text_off = scene.nodes()[1].text.offset as usize;
    bad_utf8[text_off] = 0xFF;
    bad_utf8[text_off + 1] = 0xFE;
    let ck = compute_reloc_checksum(&bad_utf8);
    bad_utf8[24..32].copy_from_slice(&ck.to_le_bytes());
    let bad_buf = RelocBuf::from_bytes(&bad_utf8);
    assert!(matches!(
        SceneRelocView::from_bytes(&bad_buf, VerifyMode::Full),
        Err(RelocError::InvalidUtf8)
    ));

    // Content kind mismatch rejected when opening bundle as scene.
    let bundle = encode_reloc_bundle(2, 0, b"f0", b"st", b"").unwrap();
    assert!(matches!(
        SceneRelocView::from_bytes(&bundle, VerifyMode::Full),
        Err(RelocError::ContentKindMismatch {
            expected: RELOC_KIND_SCENE,
            actual: RELOC_KIND_BUNDLE,
        })
    ));
}
