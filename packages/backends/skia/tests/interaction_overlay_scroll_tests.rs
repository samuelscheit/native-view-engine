//! Skia integration tests for:
//! 1. `requested_scroll_seq` 0-alloc `CompositeOnly` lane + raster pixel reflection.
//! 2. `set_focused_node_with_modality` (`:focus` vs `:focus-visible`).
//! 3. Top-layer `Overlay` escaping `overflow: hidden` ancestors + modal scrim & hit-blocking.

use nve_abi::{ColorRgba, EventSet, InteractionEvent, SurfaceId};
use nve_backend_skia::{
    FocusModality, InteractionState, PixelBuffer, SceneDirtyKind, SkiaBackend, StateStyleRule,
};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, RowPatch};
use nve_tables::tables::{
    DebugId, FlexDirection, Interaction, Layout, Overflow, Overlay, OverlayFlags, OverlayLayer,
    OverlayPlacement, Position, Scroll, StateRules, Transform, Visual, register_core_tables,
};

#[test]
fn skia_requested_scroll_seq_takes_composite_only_lane_and_updates_pixels() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let scroller = {
        let mut tx = store.tx();
        let sc = tx.create();
        tx.edit::<Layout>(sc)
            .flex_direction(FlexDirection::Column)
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Scroll>(sc);

        let top_block = tx.create();
        tx.edit::<Layout>(top_block)
            .flex_shrink(0.0)
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Visual>(top_block).background(Rgba::rgb(220, 38, 38)); // red

        let bottom_block = tx.create();
        tx.edit::<Layout>(bottom_block)
            .flex_shrink(0.0)
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Visual>(bottom_block).background(Rgba::rgb(34, 197, 94)); // green

        tx.set_children(sc, &[top_block, bottom_block]);
        tx.set_children(NodeId::ROOT, &[sc]);
        tx.commit_into(&mut changes);
        sc
    };

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 100.0,
            height: 100.0,
            scale_factor: 1.0,
        },
    );
    skia.commit(&store, &changes).unwrap();

    let mut px = PixelBuffer::new(100, 100);
    skia.rasterize(&mut px, ColorRgba::BLACK);
    assert_eq!(px.get_rgb(50, 50), Some((220, 38, 38)));

    // Commit controlled jump to y=100 with seq=1 -> MUST stay on CompositeOnly lane (0 chunks emitted)!
    let chunks_before = skia.scene().stats().chunks_emitted;
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(100.0))
            .requested_scroll_seq(1);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly)
    );
    assert_eq!(skia.scene().stats().chunks_emitted, chunks_before);
    assert_eq!(skia.scroll_offset(scroller), (0.0, 100.0));

    skia.rasterize(&mut px, ColorRgba::BLACK);
    assert_eq!(px.get_rgb(50, 50), Some((34, 197, 94)));
}

#[test]
fn skia_focus_modality_drives_focus_visible_paint_and_top_layer_escapes_clip_with_modal_scrim() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (focus_btn, overlay_node, overlay_btn) = {
        let mut tx = store.tx();

        let mut fv_patch = RowPatch::new();
        fv_patch.set(Visual::BACKGROUND, Rgba::rgb(250, 204, 21)); // yellow on :focus-visible

        let focus_btn = tx.create();
        tx.edit::<Layout>(focus_btn)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(60.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(focus_btn).background(Rgba::rgb(37, 99, 235)); // blue base
        tx.edit::<Interaction>(focus_btn)
            .focusable(true)
            .listeners(EventSet::one(InteractionEvent::Press));
        tx.set_resource(
            focus_btn,
            Interaction::STATE_RULES,
            Some(StateRules(
                vec![StateStyleRule::new(
                    InteractionState::FOCUS_VISIBLE,
                    fv_patch,
                )]
                .into(),
            )),
        );

        // Tiny overflow:hidden container holding an anchored top-layer overlay!
        let clipper = tx.create();
        tx.edit::<Layout>(clipper)
            .position(Position::Absolute)
            .inset_left(Length::points(100.0))
            .inset_top(Length::points(20.0))
            .width(Length::points(40.0))
            .height(Length::points(20.0));
        tx.edit::<Visual>(clipper)
            .overflow_x(Overflow::Hidden)
            .overflow_y(Overflow::Hidden);
        tx.edit::<DebugId>(clipper).id("anchor-box");

        let ov = tx.create();
        tx.edit::<Layout>(ov)
            .position(Position::Fixed)
            .width(Length::points(100.0))
            .height(Length::points(60.0));
        tx.edit::<Visual>(ov).background(Rgba::rgb(16, 185, 129)); // emerald
        tx.edit::<Overlay>(ov)
            .layer(OverlayLayer::Popover)
            .anchor_id("anchor-box")
            .placement(OverlayPlacement::BottomStart)
            .offset_px(8.0)
            .options(OverlayFlags::OPEN | OverlayFlags::FLIP | OverlayFlags::SHIFT);

        let ov_btn = tx.create();
        tx.edit::<Layout>(ov_btn)
            .width(Length::points(100.0))
            .height(Length::points(60.0));
        tx.edit::<Interaction>(ov_btn).listeners(EventSet::one(InteractionEvent::Press));

        tx.set_children(ov, &[ov_btn]);
        tx.set_children(clipper, &[ov]);
        tx.set_children(NodeId::ROOT, &[focus_btn, clipper]);
        tx.commit_into(&mut changes);
        (focus_btn, ov, ov_btn)
    };

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    skia.commit(&store, &changes).unwrap();

    // 1. Focus modality: Pointer -> no yellow :focus-visible; Keyboard -> yellow :focus-visible!
    skia.set_focused_node_with_modality(&store, Some(focus_btn), FocusModality::Pointer);
    assert_eq!(skia.interaction_state(focus_btn), InteractionState::FOCUS);
    let mut px = PixelBuffer::new(300, 200);
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(30, 25), Some((37, 99, 235)));

    skia.set_focused_node_with_modality(&store, Some(focus_btn), FocusModality::Keyboard);
    assert_eq!(
        skia.interaction_state(focus_btn),
        InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE
    );
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(30, 25), Some((250, 204, 21)));

    // 2. Top-layer overlay paints at (100, 20+20+8 = 48..108) completely outside clipper (20..40)!
    assert_eq!(px.get_rgb(150, 75), Some((16, 185, 129)));
    assert_eq!(skia.hit_test(&store, 150.0, 75.0), Some(overlay_btn));
    assert_eq!(skia.hit_test(&store, 30.0, 25.0), Some(focus_btn));

    // 3. Switch overlay to Modal -> background white pixel at (250, 180) is darkened by scrim AND
    //    background button hits are blocked!
    let bg_before = px.get_rgb(250, 180).unwrap();
    assert_eq!(bg_before, (255, 255, 255));
    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(overlay_node).layer(OverlayLayer::Modal);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    skia.rasterize(&mut px, ColorRgba::WHITE);
    let bg_after = px.get_rgb(250, 180).unwrap();
    assert!(
        bg_after.0 < 200 && bg_after.1 < 200 && bg_after.2 < 200,
        "modal backdrop scrim must darken background viewport pixels, got {bg_after:?}"
    );
    assert_eq!(px.get_rgb(150, 75), Some((16, 185, 129)));
    assert_eq!(skia.hit_test(&store, 30.0, 25.0), None);
}

#[test]
fn skia_focus_drops_on_overlay_close_and_disable_and_rejects_closed_overlay_focus() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (focus_btn, ov_node, ov_btn) = {
        let mut tx = store.tx();
        let mut fv_patch = RowPatch::new();
        fv_patch.set(Visual::BACKGROUND, Rgba::rgb(250, 204, 21));

        let focus_btn = tx.create();
        tx.edit::<Layout>(focus_btn)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(60.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(focus_btn).background(Rgba::rgb(37, 99, 235));
        tx.edit::<Interaction>(focus_btn).focusable(true);
        tx.set_resource(
            focus_btn,
            Interaction::STATE_RULES,
            Some(StateRules(
                vec![StateStyleRule::new(
                    InteractionState::FOCUS_VISIBLE,
                    fv_patch.clone(),
                )]
                .into(),
            )),
        );

        let ov = tx.create();
        tx.edit::<Layout>(ov)
            .position(Position::Fixed)
            .inset_left(Length::points(100.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Overlay>(ov)
            .layer(OverlayLayer::Popover)
            .options(OverlayFlags::NONE);

        let ov_btn = tx.create();
        tx.edit::<Layout>(ov_btn)
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(ov_btn).background(Rgba::rgb(37, 99, 235));
        tx.edit::<Interaction>(ov_btn).focusable(true);
        tx.set_resource(
            ov_btn,
            Interaction::STATE_RULES,
            Some(StateRules(
                vec![StateStyleRule::new(
                    InteractionState::FOCUS_VISIBLE,
                    fv_patch,
                )]
                .into(),
            )),
        );

        tx.set_children(ov, &[ov_btn]);
        tx.set_children(NodeId::ROOT, &[focus_btn, ov]);
        tx.commit_into(&mut changes);
        (focus_btn, ov, ov_btn)
    };

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 240.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );
    skia.commit(&store, &changes).unwrap();

    // 1. Focusing a button inside a closed overlay MUST be rejected.
    skia.set_focused_node_with_modality(&store, Some(ov_btn), FocusModality::Keyboard);
    assert_eq!(skia.focused_node(), None);
    assert_eq!(skia.interaction_state(ov_btn), InteractionState::NONE);

    // 2. Open overlay -> focus succeeds; close overlay -> commit MUST drop focus cleanly!
    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(ov_node).options(OverlayFlags::OPEN);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    skia.set_focused_node_with_modality(&store, Some(ov_btn), FocusModality::Keyboard);
    assert_eq!(skia.focused_node(), Some(ov_btn));
    assert!(skia.interaction_state(ov_btn).contains(InteractionState::FOCUS_VISIBLE));

    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(ov_node).options(OverlayFlags::NONE);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    assert_eq!(skia.focused_node(), None);
    assert_eq!(skia.interaction_state(ov_btn), InteractionState::NONE);

    // 3. Disabling a live focused button MUST drop focus AND repaint away :focus-visible yellow!
    skia.set_focused_node_with_modality(&store, Some(focus_btn), FocusModality::Keyboard);
    let mut px = PixelBuffer::new(240, 120);
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(30, 25), Some((250, 204, 21)));

    {
        let mut tx = store.tx();
        tx.edit::<Interaction>(focus_btn).disabled(true);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    assert_eq!(skia.focused_node(), None);
    assert_eq!(skia.interaction_state(focus_btn), InteractionState::DISABLED);
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(30, 25), Some((37, 99, 235)));
}

#[test]
fn skia_composite_only_transform_moves_anchored_overlay_and_reemits_on_auto_size() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (anchor, ov) = {
        let mut tx = store.tx();
        let anchor = tx.create();
        tx.edit::<Layout>(anchor)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(30.0))
            .height(Length::points(20.0));
        tx.edit::<DebugId>(anchor).id("moving-anchor");

        let ov = tx.create();
        tx.edit::<Layout>(ov)
            .position(Position::Fixed)
            .width(Length::points(80.0))
            .height(Length::points(50.0));
        tx.edit::<Visual>(ov).background(Rgba::rgb(16, 185, 129));
        tx.edit::<Overlay>(ov)
            .layer(OverlayLayer::Popover)
            .anchor_id("moving-anchor")
            .placement(OverlayPlacement::BottomStart)
            .offset_px(4.0)
            .viewport_padding_px(8.0)
            .options(OverlayFlags::OPEN | OverlayFlags::AUTO_SIZE);

        tx.set_children(NodeId::ROOT, &[anchor, ov]);
        tx.commit_into(&mut changes);
        (anchor, ov)
    };

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 120.0,
            scale_factor: 1.0,
        },
    );
    skia.commit(&store, &changes).unwrap();

    // Initial: anchor at (10,10,30,20) -> overlay at (10, 34), size (80, 50).
    let mut px = PixelBuffer::new(200, 120);
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(50, 55), Some((16, 185, 129)));
    assert_eq!(px.get_rgb(130, 80), Some((255, 255, 255)));

    // CompositeOnly Transform moves anchor to (80, 50, 30, 20):
    // Available height below (70 + 4 = 74) within 120 - 8 = 112 is only 38px (< 50px)!
    // AUTO_SIZE must shrink overlay height to 38px and re-emit its chunk so (100, 95) is emerald
    // while (100, 115) stays white!
    {
        let mut tx = store.tx();
        tx.edit::<Transform>(anchor).translate_x(70.0).translate_y(40.0);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).unwrap();
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly)
    );
    skia.rasterize(&mut px, ColorRgba::WHITE);
    assert_eq!(px.get_rgb(50, 55), Some((255, 255, 255)));
    assert_eq!(px.get_rgb(100, 95), Some((16, 185, 129)));
    assert_eq!(px.get_rgb(100, 115), Some((255, 255, 255)));
    let _ = ov;
}
