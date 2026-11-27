use nve_abi::{ColorRgba, EventMask, NVE_PRIMARY_SLOT_NONE};
use nve_components::tables::{
    core_realization_order, FlexDirection, Interaction, Layout, TextContent, Typography, Visual,
};
use nve_engine::{SceneRelocView, StoreBackend, VerifyMode, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba};

use crate::{HeadlessBackend, HeadlessMutation};

#[test]
fn headless_commit_layout_hit_test_scene_view_and_reloc_export() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 2.0,
        },
    );

    let mut tx = store.tx();
    let header = tx.create();
    let title = tx.create();
    let btn = tx.create();
    tx.set_children(NodeId::ROOT, &[header, btn]);
    tx.set_children(header, &[title]);

    tx.edit::<Layout>(header)
        .height(Length::points(60.0))
        .padding_left(Length::points(16.0))
        .padding_top(Length::points(12.0));
    tx.edit::<Visual>(header).background(Rgba::rgb(15, 23, 42));
    tx.edit::<TextContent>(title).text("Dashboard");
    tx.edit::<Typography>(title)
        .color(Rgba::WHITE)
        .font_size(20.0)
        .font_weight(700);
    tx.edit::<Layout>(btn)
        .height(Length::points(48.0))
        .flex_direction(FlexDirection::Row);
    tx.edit::<Interaction>(btn).event_mask(EventMask::PRESS as u32);
    tx.commit_into(&mut changes);

    let stats = backend.commit(&store, &changes).unwrap();
    assert!(stats.mutation_count >= 4);
    assert!(backend
        .last_mutations()
        .iter()
        .any(|m| matches!(m, HeadlessMutation::Create(id, slot) if *id == title.raw() && *slot == TextContent::slot())));

    let btn_rect = backend.layout_of(btn.raw()).unwrap();
    assert_eq!((btn_rect.x, btn_rect.y, btn_rect.width, btn_rect.height), (0.0, 60.0, 400.0, 48.0));
    assert_eq!(backend.hit_test(&store, 100.0, 80.0), Some(btn.raw()));
    assert_eq!(backend.hit_test(&store, 100.0, 20.0), None);

    // Live NveSceneView publishes primary_slot + NveTableView slices.
    let view = backend.publish_scene_view(&store, &changes, 1);
    assert_eq!(view.node_count, 4);
    assert!(view.table_count >= 2);
    let nodes = unsafe { std::slice::from_raw_parts(view.nodes, view.node_count as usize) };
    let root_node = nodes.iter().find(|n| n.node_id == NodeId::ROOT.raw()).unwrap();
    assert_eq!(root_node.primary_slot, NVE_PRIMARY_SLOT_NONE);
    let title_node = nodes.iter().find(|n| n.node_id == title.raw()).unwrap();
    assert_eq!(title_node.primary_slot, TextContent::slot().0);

    // Frame-0 NVERELOC export round-trips cleanly.
    let reloc = backend
        .export_scene_reloc(&store, 1, 1, "headless-test", ColorRgba::BLACK)
        .unwrap();
    let parsed = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    assert_eq!(parsed.nodes().len(), 4);
    let reloc_title = parsed.find_node_by_id(title.raw()).unwrap().1;
    assert_eq!(parsed.text_of(reloc_title).unwrap(), "Dashboard");
    assert_eq!(reloc_title.primary_slot(), TextContent::slot().0);

    let dump = backend.dump_tree(&store);
    assert!(dump.contains("Dashboard"));
}

#[test]
fn publish_scene_view_unifies_text_slab_across_render_nodes_and_table_views() {
    let mut store = NodeStore::new();
    store.set_realization_order(core_realization_order());
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        },
    );

    let child = {
        let mut tx = store.tx();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[c]);
        tx.edit::<TextContent>(c).text("Unified Slab ✓");
        tx.commit_into(&mut changes);
        c
    };
    backend.commit(&store, &changes).unwrap();
    let view = backend.publish_scene_view(&store, &changes, 1);

    let scene = unsafe { nve_abi::NveSceneRef::from_raw(view) }.expect("valid NveSceneRef");
    let rn = scene
        .nodes()
        .iter()
        .find(|n| n.node_id == child.raw())
        .expect("child render node");
    let rn_text = std::str::from_utf8(
        &scene.text_slab()[rn.text_offset as usize..(rn.text_offset as usize + rn.text_len as usize)],
    )
    .expect("render node text");
    assert_eq!(rn_text, "Unified Slab ✓");

    // Verify `TextContent` NveTableView's raw StrSpan words decode identically against `scene.text_slab()`!
    let tv = scene
        .tables()
        .iter()
        .find(|t| t.key_lo == TextContent::KEY.lo() && t.key_hi == TextContent::KEY.hi())
        .expect("TextContent table view");
    let rows = unsafe { std::slice::from_raw_parts(tv.rows, (tv.row_count * tv.words) as usize) };
    let owners = unsafe { std::slice::from_raw_parts(tv.owners, tv.row_count as usize) };
    let row_idx = owners
        .iter()
        .position(|&o| o == child.raw())
        .expect("child owner in TextContent");
    let off = rows[row_idx * tv.words as usize] as usize;
    let len = rows[row_idx * tv.words as usize + 1] as usize;
    let tv_text = std::str::from_utf8(&scene.text_slab()[off..off + len]).unwrap();
    assert_eq!(tv_text, "Unified Slab ✓");
    assert_eq!(rn.text_offset as usize, off);
    assert_eq!(rn.text_len as usize, len);
}

#[test]
fn visibility_hidden_blocks_hit_test_while_opacity_zero_remains_hittable_and_fixed_uses_viewport() {
    use nve_components::tables::{BoxShadow, BoxShadows, Position, Visibility};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 500.0,
            height: 400.0,
            scale_factor: 1.0,
        },
    );

    let (parent, zero_op, _hidden_vis, fixed_box) = {
        let mut tx = store.tx();
        let p = tx.create();
        let z = tx.create();
        let h = tx.create();
        let f = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[z, h, f]);

        tx.edit::<Layout>(p)
            .margin_top(Length::points(100.0))
            .margin_left(Length::points(50.0))
            .width(Length::points(200.0))
            .height(Length::points(200.0));
        tx.edit::<Visual>(p).box_shadows(Some(BoxShadows::new([BoxShadow {
            offset_x: 2.0,
            offset_y: 4.0,
            blur_radius: 8.0,
            spread_radius: 0.0,
            color: Rgba::new(0, 0, 0, 128),
            inset: false,
        }])));

        // opacity == 0.0 is still hit-testable in CSS.
        tx.edit::<Layout>(z)
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(z).opacity(0.0);
        tx.edit::<Interaction>(z).event_mask(EventMask::PRESS as u32);

        // visibility: hidden is NOT hit-testable.
        tx.edit::<Layout>(h)
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(h).visibility(Visibility::Hidden);
        tx.edit::<Interaction>(h).event_mask(EventMask::PRESS as u32);

        // position: fixed is positioned relative to root viewport (0, 0), NOT parent (50, 100).
        tx.edit::<Layout>(f)
            .position(Position::Fixed)
            .inset_top(Length::points(10.0))
            .inset_left(Length::points(20.0))
            .width(Length::points(60.0))
            .height(Length::points(30.0));

        tx.commit_into(&mut changes);
        (p, z, h, f)
    };

    backend.commit(&store, &changes).unwrap();

    // opacity: 0 box at (50, 100, 80x40) hits `zero_op`.
    assert_eq!(backend.hit_test(&store, 70.0, 110.0), Some(zero_op.raw()));
    // visibility: hidden box at (50, 140, 80x40) does NOT hit.
    assert_eq!(backend.hit_test(&store, 70.0, 150.0), None);
    // position: fixed box is anchored to viewport (20, 10), NOT parent (50+20, 100+10).
    let fr = backend.layout_of(fixed_box.raw()).unwrap();
    assert_eq!((fr.x, fr.y, fr.width, fr.height), (20.0, 10.0, 60.0, 30.0));

    // NVERELOC export emits real CSS box-shadow string (not empty "").
    let reloc = backend
        .export_scene_reloc(&store, 1, 1, "shadow-test", ColorRgba::BLACK)
        .unwrap();
    let parsed = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    let reloc_p = parsed.find_node_by_id(parent.raw()).unwrap().1;
    let shadow_css = parsed.box_shadow_of(reloc_p).unwrap();
    assert!(shadow_css.contains("2px 4px 8px"), "unexpected box_shadow: {shadow_css}");

    // Stale generation query after removal returns None (no cross-generation rect aliasing).
    let mut tx = store.tx();
    tx.remove(zero_op);
    tx.commit_into(&mut changes);
    backend.commit(&store, &changes).unwrap();
    assert_eq!(backend.layout_of(zero_op.raw()), None);
}

#[test]
fn publish_scene_view_marks_in_place_text_edits_dirty_and_skips_external_slab_compaction() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, Viewport { width: 320.0, height: 240.0, scale_factor: 1.0 });

    let label = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n).width(Length::points(120.0)).height(Length::points(30.0));
        tx.edit::<TextContent>(n).text("10");
        tx.commit_into(&mut changes);
        n
    };
    backend.commit(&store, &changes).unwrap();
    let view1 = backend.publish_scene_view(&store, &changes, 1);
    let ref1 = unsafe { nve_abi::NveSceneRef::from_raw(view1) }.unwrap();
    let idx1 = backend.scene_table.index_of(label.raw()).unwrap();
    assert_eq!(ref1.node_text(&ref1.nodes()[idx1]).unwrap(), "10");
    let off1 = ref1.nodes()[idx1].text_offset;
    let len1 = ref1.nodes()[idx1].text_len;

    // Overwrite "10" -> "11" in the same StrArena size class (identical offset & length).
    {
        let mut tx = store.tx();
        tx.edit::<TextContent>(label).text("11");
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).unwrap();
    let view2 = backend.publish_scene_view(&store, &changes, 2);
    let ref2 = unsafe { nve_abi::NveSceneRef::from_raw(view2) }.unwrap();
    let idx2 = backend.scene_table.index_of(label.raw()).unwrap();
    assert_eq!(ref2.nodes()[idx2].text_offset, off1);
    assert_eq!(ref2.nodes()[idx2].text_len, len1);
    assert!(
        ref2.dirty_indices().contains(&(idx2 as u32)),
        "in-place same-class text edit must mark node dirty in NveSceneView"
    );
    assert_eq!(ref2.node_text(&ref2.nodes()[idx2]).unwrap(), "11");

    // Calling compact_text_slab() on an external-text NveSceneTable must be a safe no-op.
    backend.scene_table.compact_text_slab();
    let view3 = backend.publish_scene_view(&store, &ChangeSet::default(), 3);
    let ref3 = unsafe { nve_abi::NveSceneRef::from_raw(view3) }.unwrap();
    assert_eq!(ref3.node_text(&ref3.nodes()[idx2]).unwrap(), "11");
}

#[test]
fn hit_test_recurses_into_visible_child_under_visibility_hidden_parent() {
    use nve_components::tables::Visibility;
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(1, Viewport { width: 300.0, height: 300.0, scale_factor: 1.0 });

    let (hidden_parent, inherit_child, visible_child) = {
        let mut tx = store.tx();
        let p = tx.create();
        let inh = tx.create();
        let vis = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[inh, vis]);
        tx.edit::<Layout>(p).width(Length::points(200.0)).height(Length::points(200.0));
        tx.edit::<Visual>(p).visibility(Visibility::Hidden);
        tx.edit::<Interaction>(p).event_mask(EventMask::PRESS as u32);
        // Ordinary `Visibility::Inherit` child under Hidden parent inherits hiddenness.
        tx.edit::<Layout>(inh).width(Length::points(80.0)).height(Length::points(80.0));
        tx.edit::<Interaction>(inh).event_mask(EventMask::PRESS as u32);
        // Explicit `Visibility::Visible` child overrides Hidden parent (CSS parity).
        tx.edit::<Layout>(vis).width(Length::points(80.0)).height(Length::points(80.0));
        tx.edit::<Visual>(vis).visibility(Visibility::Visible);
        tx.edit::<Interaction>(vis).event_mask(EventMask::PRESS as u32);
        tx.commit_into(&mut changes);
        (p, inh, vis)
    };
    backend.commit(&store, &changes).unwrap();

    let inh_r = backend.layout_of(inherit_child.raw()).unwrap();
    let vis_r = backend.layout_of(visible_child.raw()).unwrap();
    // Inherit child under Hidden parent: NOT hit-testable.
    assert_eq!(backend.hit_test(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_interaction(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    // Explicit Visible child under Hidden parent: IS hit-testable.
    assert_eq!(backend.hit_test(&store, vis_r.x + 10.0, vis_r.y + 10.0), Some(visible_child.raw()));
    assert_eq!(
        backend.hit_test_interaction(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child.raw())
    );
    // Inside hidden_parent outside visible_child: must NOT hit hidden_parent.
    assert_eq!(backend.hit_test(&store, 190.0, 190.0), None);
    let _ = hidden_parent;
}
