use std::mem::{align_of, size_of};
use std::path::Path;

use super::*;

#[test]
fn render_node_is_exactly_one_64_byte_cache_line() {
    assert_eq!(size_of::<NveRenderNode>(), 64);
    assert_eq!(align_of::<NveRenderNode>(), 64);
    assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
}

#[test]
fn incremental_table_tracks_only_changed_and_removed_nodes() {
    let mut table = NveSceneTable::with_capacity(1000);
    table.begin_frame(1);
    for id in 1..=1000u32 {
        let label = if id % 2 == 0 { "even" } else { "odd" };
        let children: &[u32] = if id == 1 { &[2, 3, 4] } else { &[] };
        table
            .upsert_node(SceneNodeInput {
                node_id: id,
                kind: if id == 1 {
                    NVE_NODE_KIND_ROOT
                } else if id % 3 == 0 {
                    NVE_NODE_KIND_PRESSABLE
                } else {
                    NVE_NODE_KIND_TEXT
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
                z_index: 0,
                text: label,
                children,
            })
            .unwrap();
    }
    assert_eq!(table.len(), 1000);
    assert_eq!(table.as_ref().dirty_indices().len(), 1000);

    // Frame 2: touch only 3 nodes out of 1000.
    table.begin_frame(2);
    let slab_before = table.as_ref().text_slab().len();
    for &id in &[10u32, 42, 999] {
        table
            .upsert_node(SceneNodeInput {
                node_id: id,
                kind: NVE_NODE_KIND_TEXT,
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
                z_index: 1,
                text: "updated",
                children: &[],
            })
            .unwrap();
    }
    // Also upsert an identical node: must NOT be marked dirty or grow text_slab.
    table
        .upsert_node(SceneNodeInput {
            node_id: 2,
            kind: NVE_NODE_KIND_TEXT,
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
            z_index: 0,
            text: "even",
            children: &[],
        })
        .unwrap();

    let view = table.as_view();
    assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(view.frame_id, 2);
    assert_eq!(view.node_count, 1000);
    assert_eq!(view.dirty_count, 3);
    assert_eq!(view.removed_count, 0);
    assert_eq!(view.text_slab_len as usize, slab_before + 3 * "updated".len());

    let reader = unsafe { NveSceneRef::from_raw(view) }.expect("valid raw view");
    let dirty_ids: Vec<u32> = reader.dirty_nodes().map(|(_, n)| n.node_id).collect();
    assert_eq!(dirty_ids, vec![10, 42, 999]);
    for (_, node) in reader.dirty_nodes() {
        assert_eq!(reader.node_text(node).unwrap(), "updated");
        assert_eq!(node.opacity_u8, 200);
    }
    let root = reader.node(table.index_of(1).unwrap()).unwrap();
    assert_eq!(reader.node_children(root).unwrap(), &[2, 3, 4]);

    // Frame 3: remove nodes 10 and 1000 via swap-remove.
    table.begin_frame(3);
    assert!(table.remove_node(10));
    assert!(table.remove_node(1000));
    assert!(!table.remove_node(10));
    assert_eq!(table.len(), 998);
    let reader3 = table.as_ref();
    assert_eq!(reader3.removed_ids(), &[10, 1000]);
    assert!(table.index_of(10).is_none());
    assert!(table.index_of(1000).is_none());
    // Node 999 was swapped into slot of 10 (after 1000 moved into slot of 10, then 1000 removed).
    let idx999 = table.index_of(999).unwrap();
    assert_eq!(reader3.node(idx999).unwrap().node_id, 999);
    assert_eq!(reader3.node_text(reader3.node(idx999).unwrap()).unwrap(), "updated");
}

#[test]
fn sync_frame_and_compaction_preserve_live_data() {
    let mut table = NveSceneTable::new();
    table
        .sync_frame(
            1,
            [
                SceneNodeInput {
                    node_id: 1,
                    kind: NVE_NODE_KIND_ROOT,
                    children: &[2, 3],
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: 2,
                    kind: NVE_NODE_KIND_TEXT,
                    flags: NVE_NODE_LEAF_TEXT,
                    text: "hello",
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: 3,
                    kind: NVE_NODE_KIND_TEXT,
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
            2,
            [
                SceneNodeInput {
                    node_id: 1,
                    kind: NVE_NODE_KIND_ROOT,
                    children: &[2],
                    ..Default::default()
                },
                SceneNodeInput {
                    node_id: 2,
                    kind: NVE_NODE_KIND_TEXT,
                    flags: NVE_NODE_LEAF_TEXT,
                    text: "updated 🚀",
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table.as_ref().removed_ids(), &[3]);

    table.compact_text_slab();
    table.compact_children_slab();
    let reader = table.as_ref();
    assert_eq!(reader.text_slab(), "updated 🚀".as_bytes());
    assert_eq!(reader.children_slab(), &[2]);
    let n2 = reader.node(table.index_of(2).unwrap()).unwrap();
    assert_eq!(reader.node_text(n2).unwrap(), "updated 🚀");
}

#[test]
fn raw_scene_ref_rejects_corrupt_views() {
    let mut view = NveSceneTable::new().as_view();
    view.schema_hash ^= 1;
    assert_eq!(
        unsafe { NveSceneRef::from_raw(view) }.unwrap_err(),
        SceneError::SchemaHashMismatch {
            expected: NVE_SCENE_SCHEMA_HASH,
            actual: NVE_SCENE_SCHEMA_HASH ^ 1,
        }
    );

    let mut null_nodes = NveSceneTable::new().as_view();
    null_nodes.node_count = 1;
    assert_eq!(
        unsafe { NveSceneRef::from_raw(null_nodes) }.unwrap_err(),
        SceneError::NullBufferPointer("nodes")
    );

    // Out-of-bounds text & invalid UTF-8 checks.
    let mut node = NveRenderNode::zeroed();
    node.node_id = 7;
    node.text_offset = 0;
    node.text_len = 2;
    let bad_utf8: [u8; 2] = [0xFF, 0xFE];
    let valid_view = NveSceneView {
        schema_hash: NVE_SCENE_SCHEMA_HASH,
        frame_id: 1,
        node_count: 1,
        dirty_count: 0,
        removed_count: 0,
        text_slab_len: 2,
        children_slab_len: 0,
        _pad: 0,
        nodes: &node,
        text_slab: bad_utf8.as_ptr(),
        children_slab: std::ptr::null(),
        dirty_indices: std::ptr::null(),
        removed_ids: std::ptr::null(),
    };
    let reader = unsafe { NveSceneRef::from_raw(valid_view) }.unwrap();
    assert_eq!(
        reader.node_text(&reader.nodes()[0]).unwrap_err(),
        SceneError::InvalidUtf8 { node_id: 7 }
    );

    node.text_len = 10;
    let oob_view = NveSceneView {
        nodes: &node,
        ..valid_view
    };
    let oob_reader = unsafe { NveSceneRef::from_raw(oob_view) }.unwrap();
    assert_eq!(
        oob_reader.node_text(&oob_reader.nodes()[0]).unwrap_err(),
        SceneError::TextOutOfBounds {
            node_id: 7,
            offset: 0,
            len: 10,
            slab_len: 2,
        }
    );
}

#[test]
fn checked_in_c_header_matches_compile_time_schema_hash() {
    let header_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h");
    let content = std::fs::read_to_string(&header_path)
        .expect("read generated cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h");
    let expected_macro = format!("0x{NVE_SCENE_SCHEMA_HASH:016X}ULL");
    assert!(
        content.contains(&expected_macro),
        "nve_scene_abi.generated.h schema hash drift! Expected {expected_macro} in {}",
        header_path.display()
    );
}
