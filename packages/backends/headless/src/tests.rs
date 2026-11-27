use nve_abi::{ColorRgba, FrameId, InteractionEvent, PrimarySlot, SurfaceId};
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
        SurfaceId::ROOT,
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
    tx.edit::<Interaction>(btn).listen(InteractionEvent::Press);
    tx.commit_into(&mut changes);

    let stats = backend.commit(&store, &changes).unwrap();
    assert!(stats.mutation_count >= 4);
    assert!(backend
        .last_mutations()
        .iter()
        .any(|m| matches!(m, HeadlessMutation::Create(id, slot) if *id == title && *slot == TextContent::slot())));

    let btn_rect = backend.layout_of(btn).unwrap();
    assert_eq!((btn_rect.x, btn_rect.y, btn_rect.width, btn_rect.height), (0.0, 60.0, 400.0, 48.0));
    assert_eq!(backend.hit_test(&store, 100.0, 80.0), Some(btn));
    assert_eq!(backend.hit_test(&store, 100.0, 20.0), None);

    // Live NveSceneView publishes primary_slot + NveTableView slices.
    let view = backend.publish_scene_view(&store, &changes, FrameId(1));
    assert_eq!(view.node_count, 4);
    assert!(view.table_count >= 2);
    // SAFETY: `view` borrows `backend`'s published scene table with `node_count` valid nodes.
    let nodes = unsafe { std::slice::from_raw_parts(view.nodes, view.node_count as usize) };
    let root_node = nodes.iter().find(|n| n.node_id == NodeId::ROOT).unwrap();
    assert_eq!(root_node.primary_slot, PrimarySlot::NONE);
    let title_node = nodes.iter().find(|n| n.node_id == title).unwrap();
    assert_eq!(title_node.primary_slot, PrimarySlot(TextContent::slot().0));

    // Frame-0 NVERELOC export round-trips cleanly.
    let reloc = backend
        .export_scene_reloc(&store, 1, FrameId(1), "headless-test", ColorRgba::BLACK)
        .unwrap();
    let parsed = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    assert_eq!(parsed.nodes().len(), 4);
    let reloc_title = parsed.find_node_by_id(title).unwrap().1;
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
        SurfaceId::ROOT,
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
    let view = backend.publish_scene_view(&store, &changes, FrameId(1));

    // SAFETY: `view` borrows `backend`'s live scene table for the lifetime of `scene`.
    let scene = unsafe { nve_abi::NveSceneRef::from_raw(view) }.expect("valid NveSceneRef");
    let rn = scene
        .nodes()
        .iter()
        .find(|n| n.node_id == child)
        .expect("child render node");
    let rn_text = std::str::from_utf8(
        &scene.text_slab()[rn.text_offset as usize..(rn.text_offset as usize + rn.text_len.as_usize())],
    )
    .expect("render node text");
    assert_eq!(rn_text, "Unified Slab ✓");

    // Verify `TextContent` NveTableView's raw StrSpan words decode identically against `scene.text_slab()`!
    let tv = scene
        .tables()
        .iter()
        .find(|t| t.key_lo == TextContent::KEY.lo() && t.key_hi == TextContent::KEY.hi())
        .expect("TextContent table view");
    // SAFETY: `tv` points into `store`'s live dense table buffers while `store` is immutably borrowed.
    let (rows, owners) = unsafe {
        (
            std::slice::from_raw_parts(tv.rows, (tv.row_count * tv.words) as usize),
            std::slice::from_raw_parts(tv.owners, tv.row_count as usize),
        )
    };
    let row_idx = owners
        .iter()
        .position(|&o| o == child)
        .expect("child owner in TextContent");
    let off = rows[row_idx * tv.words as usize] as usize;
    let len = rows[row_idx * tv.words as usize + 1] as usize;
    let tv_text = std::str::from_utf8(&scene.text_slab()[off..off + len]).unwrap();
    assert_eq!(tv_text, "Unified Slab ✓");
    assert_eq!(rn.text_offset as usize, off);
    assert_eq!(rn.text_len.as_usize(), len);
}

#[test]
fn visibility_hidden_blocks_hit_test_while_opacity_zero_remains_hittable_and_fixed_uses_viewport() {
    use nve_components::tables::{BoxShadow, BoxShadows, Position, Visibility};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
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
        tx.edit::<Interaction>(z).listen(InteractionEvent::Press);

        // visibility: hidden is NOT hit-testable.
        tx.edit::<Layout>(h)
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(h).visibility(Visibility::Hidden);
        tx.edit::<Interaction>(h).listen(InteractionEvent::Press);

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
    assert_eq!(backend.hit_test(&store, 70.0, 110.0), Some(zero_op));
    // visibility: hidden box at (50, 140, 80x40) does NOT hit.
    assert_eq!(backend.hit_test(&store, 70.0, 150.0), None);
    // position: fixed box is anchored to viewport (20, 10), NOT parent (50+20, 100+10).
    let fr = backend.layout_of(fixed_box).unwrap();
    assert_eq!((fr.x, fr.y, fr.width, fr.height), (20.0, 10.0, 60.0, 30.0));

    // NVERELOC export emits real CSS box-shadow string (not empty "").
    let reloc = backend
        .export_scene_reloc(&store, 1, FrameId(1), "shadow-test", ColorRgba::BLACK)
        .unwrap();
    let parsed = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    let reloc_p = parsed.find_node_by_id(parent).unwrap().1;
    let shadow_css = parsed.box_shadow_of(reloc_p).unwrap();
    assert!(shadow_css.contains("2px 4px 8px"), "unexpected box_shadow: {shadow_css}");

    // Stale generation query after removal returns None (no cross-generation rect aliasing).
    let mut tx = store.tx();
    tx.remove(zero_op);
    tx.commit_into(&mut changes);
    backend.commit(&store, &changes).unwrap();
    assert_eq!(backend.layout_of(zero_op), None);
}

#[test]
fn publish_scene_view_marks_in_place_text_edits_dirty_and_skips_external_slab_compaction() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport { width: 320.0, height: 240.0, scale_factor: 1.0 },
    );

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
    let view1 = backend.publish_scene_view(&store, &changes, FrameId(1));
    // SAFETY: `view1` borrows live `backend` and `store` slabs for this test assertion.
    let ref1 = unsafe { nve_abi::NveSceneRef::from_raw(view1) }.unwrap();
    let idx1 = backend.scene_table.index_of(label).unwrap();
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
    let view2 = backend.publish_scene_view(&store, &changes, FrameId(2));
    // SAFETY: `view2` borrows live `backend` and `store` slabs for this test assertion.
    let ref2 = unsafe { nve_abi::NveSceneRef::from_raw(view2) }.unwrap();
    let idx2 = backend.scene_table.index_of(label).unwrap();
    assert_eq!(ref2.nodes()[idx2].text_offset, off1);
    assert_eq!(ref2.nodes()[idx2].text_len, len1);
    assert!(
        ref2.dirty_indices().contains(&(idx2 as u32)),
        "in-place same-class text edit must mark node dirty in NveSceneView"
    );
    assert_eq!(ref2.node_text(&ref2.nodes()[idx2]).unwrap(), "11");

    // Calling compact_text_slab() on an external-text NveSceneTable must be a safe no-op.
    backend.scene_table.compact_text_slab();
    let view3 = backend.publish_scene_view(&store, &ChangeSet::default(), FrameId(3));
    // SAFETY: `view3` borrows live `backend` and `store` slabs for this test assertion.
    let ref3 = unsafe { nve_abi::NveSceneRef::from_raw(view3) }.unwrap();
    assert_eq!(ref3.node_text(&ref3.nodes()[idx2]).unwrap(), "11");
}

#[test]
fn hit_test_recurses_into_visible_child_under_visibility_hidden_parent() {
    use nve_components::tables::Visibility;
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport { width: 300.0, height: 300.0, scale_factor: 1.0 },
    );

    let (hidden_parent, inherit_child, visible_child) = {
        let mut tx = store.tx();
        let p = tx.create();
        let inh = tx.create();
        let vis = tx.create();
        tx.set_children(NodeId::ROOT, &[p]);
        tx.set_children(p, &[inh, vis]);
        tx.edit::<Layout>(p).width(Length::points(200.0)).height(Length::points(200.0));
        tx.edit::<Visual>(p).visibility(Visibility::Hidden);
        tx.edit::<Interaction>(p).listen(InteractionEvent::Press);
        // Ordinary `Visibility::Inherit` child under Hidden parent inherits hiddenness.
        tx.edit::<Layout>(inh).width(Length::points(80.0)).height(Length::points(80.0));
        tx.edit::<Interaction>(inh).listen(InteractionEvent::Press);
        // Explicit `Visibility::Visible` child overrides Hidden parent (CSS parity).
        tx.edit::<Layout>(vis).width(Length::points(80.0)).height(Length::points(80.0));
        tx.edit::<Visual>(vis).visibility(Visibility::Visible);
        tx.edit::<Interaction>(vis).listen(InteractionEvent::Press);
        tx.commit_into(&mut changes);
        (p, inh, vis)
    };
    backend.commit(&store, &changes).unwrap();

    let inh_r = backend.layout_of(inherit_child).unwrap();
    let vis_r = backend.layout_of(visible_child).unwrap();
    // Inherit child under Hidden parent: NOT hit-testable.
    assert_eq!(backend.hit_test(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    assert_eq!(backend.hit_test_interaction(&store, inh_r.x + 10.0, inh_r.y + 10.0), None);
    // Explicit Visible child under Hidden parent: IS hit-testable.
    assert_eq!(backend.hit_test(&store, vis_r.x + 10.0, vis_r.y + 10.0), Some(visible_child));
    assert_eq!(
        backend.hit_test_interaction(&store, vis_r.x + 10.0, vis_r.y + 10.0),
        Some(visible_child)
    );
    // Inside hidden_parent outside visible_child: must NOT hit hidden_parent.
    assert_eq!(backend.hit_test(&store, 190.0, 190.0), None);
    let _ = hidden_parent;
}

#[test]
fn hidden_text_input_collapses_to_zero_layout_no_hit_and_auto_cursor() {
    use nve_components::interaction::CursorKind;
    use nve_components::tables::{InputKind, TextInput};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 320.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let (hidden_bare, hidden_sized, sibling) = {
        let mut tx = store.tx();
        let bare = tx.create();
        let sized = tx.create();
        let sib = tx.create();
        tx.set_children(NodeId::ROOT, &[bare, sized, sib]);

        tx.edit::<TextInput>(bare)
            .input_kind(InputKind::Hidden)
            .value("secret");
        tx.edit::<TextInput>(sized)
            .input_kind(InputKind::Hidden)
            .value("secret-sized");
        tx.edit::<Layout>(sized)
            .width(Length::points(200.0))
            .height(Length::points(40.0));
        tx.edit::<Interaction>(sized)
            .listen(InteractionEvent::Press)
            .listen(InteractionEvent::Focus)
            .cursor(CursorKind::Pointer);
        tx.edit::<Layout>(sib)
            .width(Length::points(120.0))
            .height(Length::points(30.0));
        tx.edit::<Interaction>(sib)
            .listen(InteractionEvent::Press);
        tx.commit_into(&mut changes);
        (bare, sized, sib)
    };
    backend.commit(&store, &changes).unwrap();

    for id in [hidden_bare, hidden_sized] {
        let r = backend.layout_of(id).expect("layout entry exists");
        assert_eq!((r.width, r.height), (0.0, 0.0), "InputKind::Hidden must lay out as 0x0");
        assert!(!backend.is_interaction_target(&store, id));
        assert_eq!(backend.effective_cursor(&store, id), CursorKind::Auto);
    }
    // Sibling starts at y=0 because both hidden inputs take 0 space in flow.
    let sib_r = backend.layout_of(sibling).expect("sibling layout");
    assert_eq!((sib_r.x, sib_r.y, sib_r.width, sib_r.height), (0.0, 0.0, 120.0, 30.0));
    assert_eq!(backend.hit_test(&store, 10.0, 10.0), Some(sibling));
    assert_eq!(backend.hit_test_interaction(&store, 10.0, 10.0), Some(sibling));

    // Toggling `InputKind::Hidden -> InputKind::Text` re-triggers layout via `#[effect(layout, paint, semantics)]`.
    {
        let mut tx = store.tx();
        tx.edit::<TextInput>(hidden_sized).input_kind(InputKind::Text);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).unwrap();
    let grown = backend.layout_of(hidden_sized).expect("unhidden layout");
    assert_eq!((grown.width, grown.height), (200.0, 40.0));
    assert!(backend.is_interaction_target(&store, hidden_sized));
    assert_eq!(backend.layout_of(sibling).unwrap().y, 40.0);
}
