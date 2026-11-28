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
    OverlayPlacement, Position, Scroll, StateRules, Visual, register_core_tables,
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
