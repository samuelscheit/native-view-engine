//! Deep Taffy layout, CSS Grid, `flex-basis`, `aspect-ratio`, `position: absolute/sticky`,
//! RTL logical layout, and `Transform2D` inverse-matrix hit-testing tests for `HeadlessBackend`.

use nve_abi::{
    CommandBufferBuilder, ComponentKind, Dimension, EventMask, FlexAlign, FlexDirection,
    LayoutProps, ROOT_NODE_ID,
};
use nve_backend_headless::{
    Display, GridItemPlacement, GridLayout, GridLine, GridTrack, GridTrackPlacement,
    HeadlessBackend, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
    Transform2D,
};
use nve_engine::{RenderBackend, Viewport};

fn assert_close(actual: f32, expected: f32, eps: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= eps,
        "{label}: expected {expected}, got {actual}"
    );
}

#[test]
fn css_grid_minmax_zero_fr_prevents_blowout_and_supports_spans_and_implicit_rows() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 420.0,
            height: 400.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None); // Grid container (420px wide)
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(420.0),
            ..Default::default()
        },
    );
    // Child 3: very long text that would blow out a naive `1fr` track if min wasn't `0`
    b.create_node(3, ComponentKind::Text, None);
    b.set_text(
        3,
        "SuperLongUnbreakableTokenThatWouldNormallyCauseGridColumnBlowoutWithoutMinMaxZero",
    );
    b.set_layout(
        3,
        &LayoutProps {
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    // Child 4: normal cell in col 2, row 1
    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    // Child 5: spans 2 columns in row 2 (implicit row)
    b.create_node(5, ComponentKind::View, None);
    b.set_layout(
        5,
        &LayoutProps {
            height: Dimension::Points(50.0),
            ..Default::default()
        },
    );
    // Child 6: explicit col 1, row 3..5 (row span 2)
    b.create_node(6, ComponentKind::View, None);
    b.set_layout(
        6,
        &LayoutProps {
            height: Dimension::Points(80.0),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.set_children(2, &[3, 4, 5, 6]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();

    // Configure 2 columns: repeat(2, minmax(0, 1fr)), column_gap=20, row_gap=10
    // Expected column width: (420 - 20) / 2 = 200px each!
    let mut grid_ext = LayoutExt {
        display: Display::Grid,
        ..Default::default()
    };
    grid_ext.grid = Some(Box::new(GridLayout {
        template_columns: vec![GridTrack::repeat_minmax_zero_fr(2)],
        template_rows: vec![GridTrack::px(40.0)],
        column_gap: Some(20.0),
        row_gap: Some(10.0),
    }));
    backend.set_node_layout_ext(2, Some(grid_ext));

    // Child 5 spans 2 columns -> full 420px width!
    backend.set_node_layout_ext(
        5,
        Some(LayoutExt {
            grid_item: GridItemPlacement {
                column: GridTrackPlacement::span(2),
                row: GridTrackPlacement::auto(),
            },
            ..Default::default()
        }),
    );

    // Child 6 explicitly placed at col 1, row 3 / span 2
    backend.set_node_layout_ext(
        6,
        Some(LayoutExt {
            grid_item: GridItemPlacement {
                column: GridTrackPlacement::line(1),
                row: GridTrackPlacement {
                    start: GridLine::Line(3),
                    end: GridLine::Span(2),
                },
            },
            ..Default::default()
        }),
    );
    backend.recompute_layout();

    let r3 = backend.layout_of(3).unwrap();
    let r4 = backend.layout_of(4).unwrap();
    let r5 = backend.layout_of(5).unwrap();
    let r6 = backend.layout_of(6).unwrap();

    // Blowout protection: col 1 stays strictly 200px wide despite long unbreakable text!
    assert_close(r3.x, 0.0, 0.5, "col1 x");
    assert_close(r3.width, 200.0, 0.5, "col1 width no blowout");
    assert_close(r3.y, 0.0, 0.5, "row1 y");

    // Col 2 starts after 200px + 20px column_gap = 220px
    assert_close(r4.x, 220.0, 0.5, "col2 x");
    assert_close(r4.width, 200.0, 0.5, "col2 width");
    assert_close(r4.y, 0.0, 0.5, "row1 col2 y");

    // Child 5 spans both columns (200 + 20 + 200 = 420) on implicit row 2 (y = 40 + 10 = 50)
    assert_close(r5.x, 0.0, 0.5, "span2 x");
    assert_close(r5.width, 420.0, 0.5, "span2 width");
    assert_close(r5.y, 50.0, 0.5, "implicit row2 y");
    assert_close(r5.height, 50.0, 0.5, "implicit row2 h");

    // Child 6 sits on row 3 (y = 50 + 50 + 10 = 110)
    assert_close(r6.x, 0.0, 0.5, "row3 x");
    assert_close(r6.y, 110.0, 0.5, "row3 y");
    assert_close(r6.width, 200.0, 0.5, "row3 col1 width");
}

#[test]
fn flex_basis_aspect_ratio_and_absolute_position_insets() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 400.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None); // Row container 400x300
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(400.0),
            height: Dimension::Points(300.0),
            flex_direction: FlexDirection::Row,
            align_items: FlexAlign::FlexStart,
            ..Default::default()
        },
    );
    // Child 3: flex-basis: 160px, aspect-ratio: 16/9 -> width=160, height=90
    b.create_node(3, ComponentKind::View, None);
    // Child 4: flex-basis: 25% -> 100px wide
    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            height: Dimension::Points(50.0),
            ..Default::default()
        },
    );
    // Child 5: position: absolute; right: 12px; bottom: 20px; width: 80px; height: 40px
    b.create_node(5, ComponentKind::View, None);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(80.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.set_children(2, &[3, 4, 5]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();

    backend.set_node_layout_ext(
        3,
        Some(LayoutExt {
            flex_basis: Dimension::Points(160.0),
            aspect_ratio: Some(16.0 / 9.0),
            ..Default::default()
        }),
    );
    backend.set_node_layout_ext(
        4,
        Some(LayoutExt {
            flex_basis: Dimension::Percent(25.0),
            ..Default::default()
        }),
    );
    backend.set_node_layout_ext(
        5,
        Some(LayoutExt {
            position: PositionMode::Absolute,
            insets: PositionInsets {
                right: Dimension::Points(12.0),
                bottom: Dimension::Points(20.0),
                ..PositionInsets::AUTO
            },
            ..Default::default()
        }),
    );
    backend.recompute_layout();

    let r3 = backend.layout_of(3).unwrap();
    assert_close(r3.width, 160.0, 0.5, "flex-basis px width");
    assert_close(r3.height, 90.0, 0.5, "aspect-ratio 16/9 height");

    let r4 = backend.layout_of(4).unwrap();
    assert_close(r4.x, 160.0, 0.5, "child4 x after child3");
    assert_close(r4.width, 100.0, 0.5, "flex-basis 25% width");

    let r5 = backend.layout_of(5).unwrap();
    // 400 - 12 - 80 = 308, 300 - 20 - 40 = 240
    assert_close(r5.x, 308.0, 0.5, "abs right:12 x");
    assert_close(r5.y, 240.0, 0.5, "abs bottom:20 y");
}

#[test]
fn rtl_logical_padding_margin_and_row_mirroring_with_inheritance() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::View, None); // RTL Row container: width=300, pad_inline_start=20, pad_inline_end=10
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(100.0),
            flex_direction: FlexDirection::Row,
            ..Default::default()
        },
    );
    // Item A (first in DOM): width=60, margin_inline_start=15
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(60.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    // Item B (second in DOM): width=80
    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(80.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.set_children(2, &[3, 4]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();

    backend.set_node_layout_ext(
        2,
        Some(LayoutExt {
            direction: Some(LayoutDirection::Rtl),
            padding_inline: LogicalInlineEdges {
                start: Some(20.0),
                end: Some(10.0),
            },
            ..Default::default()
        }),
    );
    // Child 3 inherits RTL (direction = None) and declares margin_inline_start = 15 -> right margin 15!
    backend.set_node_layout_ext(
        3,
        Some(LayoutExt {
            margin_inline: LogicalInlineEdges {
                start: Some(15.0),
                end: None,
            },
            ..Default::default()
        }),
    );
    backend.recompute_layout();

    let ra = backend.layout_of(3).unwrap();
    let rb = backend.layout_of(4).unwrap();
    // Under RTL: right edge = 300. pad_inline_start=20 (right) + margin_inline_start=15 (right)
    // Item A (w=60) lands at x = 300 - 20 - 15 - 60 = 205!
    // Item B (w=80) lands to the left of Item A at x = 205 - 80 = 125!
    assert_close(ra.x, 205.0, 0.5, "RTL item A x");
    assert_close(rb.x, 125.0, 0.5, "RTL item B x");
    assert!(ra.x > rb.x, "RTL row must place first child to the right of second child");

    // Flip container to LTR -> item A moves to left (pad_start=20 + mar_start=15 = 35), item B at 95!
    backend.set_node_layout_ext(
        2,
        Some(LayoutExt {
            direction: Some(LayoutDirection::Ltr),
            padding_inline: LogicalInlineEdges {
                start: Some(20.0),
                end: Some(10.0),
            },
            ..Default::default()
        }),
    );
    backend.recompute_layout();
    let ra_ltr = backend.layout_of(3).unwrap();
    let rb_ltr = backend.layout_of(4).unwrap();
    assert_close(ra_ltr.x, 35.0, 0.5, "LTR item A x");
    assert_close(rb_ltr.x, 95.0, 0.5, "LTR item B x");
}

#[test]
fn sticky_clamping_and_transform2d_hit_testing_in_headless() {
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 300.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    // ScrollView (node 2): 300x300
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(300.0),
            ..Default::default()
        },
    );
    // Top spacer (node 3): 100px tall
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(100.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    // Section containing block (node 4): 400px tall (y=100..500)
    b.create_node(4, ComponentKind::View, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(400.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    // Sticky header (node 5): 40px tall inside section (normal y=100..140)
    b.create_node(5, ComponentKind::Pressable, None);
    b.set_event_mask(5, EventMask::PRESS);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    // Transformed button (node 6) outside ScrollView: base rect x=0..100, y=0..40, translated +120,+50
    b.create_node(6, ComponentKind::Pressable, None);
    b.set_event_mask(6, EventMask::PRESS);
    b.set_layout(
        6,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2, 6]);
    b.set_children(2, &[3, 4]);
    b.set_children(4, &[5]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();

    backend.set_node_layout_ext(
        5,
        Some(LayoutExt {
            position: PositionMode::Sticky,
            insets: PositionInsets {
                top: Dimension::Points(10.0),
                ..PositionInsets::AUTO
            },
            ..Default::default()
        }),
    );
    backend.set_node_layout_ext(
        6,
        Some(LayoutExt {
            position: PositionMode::Absolute,
            insets: PositionInsets {
                top: Dimension::Points(0.0),
                left: Dimension::Points(0.0),
                ..PositionInsets::AUTO
            },
            ..Default::default()
        }),
    );
    backend.set_node_transform(6, Some(Transform2D::translate(120.0, 50.0)));
    backend.recompute_layout();

    // Sticky Phase 1: before threshold (scroll_y = 40 -> visual y = 100 - 40 = 60)
    backend.set_scroll_offset(2, 0.0, 40.0);
    assert_close(backend.visual_rect_of(5).unwrap().y, 60.0, 0.5, "sticky before");
    assert_eq!(backend.hit_test(50.0, 75.0, true), Some(5));

    // Sticky Phase 2: clamped at top: 10px (scroll_y = 200 -> visual y = 10)
    backend.set_scroll_offset(2, 0.0, 200.0);
    assert_close(backend.visual_rect_of(5).unwrap().y, 10.0, 0.5, "sticky clamped");
    assert_eq!(backend.hit_test(50.0, 25.0, true), Some(5));

    // Sticky Phase 3: pushed out at container bottom (scroll_y = 480 -> container bottom=20, header y=-20..20)
    backend.set_scroll_offset(2, 0.0, 480.0);
    assert_close(backend.visual_rect_of(5).unwrap().y, -20.0, 0.5, "sticky pushed out");
    assert_eq!(backend.hit_test(50.0, 10.0, true), Some(5));
    assert_eq!(backend.hit_test(50.0, 30.0, true), None);

    // Transform2D hit-testing on node 6: base (0..100, 0..40) translated to (120..220, 50..90).
    // Reset scroll so sticky header node 5 sits back at y=100..140, leaving (0..100, 0..40) unobstructed.
    backend.set_scroll_offset(2, 0.0, 0.0);
    assert_eq!(backend.hit_test(170.0, 70.0, true), Some(6));
    assert_eq!(backend.hit_test(50.0, 20.0, true), None);

    // Rotate node 6 by 90deg around center (50, 20) -> mapped box is x=[30..70], y=[-30..70]
    backend.set_node_transform(6, Some(Transform2D::rotate(90.0)));
    assert_eq!(backend.hit_test(50.0, 60.0, true), Some(6)); // inside rotated vertical span
    assert_eq!(backend.hit_test(90.0, 20.0, true), None); // outside rotated box

    // Singular scale(0) is un-hittable
    backend.set_node_transform(6, Some(Transform2D::scale(0.0)));
    assert_eq!(backend.hit_test(50.0, 20.0, true), None);
}
