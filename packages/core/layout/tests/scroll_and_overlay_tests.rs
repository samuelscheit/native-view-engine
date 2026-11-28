//! `CountingAlloc` zero-allocation guards plus exact falsifying tests for:
//! 1. Controlled scroll sync (`requested_scroll_seq`) including repeat Jump-to-0 and user-scroll preservation.
//! 2. Zero-alloc `compute_overlay_position` across all 12 placements, flips, shifts, auto-size, and fallbacks.
//! 3. Top-layer `reconcile_overlays`, clipped-ancestor escape, modal backdrop hit-blocking, and
//!    ancestor `CursorKind` inheritance (including disabled `Pointer` -> `NotAllowed`).

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;

use nve_abi::{EventSet, InteractionEvent};
use nve_engine::Viewport;
use nve_layout::overlay::{
    Placement, PositionOptions, Rect, Size, Viewport as OverlayViewport, compute_overlay_position,
};
use nve_layout::{LaidOutScene, TaffyStoreLayout, is_subtree_closed_overlay};
use nve_store::{ChangeSet, Length, NodeId, NodeStore};
use nve_tables::interaction::CursorKind;
use nve_tables::tables::{
    DebugId, FlexDirection, Interaction, Layout, Overflow, Overlay, OverlayFlags, OverlayLayer,
    OverlayPlacement, Position, Scroll, Transform, Visual, register_core_tables,
};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get().wrapping_add(1)));
}

// SAFETY: Delegates verbatim to `std::alloc::System` while bumping a thread-local counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    (out, THREAD_ALLOCS.with(Cell::get).wrapping_sub(before))
}

#[test]
fn requested_scroll_seq_supports_repeat_jump_to_zero_preserves_user_scroll_and_is_zero_alloc() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let scroller = {
        let mut tx = store.tx();
        let sc = tx.create();
        let child = tx.create();
        tx.edit::<Layout>(sc)
            .flex_direction(FlexDirection::Column)
            .width(Length::points(200.0))
            .height(Length::points(100.0));
        tx.edit::<Scroll>(sc).requested_scroll_y(Some(120.0)); // seq == 0!
        tx.edit::<Layout>(child)
            .flex_shrink(0.0)
            .width(Length::points(200.0))
            .height(Length::points(600.0));
        tx.set_children(sc, &[child]);
        tx.set_children(NodeId::ROOT, &[sc]);
        tx.commit_into(&mut changes);
        sc
    };

    let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
    scene.set_viewport(Viewport {
        width: 300.0,
        height: 300.0,
        scale_factor: 1.0,
    });
    scene.commit(&store, &changes);

    // 1. seq == 0 must NEVER force a jump even when requested_scroll_y is Some(120.0).
    assert!(scene.last_reconciled_scroll_nodes().is_empty());
    assert!(scene.reconcile_scroll_offsets(&store).is_empty());
    assert_eq!(scene.scroll_offset(scroller), (0.0, 0.0));

    // 2. seq == 1 jumps to 120.0 (clamped within [0, 500]).
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(120.0))
            .requested_scroll_seq(1);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    assert_eq!(scene.last_reconciled_scroll_nodes(), &[scroller]);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 120.0));
    assert!(scene.reconcile_scroll_offsets(&store).is_empty());

    // 3. User scrolls to 240.0; re-reconciling with unchanged seq == 1 must NOT clobber user scroll.
    scene.set_scroll_offset_clamped(&store, scroller, 0.0, 240.0);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 240.0));
    assert!(scene.reconcile_scroll_offsets(&store).is_empty());
    assert_eq!(scene.scroll_offset(scroller), (0.0, 240.0));

    // 4. Repeat Jump-to-0 falsification: jump to 0 with seq=2, user scrolls to 260, jump to 0 AGAIN
    //    with the EXACT SAME float value 0.0 and seq=3 -> MUST jump back to 0.0!
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(0.0))
            .requested_scroll_seq(2);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 0.0));

    scene.set_scroll_offset_clamped(&store, scroller, 0.0, 260.0);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 260.0));
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(0.0))
            .requested_scroll_seq(3);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 0.0));

    // 5. Positive probe proving `CountingAlloc` is active + steady-state zero-alloc guard.
    let (probe_box, probe_allocs) = count_allocs(|| std::hint::black_box(Box::new(42u64)));
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, saw {probe_allocs}"
    );
    drop(probe_box);

    let (_, allocs) = count_allocs(|| {
        for _ in 0..100 {
            let _ = scene.reconcile_scroll_offsets(&store);
        }
    });
    assert_eq!(
        allocs, 0,
        "steady-state reconcile_scroll_offsets must perform 0 heap allocations, saw {allocs}"
    );
}

#[test]
fn scroll_seq_handles_wrap_reset_single_axis_and_recycled_generation() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let scroller = {
        let mut tx = store.tx();
        let sc = tx.create();
        let child = tx.create();
        tx.edit::<Layout>(sc)
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Scroll>(sc);
        tx.edit::<Layout>(child)
            .flex_shrink(0.0)
            .width(Length::points(400.0))
            .height(Length::points(400.0));
        tx.set_children(sc, &[child]);
        tx.set_children(NodeId::ROOT, &[sc]);
        tx.commit_into(&mut changes);
        sc
    };
    let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
    scene.set_viewport(Viewport {
        width: 200.0,
        height: 200.0,
        scale_factor: 1.0,
    });
    scene.commit(&store, &changes);

    // 1. u32::MAX -> 1 wrap must trigger.
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(120.0))
            .requested_scroll_seq(u32::MAX);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 120.0));

    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_y(Some(50.0))
            .requested_scroll_seq(1);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    assert_eq!(scene.scroll_offset(scroller), (0.0, 50.0));

    // 2. 1 -> 0 -> 1 reset must re-trigger seq == 1 after clearing to 0!
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller).requested_scroll_seq(0);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert!(scene.reconcile_scroll_offsets(&store).is_empty());
    scene.set_scroll_offset_clamped(&store, scroller, 10.0, 180.0);
    {
        let mut tx = store.tx();
        tx.edit::<Scroll>(scroller)
            .requested_scroll_x(Some(75.0))
            .requested_scroll_y(None)
            .requested_scroll_seq(1);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[scroller]);
    // 3. Single-axis (Some(75.0), None) MUST preserve existing Y=180.0!
    assert_eq!(scene.scroll_offset(scroller), (75.0, 180.0));

    // 4. Recycled NodeId slot (same index, bumped generation) with seq == 1 MUST trigger!
    let old_idx = scroller.index();
    let recycled = {
        let mut tx = store.tx();
        tx.remove(scroller);
        let next = tx.create();
        let child2 = tx.create();
        tx.edit::<Layout>(next)
            .width(Length::points(100.0))
            .height(Length::points(100.0));
        tx.edit::<Scroll>(next)
            .requested_scroll_y(Some(90.0))
            .requested_scroll_seq(1);
        tx.edit::<Layout>(child2)
            .flex_shrink(0.0)
            .width(Length::points(200.0))
            .height(Length::points(400.0));
        tx.set_children(next, &[child2]);
        tx.set_children(NodeId::ROOT, &[next]);
        tx.commit_into(&mut changes);
        next
    };
    assert_eq!(recycled.index(), old_idx, "expected recycled NodeId index");
    assert_ne!(recycled, scroller);
    scene.commit(&store, &changes);
    assert_eq!(scene.reconcile_scroll_offsets(&store), &[recycled]);
    assert_eq!(scene.scroll_offset(recycled), (0.0, 90.0));
}

#[test]
fn compute_overlay_position_is_strictly_zero_alloc_across_all_placements_and_fallbacks() {
    let vp = OverlayViewport::new(360.0, 240.0);
    let anchor = Rect::new(300.0, 200.0, 40.0, 28.0);
    let overlay = Size::new(150.0, 90.0);
    let default_opts: Vec<PositionOptions> = Placement::ALL
        .iter()
        .map(|&p| PositionOptions::new(p).with_auto_size(true))
        .collect();
    let custom_fb = PositionOptions::new(Placement::BottomEnd)
        .with_fallbacks(vec![
            Placement::TopEnd,
            Placement::LeftStart,
            Placement::RightStart,
            Placement::Top,
        ])
        .with_auto_size(true);

    let (_, allocs) = count_allocs(|| {
        for _ in 0..50 {
            for opts in &default_opts {
                let pos = compute_overlay_position(anchor, overlay, vp, opts);
                assert!(pos.x.is_finite() && pos.y.is_finite());
            }
            let pos = compute_overlay_position(anchor, overlay, vp, &custom_fb);
            assert!(pos.x.is_finite() && pos.y.is_finite());
        }
    });
    assert_eq!(
        allocs, 0,
        "compute_overlay_position must perform 0 heap allocations, saw {allocs}"
    );
}

#[test]
fn top_layer_overlay_escapes_overflow_hidden_flips_on_collision_and_modal_blocks_background() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (bg_btn, overlay_node, overlay_btn) = {
        let mut tx = store.tx();
        let bg_btn = tx.create();
        tx.edit::<Layout>(bg_btn)
            .position(Position::Absolute)
            .inset_left(Length::points(10.0))
            .inset_top(Length::points(10.0))
            .width(Length::points(80.0))
            .height(Length::points(30.0));
        tx.edit::<Interaction>(bg_btn).listeners(EventSet::one(InteractionEvent::Press));

        let clipper = tx.create();
        tx.edit::<Layout>(clipper)
            .position(Position::Absolute)
            .inset_left(Length::points(200.0))
            .inset_top(Length::points(220.0))
            .width(Length::points(60.0))
            .height(Length::points(30.0));
        tx.edit::<Visual>(clipper)
            .overflow_x(Overflow::Hidden)
            .overflow_y(Overflow::Hidden);
        tx.edit::<DebugId>(clipper).id("trigger");

        let ov = tx.create();
        tx.edit::<Layout>(ov)
            .position(Position::Fixed)
            .width(Length::points(120.0))
            .height(Length::points(80.0));
        tx.edit::<Overlay>(ov)
            .layer(OverlayLayer::Popover)
            .anchor_id("trigger")
            .placement(OverlayPlacement::BottomStart)
            .offset_px(8.0)
            .viewport_padding_px(8.0)
            .options(OverlayFlags::OPEN | OverlayFlags::FLIP | OverlayFlags::SHIFT);

        let ov_btn = tx.create();
        tx.edit::<Layout>(ov_btn)
            .width(Length::points(100.0))
            .height(Length::points(40.0));
        tx.edit::<Interaction>(ov_btn).listeners(EventSet::one(InteractionEvent::Press));

        tx.set_children(ov, &[ov_btn]);
        tx.set_children(clipper, &[ov]);
        tx.set_children(NodeId::ROOT, &[bg_btn, clipper]);
        tx.commit_into(&mut changes);
        (bg_btn, ov, ov_btn)
    };

    let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
    scene.set_viewport(Viewport {
        width: 400.0,
        height: 280.0,
        scale_factor: 1.0,
    });
    scene.commit(&store, &changes);

    // Trigger is at y=220..250 in a 280px viewport -> BottomStart (y=258..338) overflows, so
    // overlay MUST flip to TopStart (y = 220 - 8 - 80 = 132), completely outside clipper (220..250)!
    let ro = scene.resolved_overlay(overlay_node).expect("open overlay resolved");
    assert_eq!(ro.placement, OverlayPlacement::TopStart);
    assert!((ro.origin_x - 200.0).abs() < 1e-3);
    assert!((ro.origin_y - 132.0).abs() < 1e-3);

    // Hit-testing outside the clipped ancestor hits overlay_btn in top-layer space!
    assert_eq!(scene.hit_test(&store, 220.0, 150.0), Some(overlay_btn));
    // Non-modal popover allows clicking the background button outside the popover.
    assert_eq!(scene.hit_test(&store, 30.0, 20.0), Some(bg_btn));

    // Switch overlay to Modal -> background button hits MUST be blocked!
    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(overlay_node).layer(OverlayLayer::Modal);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.hit_test(&store, 220.0, 150.0), Some(overlay_btn));
    assert_eq!(
        scene.hit_test(&store, 30.0, 20.0),
        None,
        "modal overlay backdrop must block hits to background elements"
    );

    // Closing overlay hides it from top-layer hit-testing and restores background hits.
    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(overlay_node).options(OverlayFlags::NONE);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert!(scene.resolved_overlay(overlay_node).is_none());
    assert_eq!(scene.hit_test(&store, 220.0, 150.0), None);
    assert_eq!(scene.hit_test(&store, 30.0, 20.0), Some(bg_btn));
}

#[test]
fn cursor_inherits_from_ancestors_and_converts_disabled_pointer_to_not_allowed() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (parent_btn, child_label) = {
        let mut tx = store.tx();
        let parent = tx.create();
        tx.edit::<Layout>(parent)
            .width(Length::points(120.0))
            .height(Length::points(40.0));
        tx.edit::<Interaction>(parent)
            .cursor(CursorKind::Pointer)
            .listeners(EventSet::one(InteractionEvent::Press));
        let child = tx.create();
        tx.edit::<Layout>(child)
            .width(Length::points(80.0))
            .height(Length::points(20.0));
        tx.edit::<Interaction>(child).listeners(EventSet::one(InteractionEvent::PointerMove));
        tx.set_children(parent, &[child]);
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.commit_into(&mut changes);
        (parent, child)
    };

    let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
    scene.set_viewport(Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    });
    scene.commit(&store, &changes);

    let _ = scene.dispatch_pointer_move(&store, 20.0, 10.0);
    assert_eq!(scene.interaction_tracker().hovered(), Some(child_label));
    assert_eq!(
        scene.current_cursor(&store),
        CursorKind::Pointer,
        "Auto cursor on child must inherit Pointer from ancestor"
    );

    // Disable parent AND set explicit Pointer cursor on child -> ancestor disabled MUST win!
    {
        let mut tx = store.tx();
        tx.edit::<Interaction>(parent_btn).disabled(true);
        tx.edit::<Interaction>(child_label).cursor(CursorKind::Pointer);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert_eq!(scene.current_cursor(&store), CursorKind::NotAllowed);
}

#[test]
fn overlay_tracks_transformed_scrolled_anchor_handles_missing_detach_and_explicit_insets() {
    register_core_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (scroller, anchor, ov, ov_child, explicit_modal) = {
        let mut tx = store.tx();
        let sc = tx.create();
        tx.edit::<Layout>(sc)
            .position(Position::Absolute)
            .inset_left(Length::points(0.0))
            .inset_top(Length::points(0.0))
            .width(Length::points(300.0))
            .height(Length::points(200.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Scroll>(sc);

        let spacer = tx.create();
        tx.edit::<Layout>(spacer)
            .flex_shrink(0.0)
            .width(Length::points(300.0))
            .height(Length::points(100.0));

        let anc = tx.create();
        tx.edit::<Layout>(anc)
            .flex_shrink(0.0)
            .width(Length::points(40.0))
            .height(Length::points(20.0));
        tx.edit::<Transform>(anc).translate_x(30.0).translate_y(15.0);
        tx.edit::<DebugId>(anc).id("tf-anchor");

        let tall = tx.create();
        tx.edit::<Layout>(tall)
            .flex_shrink(0.0)
            .width(Length::points(300.0))
            .height(Length::points(400.0));

        let ov = tx.create();
        tx.edit::<Layout>(ov)
            .position(Position::Fixed)
            .width(Length::points(80.0))
            .height(Length::points(40.0));
        tx.edit::<Overlay>(ov)
            .layer(OverlayLayer::Popover)
            .anchor_id("tf-anchor")
            .placement(OverlayPlacement::BottomStart)
            .offset_px(4.0)
            .options(OverlayFlags::OPEN);

        let ov_child = tx.create();
        tx.edit::<Layout>(ov_child)
            .width(Length::points(40.0))
            .height(Length::points(20.0));

        let explicit_modal = tx.create();
        tx.edit::<Layout>(explicit_modal)
            .position(Position::Fixed)
            .inset_right(Length::points(20.0))
            .inset_bottom(Length::points(16.0))
            .width(Length::points(100.0))
            .height(Length::points(50.0));
        tx.edit::<Overlay>(explicit_modal)
            .layer(OverlayLayer::Modal)
            .options(OverlayFlags::OPEN);

        tx.set_children(ov, &[ov_child]);
        tx.set_children(sc, &[spacer, anc, tall]);
        tx.set_children(NodeId::ROOT, &[sc, ov, explicit_modal]);
        tx.commit_into(&mut changes);
        (sc, anc, ov, ov_child, explicit_modal)
    };

    let mut scene = LaidOutScene::new(TaffyStoreLayout::new());
    scene.set_viewport(Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 1.0,
    });
    scene.commit(&store, &changes);

    // Anchor layout is (0, 100, 40, 20), plus translate (30, 15) -> visual rect (30, 115, 40, 20).
    // BottomStart + offset 4 -> (30, 139).
    let ro = scene.resolved_overlay(ov).expect("anchored overlay resolved");
    assert!((ro.origin_x - 30.0).abs() < 1e-3, "got {}", ro.origin_x);
    assert!((ro.origin_y - 139.0).abs() < 1e-3, "got {}", ro.origin_y);

    // Scrolling scroller by y=50 moves anchor visual rect to y=65 -> overlay origin_y = 65 + 20 + 4 = 89!
    scene.set_scroll_offset_clamped(&store, scroller, 0.0, 50.0);
    scene.reconcile_overlays(&store);
    let ro_scrolled = scene.resolved_overlay(ov).expect("scrolled overlay");
    assert!((ro_scrolled.origin_y - 89.0).abs() < 1e-3, "got {}", ro_scrolled.origin_y);

    // Explicit inset_right=20, inset_bottom=16 on unanchored modal must NOT viewport-center it!
    // Viewport 400x300, box 100x50 -> x=280, y=234 (not centered 150, 125).
    let em = scene.resolved_overlay(explicit_modal).expect("explicit modal");
    assert!((em.origin_x - 280.0).abs() < 1e-3, "got {}", em.origin_x);
    assert!((em.origin_y - 234.0).abs() < 1e-3, "got {}", em.origin_y);

    // Missing anchor_id falls back cleanly without panicking; detaching Overlay clears resolution;
    // is_subtree_closed_overlay tracks closed overlay descendants.
    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(ov).anchor_id("does-not-exist");
        tx.edit::<Overlay>(explicit_modal).options(OverlayFlags::NONE);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert!(scene.resolved_overlay(ov).is_some());
    assert!(is_subtree_closed_overlay(&store, explicit_modal));
    assert!(!is_subtree_closed_overlay(&store, ov_child));

    {
        let mut tx = store.tx();
        tx.edit::<Overlay>(ov).options(OverlayFlags::NONE);
        tx.commit_into(&mut changes);
    }
    assert!(is_subtree_closed_overlay(&store, ov_child));

    {
        let mut tx = store.tx();
        tx.detach::<Overlay>(ov);
        tx.commit_into(&mut changes);
    }
    scene.commit(&store, &changes);
    assert!(scene.resolved_overlay(ov).is_none());
    let _ = anchor;
}
