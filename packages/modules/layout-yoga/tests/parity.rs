//! Deep parity tests comparing [`YogaLayoutEngine`] (real React Native Yoga C++20)
//! against [`TaffyStoreLayout`] directly over [`NodeStore`].
//!
//! When compiled without `RN_REACT_COMMON_DIR`, tests verify the clean `YogaUnavailableError`
//! fallback path so `cargo test --workspace` always succeeds on every machine.
//! When compiled with `RN_REACT_COMMON_DIR`, every Flexbox parity, text wrapping, and mutation
//! assertion executes against real RN Yoga (`examples/kitchen-sink` + live engine swaps live in
//! `parity_kitchen_sink.rs`).

#[path = "support/rect_parity.rs"]
mod rect_parity;
#[path = "support/require_yoga.rs"]
mod require_yoga;

use std::sync::Arc;

use nve_abi::{EventSet, InteractionEvent, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_tables::tables::{
    AlignItems, FlexDirection, Interaction, JustifyContent, Layout, TextContent, Typography,
};
use nve_engine::{StoreBackend, Viewport};
use nve_layout::{StoreLayoutEngine, TextMeasurer};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, Length, NodeId, NodeStore};
use rect_parity::assert_rect_parity;
use require_yoga::require_yoga_or_skip;

fn compare_store_taffy_vs_yoga(
    label: &str,
    viewport: Viewport,
    store: &NodeStore,
    changes: &ChangeSet,
    node_ids: &[NodeId],
    custom_measurer: Option<Arc<dyn TextMeasurer>>,
) {
    let mut taffy_backend = HeadlessBackend::new();
    let mut yoga_backend =
        HeadlessBackend::with_layout(YogaLayoutEngine::new().expect("real RN Yoga available"));

    if let Some(m) = custom_measurer {
        taffy_backend.set_text_measurer(Arc::clone(&m));
        yoga_backend.set_text_measurer(m);
    }

    taffy_backend.set_viewport(SurfaceId::ROOT, viewport);
    yoga_backend.set_viewport(SurfaceId::ROOT, viewport);

    taffy_backend
        .commit(store, changes)
        .expect("taffy parity commit");
    yoga_backend
        .commit(store, changes)
        .expect("yoga parity commit");

    assert_eq!(taffy_backend.name(), "headless");
    assert_eq!(taffy_backend.layout_engine_name(), "taffy");
    assert_eq!(yoga_backend.name(), "headless");
    assert_eq!(yoga_backend.layout_engine_name(), "yoga");

    for &id in node_ids {
        let tr = taffy_backend.layout_of(id);
        assert!(tr.is_some(), "[{label}] missing taffy rect for node #{}", id.raw());
        let tr = tr.expect("checked above");
        let yr = yoga_backend.layout_of(id);
        assert!(yr.is_some(), "[{label}] missing yoga rect for node #{}", id.raw());
        let yr = yr.expect("checked above");
        assert_rect_parity(label, id, tr, yr);
    }
}

#[test]
fn parity_column_row_padding_margin_gap_flex_grow() {
    if !require_yoga_or_skip() {
        assert!(YogaLayoutEngine::new().is_err());
        return;
    }

    let viewport = Viewport {
        width: 400.0,
        height: 800.0,
        scale_factor: 2.0,
    };

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (n2, n3, n4, n5, n6, n7, n8);
    {
        let mut tx = store.tx();
        n2 = tx.create();
        tx.edit::<Layout>(n2)
            .width(Length::percent(100.0))
            .height(Length::percent(100.0))
            .flex_direction(FlexDirection::Column)
            .padding_top(Length::points(20.0))
            .padding_right(Length::points(16.0))
            .padding_bottom(Length::points(24.0))
            .padding_left(Length::points(16.0))
            .row_gap(Length::points(12.0))
            .column_gap(Length::points(12.0));

        n3 = tx.create();
        tx.edit::<Layout>(n3)
            .width(Length::percent(100.0))
            .height(Length::points(64.0))
            .flex_direction(FlexDirection::Row)
            .row_gap(Length::points(8.0))
            .column_gap(Length::points(8.0));

        n4 = tx.create();
        tx.edit::<Layout>(n4)
            .height(Length::points(48.0))
            .flex_grow(1.0)
            .margin_top(Length::points(4.0))
            .margin_left(Length::points(6.0));

        n5 = tx.create();
        tx.edit::<Layout>(n5)
            .height(Length::points(48.0))
            .flex_grow(2.0);

        n6 = tx.create();
        tx.edit::<Layout>(n6)
            .width(Length::percent(100.0))
            .flex_grow(1.0)
            .padding_top(Length::points(10.0))
            .padding_right(Length::points(10.0))
            .padding_bottom(Length::points(10.0))
            .padding_left(Length::points(10.0))
            .justify_content(JustifyContent::Center)
            .align_items(AlignItems::Center);

        n7 = tx.create();
        tx.edit::<Layout>(n7)
            .width(Length::points(140.0))
            .height(Length::points(44.0));

        n8 = tx.create();
        tx.edit::<Layout>(n8)
            .width(Length::percent(100.0))
            .height(Length::points(56.0));

        tx.set_children(n3, &[n4, n5]);
        tx.set_children(n6, &[n7]);
        tx.set_children(n2, &[n3, n6, n8]);
        tx.set_children(NodeId::ROOT, &[n2]);
        tx.commit_into(&mut changes);
    }

    compare_store_taffy_vs_yoga(
        "column_row_padding_margin_gap_flex_grow",
        viewport,
        &store,
        &changes,
        &[NodeId::ROOT, n2, n3, n4, n5, n6, n7, n8],
        None,
    );
}

#[test]
fn parity_justify_and_align_variants() {
    if !require_yoga_or_skip() {
        return;
    }

    let viewport = Viewport {
        width: 360.0,
        height: 240.0,
        scale_factor: 1.0,
    };

    let justifies = [
        JustifyContent::FlexStart,
        JustifyContent::Center,
        JustifyContent::FlexEnd,
        JustifyContent::SpaceBetween,
        JustifyContent::SpaceAround,
        JustifyContent::SpaceEvenly,
    ];
    let aligns = [
        AlignItems::FlexStart,
        AlignItems::Center,
        AlignItems::FlexEnd,
        AlignItems::Stretch,
    ];

    for justify in justifies {
        for align in aligns {
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            let mut ids = Vec::with_capacity(5);
            ids.push(NodeId::ROOT);
            {
                let mut tx = store.tx();
                let container = tx.create();
                tx.edit::<Layout>(container)
                    .width(Length::points(320.0))
                    .height(Length::points(180.0))
                    .flex_direction(FlexDirection::Row)
                    .justify_content(justify)
                    .align_items(align)
                    .padding_top(Length::points(10.0))
                    .padding_bottom(Length::points(10.0))
                    .padding_left(Length::points(12.0))
                    .padding_right(Length::points(12.0));
                ids.push(container);

                let mut kids = Vec::with_capacity(3);
                for i in 0..3u32 {
                    let kid = tx.create();
                    tx.edit::<Layout>(kid)
                        .width(Length::points(50.0 + (i as f32) * 10.0))
                        .height(if align == AlignItems::Stretch {
                            Length::AUTO
                        } else {
                            Length::points(30.0 + (i as f32) * 10.0)
                        });
                    kids.push(kid);
                    ids.push(kid);
                }
                tx.set_children(container, &kids);
                tx.set_children(NodeId::ROOT, &[container]);
                tx.commit_into(&mut changes);
            }

            let label = format!("justify={justify:?}_align={align:?}");
            compare_store_taffy_vs_yoga(&label, viewport, &store, &changes, &ids, None);
        }
    }
}

#[test]
fn parity_custom_text_measurer_wrapping_and_dynamic_text_updates() {
    if !require_yoga_or_skip() {
        return;
    }

    let viewport = Viewport {
        width: 300.0,
        height: 400.0,
        scale_factor: 1.0,
    };

    let measurer: Arc<dyn TextMeasurer> =
        Arc::new(|text: &str, style: &nve_layout::TextStyle<&str>, max_width: Option<f32>| {
            if text.is_empty() {
                return (0.0, 0.0);
            }
            let font_size = style.font_size;
            let char_w = font_size * 0.5;
            let line_h = font_size * 1.25;
            let raw_w = text.chars().count() as f32 * char_w;
            match max_width {
                Some(mw) if mw <= 0.0 => (0.0, line_h),
                Some(mw) if raw_w > mw => {
                    let lines = (raw_w / mw).ceil().max(1.0);
                    (mw, lines * line_h)
                }
                _ => (raw_w, line_h),
            }
        });

    let mut store = NodeStore::new();
    let mut changes1 = ChangeSet::default();
    let (n2, n3, n4);
    {
        let mut tx = store.tx();
        n2 = tx.create();
        tx.edit::<Layout>(n2)
            .width(Length::points(120.0))
            .flex_direction(FlexDirection::Column)
            .padding_top(Length::points(10.0))
            .padding_right(Length::points(10.0))
            .padding_bottom(Length::points(10.0))
            .padding_left(Length::points(10.0))
            .row_gap(Length::points(8.0))
            .column_gap(Length::points(8.0));

        n3 = tx.create();
        tx.edit::<Typography>(n3).font_size(20.0);
        tx.edit::<TextContent>(n3).text("1234567890123456789012345");

        n4 = tx.create();
        tx.edit::<Layout>(n4)
            .width(Length::points(80.0))
            .height(Length::points(32.0));
        tx.edit::<Interaction>(n4).listeners(EventSet::one(InteractionEvent::Press));

        tx.set_children(n2, &[n3, n4]);
        tx.set_children(NodeId::ROOT, &[n2]);
        tx.commit_into(&mut changes1);
    }

    compare_store_taffy_vs_yoga(
        "custom_text_wrapping_frame1",
        viewport,
        &store,
        &changes1,
        &[NodeId::ROOT, n2, n3, n4],
        Some(Arc::clone(&measurer)),
    );

    let mut taffy = HeadlessBackend::new();
    let mut yoga = HeadlessBackend::with_layout(YogaLayoutEngine::new().unwrap());
    taffy.set_text_measurer(Arc::clone(&measurer));
    yoga.set_text_measurer(Arc::clone(&measurer));
    taffy.set_viewport(SurfaceId::ROOT, viewport);
    yoga.set_viewport(SurfaceId::ROOT, viewport);

    taffy.commit(&store, &changes1).unwrap();
    yoga.commit(&store, &changes1).unwrap();

    let y_text_1 = yoga.layout_of(n3).unwrap();
    assert!((y_text_1.width - 100.0).abs() <= 1.0, "got {y_text_1:?}");
    assert!((y_text_1.height - 75.0).abs() <= 1.0, "got {y_text_1:?}");
    let y_sib_1 = yoga.layout_of(n4).unwrap();
    assert!((y_sib_1.y - 93.0).abs() <= 1.0, "got {y_sib_1:?}");

    // Frame 2: shorten text to 5 chars -> 50px -> 1 line = 25px, sibling moves up to y = 43!
    let mut changes2 = ChangeSet::default();
    {
        let mut tx = store.tx();
        tx.edit::<TextContent>(n3).text("Short");
        tx.commit_into(&mut changes2);
    }

    taffy.commit(&store, &changes2).unwrap();
    yoga.commit(&store, &changes2).unwrap();

    for id in [NodeId::ROOT, n2, n3, n4] {
        assert_rect_parity(
            "custom_text_wrapping_frame2",
            id,
            taffy.layout_of(id).unwrap(),
            yoga.layout_of(id).unwrap(),
        );
    }
    let y_text_2 = yoga.layout_of(n3).unwrap();
    assert!((y_text_2.height - 25.0).abs() <= 1.0, "got {y_text_2:?}");
    let y_sib_2 = yoga.layout_of(n4).unwrap();
    assert!((y_sib_2.y - 43.0).abs() <= 1.0, "got {y_sib_2:?}");
}

#[test]
fn yoga_handles_reparenting_removal_and_reset_cleanly() {
    if !require_yoga_or_skip() {
        return;
    }

    let mut engine = YogaLayoutEngine::new().unwrap();
    let viewport = Viewport {
        width: 300.0,
        height: 300.0,
        scale_factor: 1.0,
    };

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (n2, n3, n4);
    {
        let mut tx = store.tx();
        n2 = tx.create();
        tx.edit::<Layout>(n2)
            .width(Length::points(100.0))
            .height(Length::points(100.0))
            .padding_top(Length::points(10.0))
            .padding_right(Length::points(10.0))
            .padding_bottom(Length::points(10.0))
            .padding_left(Length::points(10.0));
        n3 = tx.create();
        tx.edit::<Layout>(n3)
            .width(Length::points(100.0))
            .height(Length::points(100.0))
            .padding_top(Length::points(20.0))
            .padding_right(Length::points(20.0))
            .padding_bottom(Length::points(20.0))
            .padding_left(Length::points(20.0));
        n4 = tx.create();
        tx.edit::<Layout>(n4)
            .width(Length::points(30.0))
            .height(Length::points(30.0));
        tx.set_children(n2, &[n4]);
        tx.set_children(NodeId::ROOT, &[n2, n3]);
        tx.commit_into(&mut changes);
    }

    engine.sync_changes(&store, &changes);
    engine.compute_layout(&store, NodeId::ROOT, viewport);
    assert_eq!(engine.layout_of(n4).unwrap().x, 10.0);
    assert_eq!(engine.layout_of(n4).unwrap().y, 10.0);

    // Reparent node 4 from node 2 -> node 3.
    {
        let mut tx = store.tx();
        tx.set_children(n2, &[]);
        tx.set_children(n3, &[n4]);
        tx.commit_into(&mut changes);
    }
    engine.sync_changes(&store, &changes);
    engine.compute_layout(&store, NodeId::ROOT, viewport);
    let r4 = engine.layout_of(n4).unwrap();
    assert_eq!(r4.x, 20.0);
    assert_eq!(r4.y, 120.0);

    // Remove node 4 and verify layout_of returns None.
    {
        let mut tx = store.tx();
        tx.remove(n4);
        tx.commit_into(&mut changes);
    }
    engine.sync_changes(&store, &changes);
    engine.compute_layout(&store, NodeId::ROOT, viewport);
    assert!(engine.layout_of(n4).is_none());

    // Reset clears everything cleanly and allows rebuilding.
    engine.reset();
    assert!(engine.layout_of(NodeId::ROOT).is_none());
    let mut fresh = NodeStore::new();
    let mut c_fresh = ChangeSet::default();
    {
        let tx = fresh.tx();
        tx.commit_into(&mut c_fresh);
    }
    c_fresh.created.push(NodeId::ROOT);
    engine.sync_changes(&fresh, &c_fresh);
    engine.compute_layout(&fresh, NodeId::ROOT, viewport);
    assert_eq!(engine.layout_of(NodeId::ROOT).unwrap().width, 300.0);
}
