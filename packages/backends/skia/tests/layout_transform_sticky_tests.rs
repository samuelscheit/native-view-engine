//! Deep Skia backend tests for `PaintOp::PushTransform`/`PopTransform`, software rasterizer
//! transformed pixels, `position: sticky` display-list + hit-test clamping, `Transform2D`
//! hit-testing, and `MotionDriver` `TransitionProperty::Transform` animation.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EventMask, LayoutProps, StyleProps,
    ROOT_NODE_ID,
};
use nve_backend_skia::{
    EasingCurve, LayoutExt, MotionPreference, PaintOp, PixelBuffer, PositionInsets, PositionMode,
    SkiaBackend, StateStyleRule, StylePatch, Transform2D, TransitionProperty, TransitionSpec,
    InteractionState,
};
use nve_engine::{RenderBackend, Viewport};

fn assert_close(actual: f32, expected: f32, eps: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= eps,
        "{label}: expected {expected}, got {actual}"
    );
}

#[test]
fn skia_emits_push_pop_transform_and_rasterizes_translated_and_rotated_boxes() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 200.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_event_mask(2, EventMask::PRESS);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(60.0),
            height: Dimension::Points(20.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(239, 68, 68),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    // Translate (0..60, 0..20) by (+80, +60) -> visually at (80..140, 60..80)
    skia.set_node_transform(2, Some(Transform2D::translate(80.0, 60.0)));

    let ops = &skia.display_list().ops;
    assert!(
        ops.iter().any(|op| matches!(op, PaintOp::PushTransform { matrix } if (matrix[4] - 80.0).abs() < 1e-4 && (matrix[5] - 60.0).abs() < 1e-4)),
        "expected PushTransform in display list: {ops:?}"
    );
    assert!(ops.iter().any(|op| matches!(op, PaintOp::PopTransform)));

    // Rasterize and check pixels: old rect (20, 10) is clear black; translated center (110, 70) is red!
    let mut buf = PixelBuffer::new(200, 200);
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    assert_eq!(buf.get_rgb(20, 10), Some((0, 0, 0)));
    assert_eq!(buf.get_rgb(110, 70), Some((239, 68, 68)));

    // Hit-test lands at translated coordinates, misses untranslated coordinates!
    assert_eq!(skia.hit_test(110.0, 70.0), Some(2));
    assert_eq!(skia.hit_test(20.0, 10.0), None);

    // Rotate 90deg around center (30, 10) with translation (+70, +90) -> center at (100, 100),
    // rotated size is 20px wide (x=90..110) x 60px tall (y=70..130)!
    skia.set_node_transform(
        2,
        Some(Transform2D::translate(70.0, 90.0).with_rotate(90.0)),
    );
    skia.rasterize(&mut buf, ColorRgba::BLACK);
    // Point (100, 75) is inside rotated vertical bar; (75, 100) is outside!
    assert_eq!(buf.get_rgb(100, 75), Some((239, 68, 68)));
    assert_eq!(buf.get_rgb(75, 100), Some((0, 0, 0)));
    assert_eq!(skia.hit_test(100.0, 75.0), Some(2));
    assert_eq!(skia.hit_test(75.0, 100.0), None);
}

#[test]
fn skia_sticky_header_clamps_display_list_and_hit_testing_during_scroll() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 300.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(300.0),
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::View, None); // 100px lead-in
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(100.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.create_node(4, ComponentKind::View, None); // Section container y=100..500
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(400.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.create_node(5, ComponentKind::Pressable, None); // Sticky header h=40
    b.set_event_mask(5, EventMask::PRESS);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(40.0),
            ..Default::default()
        },
    );
    b.set_style(
        5,
        &StyleProps {
            background: ColorRgba::rgb(59, 130, 246),
            ..Default::default()
        },
    );
    b.create_node(6, ComponentKind::View, None); // Tail spacer so scroll can reach y=480
    b.set_layout(
        6,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(400.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.set_children(2, &[3, 4, 6]);
    b.set_children(4, &[5]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    skia.set_node_layout_ext(
        5,
        Some(LayoutExt {
            position: PositionMode::Sticky,
            insets: PositionInsets {
                top: Dimension::Points(12.0),
                ..PositionInsets::AUTO
            },
            ..Default::default()
        }),
    );

    let header_fill_y = |s: &SkiaBackend| {
        s.display_list()
            .ops
            .iter()
            .find_map(|op| match op {
                PaintOp::FillRoundedRect { rect, color, .. }
                    if *color == ColorRgba::rgb(59, 130, 246) =>
                {
                    Some(rect.y)
                }
                _ => None,
            })
            .unwrap()
    };

    // Before threshold (scroll_y = 40): y = 60
    skia.set_scroll_offset(2, 0.0, 40.0);
    assert_close(header_fill_y(&skia), 60.0, 0.5, "display list before");
    assert_close(skia.visual_rect_of(5).unwrap().y, 60.0, 0.5, "visual_rect before");
    assert_eq!(skia.hit_test(50.0, 75.0), Some(5));

    // Clamped while scrolling (scroll_y = 220): pinned at top: 12px
    skia.set_scroll_offset(2, 0.0, 220.0);
    assert_close(header_fill_y(&skia), 12.0, 0.5, "display list clamped");
    assert_close(skia.visual_rect_of(5).unwrap().y, 12.0, 0.5, "visual_rect clamped");
    assert_eq!(skia.hit_test(50.0, 25.0), Some(5));

    // Pushed out at container bottom (scroll_y = 480): container bottom = 20, header y = -20..20
    skia.set_scroll_offset(2, 0.0, 480.0);
    assert_close(header_fill_y(&skia), -20.0, 0.5, "display list pushed out");
    assert_close(skia.visual_rect_of(5).unwrap().y, -20.0, 0.5, "visual_rect pushed out");
    assert_eq!(skia.hit_test(50.0, 10.0), Some(5));
    assert_eq!(skia.hit_test(50.0, 30.0), None);
}

#[test]
fn skia_motion_driver_interpolates_hover_transform_and_settles_idle() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 200.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_event_mask(2, EventMask::PRESS);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(100.0),
            height: Dimension::Points(50.0),
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(34, 197, 94),
            ..Default::default()
        },
    );
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    skia.set_node_state_rules(
        2,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().translate(0.0, -20.0).scale(1.2),
        )],
    );
    skia.set_node_transitions(
        2,
        vec![TransitionSpec::new(TransitionProperty::Transform, 200).easing(EasingCurve::Linear)],
    );

    // Hover starts transform animation
    skia.pointer_move(50.0, 25.0);
    assert!(skia.needs_next_frame());

    // Advance 100ms (50% linear): translate_y = -10, scale = 1.1
    assert!(skia.advance_animations(0.1));
    let mid_tf = skia
        .motion_driver()
        .resolve_animated_transform(2, None)
        .unwrap();
    assert_close(mid_tf.translate_y, -10.0, 1e-2, "mid translate_y");
    assert_close(mid_tf.scale_x, 1.1, 1e-2, "mid scale_x");

    // Reduced motion snaps immediately and drops to idle
    skia.set_motion_preference(MotionPreference::Reduce);
    assert!(!skia.needs_next_frame());
}
