//! Inherited typography stop-pruning, chunk arena compaction memory/pixel parity, and
//! `scroll_into_view` (including `display: none` and virtualized `virtual_top_spacer`) tests.

use nve_abi::{ColorRgba, SurfaceId};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::tables::{
    BackgroundFillRes, Display, FlexDirection, Layout, Overflow, Scroll, TextContent, Typography,
    Visual,
};

use super::oracle_tests::assert_scene_matches_fresh_display_list;
use super::SceneDirtyKind;
use crate::SkiaBackend;

#[test]
fn inherited_typography_stop_pruning_skips_overriding_subtree() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let parent = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(300.0))
            .height(Length::points(200.0));
        let parent = tx.create();
        tx.edit::<Layout>(parent)
            .width(Length::points(300.0))
            .height(Length::points(200.0));
        tx.edit::<Typography>(parent).color(Rgba::rgb(200, 200, 200));

        let override_branch = tx.create();
        tx.edit::<Layout>(override_branch)
            .width(Length::points(150.0))
            .height(Length::points(200.0));
        tx.edit::<Typography>(override_branch).color(Rgba::rgb(255, 0, 0));
        let mut leaves = Vec::with_capacity(50);
        for _ in 0..50 {
            let leaf = tx.create();
            tx.edit::<Layout>(leaf)
                .width(Length::points(100.0))
                .height(Length::points(10.0));
            tx.edit::<TextContent>(leaf).text("Fixed Red");
            leaves.push(leaf);
        }
        tx.set_children(override_branch, &leaves);

        let inherit_leaf = tx.create();
        tx.edit::<Layout>(inherit_leaf)
            .width(Length::points(100.0))
            .height(Length::points(20.0));
        tx.edit::<TextContent>(inherit_leaf).text("Inherits");

        tx.set_children(parent, &[override_branch, inherit_leaf]);
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.commit_into(&mut changes);
        parent
    };
    backend.commit(&store, &changes).expect("initial commit");

    let before_emitted = backend.scene().stats().chunks_emitted;
    {
        let mut tx = store.tx();
        tx.edit::<Typography>(parent).color(Rgba::rgb(0, 255, 128));
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("color change");
    let emitted = backend.scene().stats().chunks_emitted - before_emitted;

    assert_eq!(
        backend.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly)
    );
    assert!(
        emitted <= 4,
        "stop-pruning must NOT re-emit the 50 descendants under overriding branch (emitted {emitted})"
    );
    assert_scene_matches_fresh_display_list(&mut backend, &store, 300, 200, "stop-pruning");
}

#[test]
fn chunk_arena_compaction_keeps_memory_bounded_and_preserves_pixel_parity() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 100.0,
            scale_factor: 1.0,
        },
    );

    let card = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(200.0))
            .height(Length::points(100.0))
            .flex_direction(FlexDirection::Column);
        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(180.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(card).background(Rgba::rgb(20, 40, 80));
        tx.edit::<TextContent>(card).text("Churn frame 0000000000000000");

        let grad_card = tx.create();
        tx.edit::<Layout>(grad_card)
            .width(Length::points(180.0))
            .height(Length::points(40.0));
        tx.edit::<Visual>(grad_card)
            .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
                45.0,
                vec![
                    GradientStop::new(0.0, ColorRgba::rgb(239, 68, 68)),
                    GradientStop::new(1.0, ColorRgba::rgb(59, 130, 246)),
                ],
            ))));
        tx.set_children(NodeId::ROOT, &[card, grad_card]);
        tx.commit_into(&mut changes);
        card
    };
    backend.commit(&store, &changes).expect("init");

    for frame in 0..600u32 {
        let mut tx = store.tx();
        tx.edit::<Visual>(card)
            .background(Rgba::rgb((frame & 255) as u8, 80, 120));
        tx.commit_into(&mut changes);
        backend.commit(&store, &changes).expect("churn frame");
    }

    assert!(
        backend.scene().arena_compactions() >= 1,
        "arena must compact dead ops under sustained paint churn"
    );
    assert!(
        backend.scene().arenas.ops.len() < 600,
        "compacted ops arena must stay bounded, got {}",
        backend.scene().arenas.ops.len()
    );
    assert_scene_matches_fresh_display_list(
        &mut backend,
        &store,
        200,
        100,
        "post-compaction pixel parity (including preserved gradient + text)",
    );
}

#[test]
fn scroll_into_view_handles_offscreen_display_none_and_virtual_top_spacer() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 100.0,
            scale_factor: 1.0,
        },
    );

    let (scroll, target, hidden_target) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        let scroll = tx.create();
        tx.edit::<Layout>(scroll)
            .width(Length::points(200.0))
            .height(Length::points(100.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(scroll);
        tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);
        let mut kids = Vec::new();
        for _ in 0..10 {
            let item = tx.create();
            tx.edit::<Layout>(item)
                .width(Length::points(200.0))
                .height(Length::points(40.0))
                .flex_shrink(0.0);
            kids.push(item);
        }
        let hidden = tx.create();
        tx.edit::<Layout>(hidden)
            .display(Display::None)
            .width(Length::points(200.0))
            .height(Length::points(40.0));
        kids.push(hidden);
        let target = kids[7]; // y = 280..320
        tx.set_children(scroll, &kids);
        tx.set_children(NodeId::ROOT, &[scroll]);
        tx.commit_into(&mut changes);
        (scroll, target, hidden)
    };
    backend.commit(&store, &changes).expect("init");

    // 1. display: none target must be rejected cleanly without scrolling.
    assert!(!backend.scroll_into_view(&store, hidden_target));
    assert_eq!(backend.scroll_offset(scroll), (0.0, 0.0));

    // 2. Normal offscreen child scrolls minimally to bring y=280..320 into 0..100.
    assert!(backend.scroll_into_view(&store, target));
    let (_, sy) = backend.scroll_offset(scroll);
    assert!(
        (sy - 220.0).abs() <= 1.0,
        "expected minimal scroll y ~= 220, got {sy}"
    );
    assert!(!backend.scroll_into_view(&store, target));

    // 3. Virtualized container with `virtual_top_spacer = Some(1000.0)` starting at raw sy = 500.0.
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroll)
            .virtual_top_spacer(Some(1000.0))
            .virtual_total_height(Some(2000.0));
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).expect("virtual scroll");
    backend.set_scroll_offset(&store, scroll, 0.0, 500.0);
    assert!(backend.scroll_into_view(&store, target));
    let (_, vsy) = backend.scroll_offset(scroll);
    assert!(
        (vsy - 1220.0).abs() <= 1.0,
        "expected virtualized raw scroll y ~= 1220 (spacer 1000 + shift 220), got {vsy}"
    );
    assert!(!backend.scroll_into_view(&store, target));
}
