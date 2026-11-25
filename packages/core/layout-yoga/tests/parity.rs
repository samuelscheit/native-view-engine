//! Deep parity & integration test suite comparing pure-Rust `TaffyLayoutEngine` against
//! real React Native Yoga (`YogaLayoutEngine`).
//!
//! When compiled without `RN_REACT_COMMON_DIR`, tests verify the clean `YogaUnavailableError`
//! fallback path so `cargo test --workspace` always succeeds on every machine.
//! When compiled with `RN_REACT_COMMON_DIR`, every Flexbox parity, text wrapping, mutation,
//! live engine swap, and `examples/kitchen-sink` CommandBuffer assertion executes against real RN Yoga.

use std::sync::Arc;

use nve_abi::{
    CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask, FlexAlign,
    FlexDirection, FlexJustify, LayoutProps, NodeId, StyleProps, ROOT_NODE_ID,
};
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_engine::{DynLayoutEngine, LayoutEngine, LayoutRect, RenderBackend, TextMeasurer, Viewport};
use nve_layout_yoga::YogaLayoutEngine;

const PARITY_TOLERANCE_PX: f32 = 1.0;

fn assert_rect_parity(label: &str, id: NodeId, taffy: LayoutRect, yoga: LayoutRect) {
    assert!(
        (taffy.x - yoga.x).abs() <= PARITY_TOLERANCE_PX
            && (taffy.y - yoga.y).abs() <= PARITY_TOLERANCE_PX
            && (taffy.width - yoga.width).abs() <= PARITY_TOLERANCE_PX
            && (taffy.height - yoga.height).abs() <= PARITY_TOLERANCE_PX,
        "[{label}] node #{id} diverged beyond {PARITY_TOLERANCE_PX}px:\n  taffy: {taffy:?}\n  yoga : {yoga:?}"
    );
}

fn compare_frame_taffy_vs_yoga(
    label: &str,
    viewport: Viewport,
    command_bytes: &[u8],
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

    taffy_backend.set_viewport(1, viewport);
    yoga_backend.set_viewport(1, viewport);

    taffy_backend.submit_frame(command_bytes).unwrap();
    yoga_backend.submit_frame(command_bytes).unwrap();

    assert_eq!(taffy_backend.name(), "headless-taffy");
    assert_eq!(yoga_backend.name(), "headless-yoga");

    for &id in node_ids {
        let tr = taffy_backend
            .layout_of(id)
            .unwrap_or_else(|| panic!("[{label}] missing taffy rect for node #{id}"));
        let yr = yoga_backend
            .layout_of(id)
            .unwrap_or_else(|| panic!("[{label}] missing yoga rect for node #{id}"));
        assert_rect_parity(label, id, tr, yr);
    }
}

#[test]
fn parity_column_row_padding_margin_gap_flex_grow() {
    if !YogaLayoutEngine::is_available() {
        assert!(YogaLayoutEngine::new().is_err());
        return;
    }

    let viewport = Viewport {
        width: 400.0,
        height: 800.0,
        scale_factor: 2.0,
    };

    let mut b = CommandBufferBuilder::new(1, 1);
    // Root column container with padding and gap
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: EdgeInsets {
                top: 20.0,
                right: 16.0,
                bottom: 24.0,
                left: 16.0,
            },
            gap: 12.0,
            ..Default::default()
        },
    );

    // Header row with flex_grow children (1:2 ratio) + margin
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Points(64.0),
            flex_direction: FlexDirection::Row,
            gap: 8.0,
            ..Default::default()
        },
    );

    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            height: Dimension::Points(48.0),
            flex_grow: 1.0,
            margin: EdgeInsets {
                top: 4.0,
                left: 6.0,
                right: 0.0,
                bottom: 0.0,
            },
            ..Default::default()
        },
    );

    b.create_node(5, ComponentKind::View, None);
    b.set_layout(
        5,
        &LayoutProps {
            height: Dimension::Points(48.0),
            flex_grow: 2.0,
            ..Default::default()
        },
    );

    // Flexible body filling remaining column height
    b.create_node(6, ComponentKind::View, None);
    b.set_layout(
        6,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            flex_grow: 1.0,
            padding: EdgeInsets::all(10.0),
            justify_content: FlexJustify::Center,
            align_items: FlexAlign::Center,
            ..Default::default()
        },
    );

    b.create_node(7, ComponentKind::Pressable, None);
    b.set_layout(
        7,
        &LayoutProps {
            width: Dimension::Points(140.0),
            height: Dimension::Points(44.0),
            ..Default::default()
        },
    );

    // Footer with fixed height
    b.create_node(8, ComponentKind::View, None);
    b.set_layout(
        8,
        &LayoutProps {
            width: Dimension::Percent(100.0),
            height: Dimension::Points(56.0),
            ..Default::default()
        },
    );

    b.set_children(3, &[4, 5]);
    b.set_children(6, &[7]);
    b.set_children(2, &[3, 6, 8]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);

    compare_frame_taffy_vs_yoga(
        "column_row_padding_margin_gap_flex_grow",
        viewport,
        b.finish(),
        &[ROOT_NODE_ID, 2, 3, 4, 5, 6, 7, 8],
        None,
    );
}

#[test]
fn parity_justify_and_align_variants() {
    if !YogaLayoutEngine::is_available() {
        return;
    }

    let viewport = Viewport {
        width: 360.0,
        height: 240.0,
        scale_factor: 1.0,
    };

    let justifies = [
        FlexJustify::FlexStart,
        FlexJustify::Center,
        FlexJustify::FlexEnd,
        FlexJustify::SpaceBetween,
        FlexJustify::SpaceAround,
        FlexJustify::SpaceEvenly,
    ];
    let aligns = [
        FlexAlign::FlexStart,
        FlexAlign::Center,
        FlexAlign::FlexEnd,
        FlexAlign::Stretch,
    ];

    for justify in justifies {
        for align in aligns {
            let mut b = CommandBufferBuilder::new(1, 1);
            b.create_node(2, ComponentKind::View, None);
            b.set_layout(
                2,
                &LayoutProps {
                    width: Dimension::Points(320.0),
                    height: Dimension::Points(180.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: justify,
                    align_items: align,
                    padding: EdgeInsets::symmetric(10.0, 12.0),
                    ..Default::default()
                },
            );

            for i in 0..3u32 {
                let id = 10 + i;
                b.create_node(id, ComponentKind::View, None);
                b.set_layout(
                    id,
                    &LayoutProps {
                        width: Dimension::Points(50.0 + (i as f32) * 10.0),
                        height: if align == FlexAlign::Stretch {
                            Dimension::Auto
                        } else {
                            Dimension::Points(30.0 + (i as f32) * 10.0)
                        },
                        ..Default::default()
                    },
                );
            }
            b.set_children(2, &[10, 11, 12]);
            b.set_children(ROOT_NODE_ID, &[2]);
            b.commit(true);

            let label = format!("justify={justify:?}_align={align:?}");
            compare_frame_taffy_vs_yoga(
                &label,
                viewport,
                b.finish(),
                &[ROOT_NODE_ID, 2, 10, 11, 12],
                None,
            );
        }
    }
}

#[test]
fn parity_examples_kitchen_sink_command_buffer_and_live_swap() {
    if !YogaLayoutEngine::is_available() {
        return;
    }

    let mut surface = nve_example_kitchen_sink::create_surface(1);
    let (_, _, cmd_bytes) = surface.render_frame(1);

    let viewport = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
    };

    let mut taffy_backend = HeadlessBackend::new();
    taffy_backend.set_viewport(1, viewport);
    taffy_backend.submit_frame(&cmd_bytes).unwrap();

    let mut yoga_backend =
        HeadlessBackend::with_layout(YogaLayoutEngine::new().expect("real RN Yoga available"));
    yoga_backend.set_viewport(1, viewport);
    yoga_backend.submit_frame(&cmd_bytes).unwrap();

    let mut node_ids: Vec<NodeId> = taffy_backend.nodes().keys().copied().collect();
    node_ids.sort_unstable();
    assert!(
        node_ids.len() >= 20,
        "expected full kitchen-sink UI tree, found {} nodes",
        node_ids.len()
    );

    for &id in &node_ids {
        let tr = taffy_backend.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("examples/kitchen-sink", id, tr, yr);
    }

    // Live swap Taffy -> Yoga -> Box<dyn LayoutEngine> -> Taffy and verify all rects stay in parity!
    let swapped_to_yoga: HeadlessBackend<YogaLayoutEngine> =
        taffy_backend.swap_layout_engine(YogaLayoutEngine::new().unwrap());
    assert_eq!(swapped_to_yoga.layout_engine().name(), "yoga");
    for &id in &node_ids {
        let sr = swapped_to_yoga.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("counter_after_swap_to_yoga", id, sr, yr);
    }

    let boxed_yoga: DynLayoutEngine = Box::new(YogaLayoutEngine::new().unwrap());
    let swapped_to_dyn: HeadlessBackend<DynLayoutEngine> =
        swapped_to_yoga.swap_layout_engine(boxed_yoga);
    assert_eq!(swapped_to_dyn.layout_engine().name(), "yoga");
    for &id in &node_ids {
        let dr = swapped_to_dyn.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("counter_after_swap_to_dyn_yoga", id, dr, yr);
    }

    let swapped_back_taffy: HeadlessBackend<TaffyLayoutEngine> =
        swapped_to_dyn.swap_layout_engine(TaffyLayoutEngine::new());
    for &id in &node_ids {
        let tr = swapped_back_taffy.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("counter_after_swap_back_to_taffy", id, tr, yr);
    }
}

#[test]
fn parity_custom_text_measurer_wrapping_and_dynamic_text_updates() {
    if !YogaLayoutEngine::is_available() {
        return;
    }

    let viewport = Viewport {
        width: 300.0,
        height: 400.0,
        scale_factor: 1.0,
    };

    // Deterministic word-wrapping custom TextMeasurer: 10px per char, 20px per line.
    let measurer: Arc<dyn TextMeasurer> =
        Arc::new(|text: &str, font_size: f32, max_width: Option<f32>| {
            if text.is_empty() {
                return (0.0, 0.0);
            }
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

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(120.0),
            flex_direction: FlexDirection::Column,
            padding: EdgeInsets::all(10.0), // content width = 100.0
            gap: 8.0,
            ..Default::default()
        },
    );

    // 25 chars * (20 * 0.5 = 10px) = 250px -> wraps in 100px content box to 3 lines * 25px = 75px!
    b.create_node(3, ComponentKind::Text, None);
    b.set_style(
        3,
        &StyleProps {
            font_size: 20.0,
            ..Default::default()
        },
    );
    b.set_text(3, "1234567890123456789012345");

    // Sibling below text verifies wrapped height pushes sibling down identically!
    b.create_node(4, ComponentKind::Pressable, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(80.0),
            height: Dimension::Points(32.0),
            ..Default::default()
        },
    );
    b.set_event_mask(4, EventMask::PRESS);

    b.set_children(2, &[3, 4]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    let frame1 = b.into_bytes();

    compare_frame_taffy_vs_yoga(
        "custom_text_wrapping_frame1",
        viewport,
        &frame1,
        &[ROOT_NODE_ID, 2, 3, 4],
        Some(Arc::clone(&measurer)),
    );

    // Verify the exact wrapped dimensions on both engines and dynamic update on frame 2.
    let mut taffy = HeadlessBackend::new();
    let mut yoga = HeadlessBackend::with_layout(YogaLayoutEngine::new().unwrap());
    taffy.set_text_measurer(Arc::clone(&measurer));
    yoga.set_text_measurer(Arc::clone(&measurer));
    taffy.set_viewport(1, viewport);
    yoga.set_viewport(1, viewport);

    taffy.submit_frame(&frame1).unwrap();
    yoga.submit_frame(&frame1).unwrap();

    let y_text_1 = yoga.layout_of(3).unwrap();
    assert!((y_text_1.width - 100.0).abs() <= 1.0, "got {y_text_1:?}");
    assert!((y_text_1.height - 75.0).abs() <= 1.0, "got {y_text_1:?}");
    let y_sib_1 = yoga.layout_of(4).unwrap();
    // 10 (pad_top) + 75 (wrapped text) + 8 (gap) = 93
    assert!((y_sib_1.y - 93.0).abs() <= 1.0, "got {y_sib_1:?}");

    // Frame 2: shorten text to 5 chars -> 50px -> 1 line = 25px, sibling moves up to y = 10 + 25 + 8 = 43!
    let mut b2 = CommandBufferBuilder::new(1, 2);
    b2.set_text(3, "Short");
    b2.commit(true);
    let frame2 = b2.into_bytes();

    taffy.submit_frame(&frame2).unwrap();
    yoga.submit_frame(&frame2).unwrap();

    for id in [ROOT_NODE_ID, 2, 3, 4] {
        assert_rect_parity(
            "custom_text_wrapping_frame2",
            id,
            taffy.layout_of(id).unwrap(),
            yoga.layout_of(id).unwrap(),
        );
    }
    let y_text_2 = yoga.layout_of(3).unwrap();
    assert!((y_text_2.height - 25.0).abs() <= 1.0, "got {y_text_2:?}");
    let y_sib_2 = yoga.layout_of(4).unwrap();
    assert!((y_sib_2.y - 43.0).abs() <= 1.0, "got {y_sib_2:?}");
}

#[test]
fn yoga_handles_reparenting_removal_and_reset_cleanly() {
    if !YogaLayoutEngine::is_available() {
        return;
    }

    let mut engine = YogaLayoutEngine::new().unwrap();
    let viewport = Viewport {
        width: 300.0,
        height: 300.0,
        scale_factor: 1.0,
    };

    engine.upsert_node(ROOT_NODE_ID, ComponentKind::Root);
    engine.upsert_node(2, ComponentKind::View);
    engine.upsert_node(3, ComponentKind::View);
    engine.upsert_node(4, ComponentKind::View);

    engine.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(100.0),
            padding: EdgeInsets::all(10.0),
            ..Default::default()
        },
    );
    engine.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(100.0),
            padding: EdgeInsets::all(20.0),
            ..Default::default()
        },
    );
    engine.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(30.0),
            height: Dimension::Points(30.0),
            ..Default::default()
        },
    );

    engine.set_children(2, &[4]);
    engine.set_children(ROOT_NODE_ID, &[2, 3]);
    engine.compute_layout(ROOT_NODE_ID, viewport);
    assert_eq!(engine.layout_of(4).unwrap().x, 10.0);
    assert_eq!(engine.layout_of(4).unwrap().y, 10.0);

    // Reparent node 4 from node 2 -> node 3 without removing it from 2 first.
    engine.set_children(3, &[4]);
    engine.set_children(2, &[]);
    engine.compute_layout(ROOT_NODE_ID, viewport);
    let r4 = engine.layout_of(4).unwrap();
    assert_eq!(r4.x, 20.0);
    assert_eq!(r4.y, 120.0); // node 3 starts at y=100, padding top=20

    // Remove node 4 and verify layout_of returns None.
    engine.remove_node(4);
    engine.set_children(3, &[]);
    engine.compute_layout(ROOT_NODE_ID, viewport);
    assert!(engine.layout_of(4).is_none());

    // Reset clears everything cleanly and allows rebuilding.
    engine.reset();
    assert!(engine.layout_of(ROOT_NODE_ID).is_none());
    engine.upsert_node(ROOT_NODE_ID, ComponentKind::Root);
    engine.compute_layout(ROOT_NODE_ID, viewport);
    assert_eq!(engine.layout_of(ROOT_NODE_ID).unwrap().width, 300.0);
}
