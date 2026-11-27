use std::mem::{align_of, size_of};
use std::path::Path;

use super::*;
use crate::{ByteLen, FrameId, NodeId, PrimarySlot, ShortLen, ZIndex};

#[test]
fn render_node_is_exactly_one_64_byte_cache_line() {
    assert_eq!(size_of::<NveRenderNode>(), 64);
    assert_eq!(align_of::<NveRenderNode>(), 64);
    assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
}

#[test]
fn incremental_table_tracks_only_changed_and_removed_nodes() {
    let mut table = NveSceneTable::with_capacity(1000);
    table.begin_frame(FrameId(1));
    for id in 1..=1000u32 {
        let label = if id % 2 == 0 { "even" } else { "odd" };
        let children: &[NodeId] = if id == 1 {
            &[NodeId(2), NodeId(3), NodeId(4)]
        } else {
            &[]
        };
        table
            .upsert_node(SceneNodeInput {
                node_id: NodeId(id),
                primary_slot: if id == 1 {
                    PrimarySlot::NONE
                } else if id % 3 == 0 {
                    PrimarySlot(2)
                } else {
                    PrimarySlot(1)
                },
                flags: if id % 3 == 0 {
                    NVE_NODE_INTERACTIVE
                } else {
                    NVE_NODE_LEAF_TEXT
                },
                opacity_u8: 255,
                x: id as f32,
                y: (id * 2) as f32,
                width: 120.0,
                height: 36.0,
                bg_rgba: 0xFF11_2233,
                text_rgba: 0xFFFF_FFFF,
                border_rgba: 0,
                border_radius: 8.0,
                border_width: 1.0,
                font_size: 16.0,
                z_index: ZIndex::ZERO,
                text: label,
                children,
            })
            .unwrap();
    }
    assert_eq!(table.len(), 1000);
    assert_eq!(table.as_ref().dirty_indices().len(), 1000);

    // Frame 2: touch only 3 nodes out of 1000.
    table.begin_frame(FrameId(2));
    let slab_before = table.as_ref().text_slab().len();
    for &id in &[10u32, 42, 999] {
        table
            .upsert_node(SceneNodeInput {
                node_id: NodeId(id),
                primary_slot: PrimarySlot(1),
                flags: NVE_NODE_LEAF_TEXT,
                opacity_u8: 200,
                x: id as f32 + 5.0,
                y: (id * 2) as f32,
                width: 120.0,
                height: 36.0,
                bg_rgba: 0xFF11_2233,
                text_rgba: 0xFFFF_FFFF,
                border_rgba: 0,
                border_radius: 8.0,
                border_width: 1.0,
                font_size: 16.0,
                z_index: ZIndex(1),
                text: "updated",
                children: &[],
            })
            .unwrap();
    }
    // Also upsert an identical node: must NOT be marked dirty or grow text_slab.
    table
        .upsert_node(SceneNodeInput {
            node_id: NodeId(2),
            primary_slot: PrimarySlot(1),
            flags: NVE_NODE_LEAF_TEXT,
            opacity_u8: 255,
            x: 2.0,
            y: 4.0,
            width: 120.0,
            height: 36.0,
            bg_rgba: 0xFF11_2233,
            text_rgba: 0xFFFF_FFFF,
            border_rgba: 0,
            border_radius: 8.0,
            border_width: 1.0,
            font_size: 16.0,
            z_index: ZIndex::ZERO,
            text: "even",
            children: &[],
        })
        .unwrap();

    let view = table.as_view();
    assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(view.frame_id, FrameId(2));
    assert_eq!(view.node_count, 1000);
    assert_eq!(view.dirty_count, 3);
    assert_eq!(view.removed_count, 0);
    assert_eq!(
        view.text_slab_len.as_usize(),
        slab_before + 3 * "updated".len()
    );

    // SAFETY: `view` borrows `table` which remains alive and unmutated for `reader`.
    let reader = unsafe { NveSceneRef::from_raw(view) }.expect("valid raw view");
    let dirty_ids: Vec<NodeId> = reader.dirty_nodes().map(|(_, n)| n.node_id).collect();
    assert_eq!(dirty_ids, vec![NodeId(10), NodeId(42), NodeId(999)]);
    for (_, node) in reader.dirty_nodes() {
        assert_eq!(reader.node_text(node).unwrap(), "updated");
        assert_eq!(node.opacity_u8, 200);
    }
    let root = reader.node(table.index_of(NodeId(1)).unwrap()).unwrap();
    assert_eq!(
        reader.node_children(root).unwrap(),
        &[NodeId(2), NodeId(3), NodeId(4)]
    );

    // Frame 3: remove nodes 10 and 1000 via swap-remove.
    table.begin_frame(FrameId(3));
    assert!(table.remove_node(NodeId(10)));
    assert!(table.remove_node(NodeId(1000)));
    assert!(!table.remove_node(NodeId(10)));
    assert_eq!(table.len(), 998);
    let reader3 = table.as_ref();
    assert_eq!(reader3.removed_ids(), &[NodeId(10), NodeId(1000)]);
    assert!(table.index_of(NodeId(10)).is_none());
    assert!(table.index_of(NodeId(1000)).is_none());
    // Node 999 was swapped into slot of 10 (after 1000 moved into slot of 10, then 1000 removed).
    let idx999 = table.index_of(NodeId(999)).unwrap();
    assert_eq!(reader3.node(idx999).unwrap().node_id, NodeId(999));
    assert_eq!(
        reader3.node_text(reader3.node(idx999).unwrap()).unwrap(),
        "updated"
    );
}

#[test]
fn sync_frame_and_compaction_preserve_live_data() {
    let mut table = NveSceneTable::new();
    table
        .sync_frame(
            FrameId(1),
            [
                SceneNodeInput {
                    node_id: NodeId(1),
                    primary_slot: PrimarySlot::NONE,
                    children: &[NodeId(2), NodeId(3)],
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: NodeId(2),
                    primary_slot: PrimarySlot(1),
                    flags: NVE_NODE_LEAF_TEXT,
                    text: "hello",
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: NodeId(3),
                    primary_slot: PrimarySlot(1),
                    flags: NVE_NODE_LEAF_TEXT,
                    text: "world",
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    assert_eq!(table.as_ref().dirty_indices().len(), 3);

    // Frame 2: remove node 3, update node 2 text.
    table
        .sync_frame(
            FrameId(2),
            [
                SceneNodeInput {
                    node_id: NodeId(1),
                    primary_slot: PrimarySlot::NONE,
                    children: &[NodeId(2)],
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: NodeId(2),
                    primary_slot: PrimarySlot(1),
                    flags: NVE_NODE_LEAF_TEXT,
                    text: "updated 🚀",
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table.as_ref().removed_ids(), &[NodeId(3)]);

    table.compact_text_slab();
    table.compact_children_slab();
    let reader = table.as_ref();
    assert_eq!(reader.text_slab(), "updated 🚀".as_bytes());
    assert_eq!(reader.children_slab(), &[NodeId(2)]);
    let n2 = reader.node(table.index_of(NodeId(2)).unwrap()).unwrap();
    assert_eq!(reader.node_text(n2).unwrap(), "updated 🚀");
}

#[test]
fn raw_scene_ref_rejects_corrupt_views() {
    let mut view = NveSceneTable::new().as_view();
    view.schema_hash ^= 1;
    // SAFETY: `from_raw` validates `schema_hash` before dereferencing any pointers.
    let bad_hash = unsafe { NveSceneRef::from_raw(view) }.unwrap_err();
    assert_eq!(
        bad_hash,
        SceneError::SchemaHashMismatch {
            expected: NVE_SCENE_SCHEMA_HASH,
            actual: NVE_SCENE_SCHEMA_HASH ^ 1,
        }
    );

    let mut null_nodes = NveSceneTable::new().as_view();
    null_nodes.node_count = 1;
    // SAFETY: `from_raw` validates non-null pointers before constructing slices.
    let null_err = unsafe { NveSceneRef::from_raw(null_nodes) }.unwrap_err();
    assert_eq!(null_err, SceneError::NullBufferPointer("nodes"));

    // Out-of-bounds text & invalid UTF-8 checks.
    let mut node = NveRenderNode::zeroed();
    node.node_id = NodeId(7);
    node.text_offset = 0;
    node.text_len = ShortLen(2);
    let bad_utf8: [u8; 2] = [0xFF, 0xFE];
    let valid_view = NveSceneView {
        schema_hash: NVE_SCENE_SCHEMA_HASH,
        frame_id: FrameId(1),
        node_count: 1,
        dirty_count: 0,
        removed_count: 0,
        text_slab_len: ByteLen(2),
        children_slab_len: ByteLen::ZERO,
        table_count: 0,
        nodes: &node,
        text_slab: bad_utf8.as_ptr(),
        children_slab: std::ptr::null(),
        dirty_indices: std::ptr::null(),
        removed_ids: std::ptr::null(),
        tables: std::ptr::null(),
    };
    // SAFETY: `valid_view` points into live local stack buffers in this test.
    let reader = unsafe { NveSceneRef::from_raw(valid_view) }.unwrap();
    assert_eq!(
        reader.node_text(&reader.nodes()[0]).unwrap_err(),
        SceneError::InvalidUtf8 { node_id: NodeId(7) }
    );

    node.text_len = ShortLen(10);
    let oob_view = NveSceneView {
        nodes: &node,
        ..valid_view
    };
    // SAFETY: `oob_view` points into live local stack buffers; `node_text` bounds-checks before slicing.
    let oob_reader = unsafe { NveSceneRef::from_raw(oob_view) }.unwrap();
    assert_eq!(
        oob_reader.node_text(&oob_reader.nodes()[0]).unwrap_err(),
        SceneError::TextOutOfBounds {
            node_id: NodeId(7),
            offset: 0,
            len: ShortLen(10),
            slab_len: 2,
        }
    );
}

#[test]
fn scene_c_header_matches_compile_time_schema_hash_when_present() {
    assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
    let header_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h");
    if !header_path.is_file() {
        return;
    }
    let content = std::fs::read_to_string(&header_path)
        .expect("read generated cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h");
    let expected_macro = format!("0x{NVE_SCENE_SCHEMA_HASH:016X}ULL");
    assert!(
        content.contains(&expected_macro),
        "nve_scene_abi.generated.h schema hash drift! Expected {expected_macro} in {}",
        header_path.display()
    );
}

#[test]
fn external_text_slab_view_and_ref_roundtrip() {
    let ext_slab = b"hello external";
    let mut table = NveSceneTable::new();
    table.begin_frame(FrameId(1));
    let idx = table
        .upsert_node_external_text(
            SceneNodeInput {
                node_id: NodeId(10),
                primary_slot: PrimarySlot(1),
                flags: NVE_NODE_LEAF_TEXT,
                ..Default::default()
            },
            6,
            ShortLen(8),
            false,
        )
        .unwrap();
    let r = table.as_ref_with_external_text(ext_slab);
    assert_eq!(r.node_text(&r.nodes()[idx as usize]).unwrap(), "external");
    let raw = table.as_view_with_external_text(ext_slab);
    // SAFETY: `raw` borrows live `table` and `ext_slab` slices for this test assertion.
    let from_raw = unsafe { NveSceneRef::from_raw(raw) }.unwrap();
    assert_eq!(from_raw.node_text(&from_raw.nodes()[idx as usize]).unwrap(), "external");
}

#[test]
#[should_panic(expected = "upsert_node_external_text cannot be mixed")]
fn external_text_slab_panics_after_internal_upsert() {
    let mut table = NveSceneTable::new();
    table
        .upsert_node(SceneNodeInput {
            node_id: NodeId(1),
            text: "internal",
            ..Default::default()
        })
        .unwrap();
    let _ = table.upsert_node_external_text(
        SceneNodeInput {
            node_id: NodeId(2),
            ..Default::default()
        },
        0,
        ShortLen::ZERO,
        false,
    );
}

#[test]
#[should_panic(expected = "upsert_node cannot be mixed")]
fn internal_upsert_panics_after_external_text_slab() {
    let mut table = NveSceneTable::new();
    table
        .upsert_node_external_text(
            SceneNodeInput {
                node_id: NodeId(1),
                ..Default::default()
            },
            0,
            ShortLen::ZERO,
            false,
        )
        .unwrap();
    let _ = table.upsert_node(SceneNodeInput {
        node_id: NodeId(2),
        text: "internal",
        ..Default::default()
    });
}

#[test]
#[should_panic(expected = "as_view called in external text slab mode")]
fn as_view_panics_in_external_text_slab_mode() {
    let mut table = NveSceneTable::new();
    table
        .upsert_node_external_text(
            SceneNodeInput {
                node_id: NodeId(1),
                ..Default::default()
            },
            0,
            ShortLen::ZERO,
            false,
        )
        .unwrap();
    let _ = table.as_view();
}

#[test]
#[should_panic(expected = "as_ref called in external text slab mode")]
fn as_ref_panics_in_external_text_slab_mode() {
    let mut table = NveSceneTable::new();
    table
        .upsert_node_external_text(
            SceneNodeInput {
                node_id: NodeId(1),
                ..Default::default()
            },
            0,
            ShortLen::ZERO,
            false,
        )
        .unwrap();
    let _ = table.as_ref();
}
