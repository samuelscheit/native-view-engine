use nve_abi::{ColorRgba, NVE_PRIMARY_SLOT_NONE};
use nve_components::style_patch::{LayoutSnapshot, VisualSnapshot};

use crate::reloc::{
    compute_reloc_checksum, encode_scene_reloc, NveTimelineArchive, NveTimelineArchiveView,
    NveTimelineEntryInput, NveTimelinePatchInput, RelocBuf, RelocError, SceneRelocInput,
    SceneRelocNodeInput, VerifyMode,
};
use crate::{LayoutRect, Viewport};

fn make_node<'a>(
    node_id: u32,
    parent_index: i32,
    primary_slot: u8,
    text: &'a str,
    children: &'a [u32],
) -> SceneRelocNodeInput<'a> {
    SceneRelocNodeInput {
        node_id,
        parent_index,
        primary_slot,
        flags: 0,
        text_align: 0,
        image_resize_mode: 0,
        font_weight: 400,
        event_mask: 0,
        computed_rect: LayoutRect::default(),
        style: VisualSnapshot::default(),
        layout: LayoutSnapshot::default(),
        text,
        tag: "",
        image_src: "",
        box_shadow: "",
        draw_list: &[],
        children,
    }
}

fn sample_base_scene() -> RelocBuf {
    let nodes = [
        make_node(1, -1, NVE_PRIMARY_SLOT_NONE, "", &[1, 2]),
        make_node(2, 0, 1, "0 - 0", &[]),
        make_node(3, 0, 1, "Q1 12:00", &[]),
    ];
    encode_scene_reloc(&SceneRelocInput {
        schema_version: 1,
        surface_id: 1,
        frame_id: 1,
        viewport: Viewport {
            width: 360.0,
            height: 160.0,
            scale_factor: 3.0,
        },
        clear_color: ColorRgba::rgba(15, 23, 42, 255),
        root_node_index: 0,
        app_tag: "live-activity-score",
        nodes: &nodes,
    })
    .expect("encode base scene")
}

#[test]
fn timeline_archive_roundtrip_binary_search_and_integrity() {
    let base = sample_base_scene();
    let archive = NveTimelineArchive::build(
        base.as_slice(),
        &[
            // Intentionally unsorted input to verify encoder sorts ascending.
            NveTimelineEntryInput {
                activate_at_unix_ms: 2_000,
                patches: vec![
                    NveTimelinePatchInput::text(2, "14 - 10"),
                    NveTimelinePatchInput::text(3, "Q2 04:15"),
                    NveTimelinePatchInput::bg_rgba(1, 0x1E293BFF),
                ],
            },
            NveTimelineEntryInput {
                activate_at_unix_ms: 1_000,
                patches: vec![
                    NveTimelinePatchInput::text(2, "7 - 0"),
                    NveTimelinePatchInput::text(3, "Q1 08:30"),
                ],
            },
            NveTimelineEntryInput {
                activate_at_unix_ms: 3_000,
                patches: vec![
                    NveTimelinePatchInput::text(2, "21 - 17"),
                    NveTimelinePatchInput::opacity(3, 0.75).with_transition_ms(220),
                ],
            },
        ],
        500,
        10_000,
        1,
    )
    .expect("build timeline");

    let view = archive.view(VerifyMode::Full).expect("verify full");
    assert_eq!(view.entries().len(), 3);
    assert_eq!(view.entries()[0].activate_at_unix_ms, 1_000);
    assert_eq!(view.entries()[1].activate_at_unix_ms, 2_000);
    assert_eq!(view.entries()[2].activate_at_unix_ms, 3_000);

    let scene = view.base_scene().expect("open embedded base scene");
    assert_eq!(scene.nodes().len(), 3);
    assert_eq!(scene.text_of(&scene.nodes()[1]).unwrap(), "0 - 0");

    // Binary search before, at, between, and after scheduled timestamps.
    assert!(view.active_entry_at(999).is_none());
    assert_eq!(view.active_patches_at(999).unwrap().len(), 0);

    let (idx1, _) = view.active_entry_at(1_000).unwrap();
    assert_eq!(idx1, 0);
    let p1 = view.active_patches_at(1_500).unwrap();
    assert_eq!(p1.len(), 2);
    assert_eq!(view.patch_str(&p1[0]).unwrap(), "7 - 0");
    assert_eq!(view.patch_str(&p1[1]).unwrap(), "Q1 08:30");

    let (idx2, _) = view.active_entry_at(2_999).unwrap();
    assert_eq!(idx2, 1);
    let p2 = view.active_patches_at(2_999).unwrap();
    assert_eq!(view.patch_str(&p2[0]).unwrap(), "14 - 10");
    assert_eq!(p2[2].value_lo, 0x1E293BFF);

    let (idx3, _) = view.active_entry_at(9_999).unwrap();
    assert_eq!(idx3, 2);
    let p3 = view.active_patches_at(9_999).unwrap();
    assert_eq!(p3[1].as_f32(), 0.75);
    assert_eq!(p3[1].transition_ms, 220);
    assert!(!view.is_expired_at(9_999));
    assert!(view.is_expired_at(10_000));

    // Corrupt checksum must fail Full verification.
    let mut corrupt = archive.as_slice().to_vec();
    let mid = corrupt.len() / 2;
    corrupt[mid] ^= 0xFF;
    let aligned_corrupt = RelocBuf::from_bytes(&corrupt);
    assert!(matches!(
        NveTimelineArchiveView::from_bytes(aligned_corrupt.as_slice(), VerifyMode::Full),
        Err(RelocError::ChecksumMismatch { .. })
    ));
}

#[test]
fn timeline_rejects_corrupt_oob_and_unsorted_entries() {
    let base = sample_base_scene();
    let archive = NveTimelineArchive::build(
        base.as_slice(),
        &[
            NveTimelineEntryInput {
                activate_at_unix_ms: 1_000,
                patches: vec![NveTimelinePatchInput::text(2, "A")],
            },
            NveTimelineEntryInput {
                activate_at_unix_ms: 2_000,
                patches: vec![NveTimelinePatchInput::text(2, "B")],
            },
        ],
        100,
        0,
        1,
    )
    .unwrap();

    // Mutate entry[1].activate_at_unix_ms < entry[0] and recompute checksum -> structural reject.
    let mut raw = archive.as_slice().to_vec();
    let view = archive.view(VerifyMode::Full).unwrap();
    let entries_off = view.root().entries.offset as usize;
    raw[entries_off + 16..entries_off + 24].copy_from_slice(&500u64.to_le_bytes());
    let new_ck = compute_reloc_checksum(&raw);
    raw[24..32].copy_from_slice(&new_ck.to_le_bytes());
    let aligned = RelocBuf::from_bytes(&raw);
    assert!(matches!(
        NveTimelineArchiveView::from_bytes(aligned.as_slice(), VerifyMode::Full),
        Err(RelocError::OutOfBounds("timeline_entries_unsorted"))
    ));

    // Corrupt entry[0].patches.offset out-of-bounds.
    let mut raw_oob = archive.as_slice().to_vec();
    raw_oob[entries_off + 8..entries_off + 12].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    let ck_oob = compute_reloc_checksum(&raw_oob);
    raw_oob[24..32].copy_from_slice(&ck_oob.to_le_bytes());
    let aligned_oob = RelocBuf::from_bytes(&raw_oob);
    assert!(matches!(
        NveTimelineArchiveView::from_bytes(aligned_oob.as_slice(), VerifyMode::Full),
        Err(RelocError::OutOfBounds(_))
    ));
}
