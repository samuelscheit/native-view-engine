//! Integration tests for [`crate::MotionDriver`] inside [`crate::SkiaBackend`]:
//! smooth hover/active color, opacity, and border-radius interpolation, mid-flight reversal,
//! `MotionPreference::Reduce` instant snapping, and zero-idle `needs_next_frame() == false`.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, LayoutProps, StyleProps,
    ROOT_NODE_ID,
};
use nve_components::interaction::{InteractionState, StateStyleRule};
use nve_components::motion::{EasingCurve, MotionPreference, TransitionProperty, TransitionSpec};
use nve_components::style_patch::StylePatch;
use nve_engine::{RenderBackend, Viewport};

use crate::{PaintOp, SkiaBackend};

fn build_hover_button_frame() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(10, ComponentKind::Pressable, None);
    b.set_children(ROOT_NODE_ID, &[10]);
    b.set_layout(
        10,
        &LayoutProps {
            width: Dimension::Points(120.0),
            height: Dimension::Points(44.0),
            ..LayoutProps::default()
        },
    );
    b.set_style(
        10,
        &StyleProps {
            background: ColorRgba::rgb(0, 0, 0),
            border_radius: 4.0,
            opacity: 0.5,
            ..StyleProps::default()
        },
    );
    b.commit(true);
    b.finish().to_vec()
}

fn first_fill_rect(backend: &SkiaBackend) -> (ColorRgba, f32) {
    backend
        .display_list()
        .ops
        .iter()
        .find_map(|op| match *op {
            PaintOp::FillRoundedRect { color, radius, .. } => Some((color, radius)),
            _ => None,
        })
        .expect("expected FillRoundedRect in display list")
}

fn first_opacity(backend: &SkiaBackend) -> Option<f32> {
    backend.display_list().ops.iter().find_map(|op| match *op {
        PaintOp::SaveOpacity(a) => Some(a),
        _ => None,
    })
}

#[test]
fn skia_hover_transition_interpolates_intermediate_color_radius_opacity_and_settles_cleanly() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    skia.submit_frame(&build_hover_button_frame()).unwrap();
    skia.set_node_state_rules(
        10,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new()
                .bg(ColorRgba::rgb(200, 100, 50))
                .radius(12.0)
                .opacity(1.0),
        )],
    );
    skia.set_node_transitions(
        10,
        vec![TransitionSpec::all(200).easing(EasingCurve::Linear)],
    );

    assert!(!skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia), (ColorRgba::rgb(0, 0, 0), 4.0));
    assert!((first_opacity(&skia).unwrap() - 0.5).abs() < 1e-4);

    // Trigger hover at (50, 20): animation starts cleanly from base style without jumping.
    let dirty = skia.pointer_move(50.0, 20.0);
    assert_eq!(dirty, vec![10]);
    assert!(skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia), (ColorRgba::rgb(0, 0, 0), 4.0));

    // Advance halfway (100ms of 200ms linear): exact midpoint color, radius, and opacity!
    assert!(skia.advance_animations(0.100));
    let (mid_color, mid_radius) = first_fill_rect(&skia);
    assert_eq!(mid_color, ColorRgba::rgb(100, 50, 25));
    assert!((mid_radius - 8.0).abs() < 1e-3);
    assert!((first_opacity(&skia).unwrap() - 0.75).abs() < 1e-3);

    // Advance to completion (another 100ms): exact hover target and zero-idle `needs_next_frame() == false`.
    assert!(!skia.advance_animations(0.100));
    assert!(!skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia), (ColorRgba::rgb(200, 100, 50), 12.0));
    assert!(first_opacity(&skia).is_none()); // opacity == 1.0 skips SaveOpacity
}

#[test]
fn skia_mid_flight_hover_reversal_starts_from_live_interpolated_color() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    skia.submit_frame(&build_hover_button_frame()).unwrap();
    skia.set_node_state_rules(
        10,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(200, 0, 0)),
        )],
    );
    skia.set_node_transitions(
        10,
        vec![
            TransitionSpec::new(TransitionProperty::BackgroundColor, 200)
                .easing(EasingCurve::Linear),
        ],
    );

    // Hover in, step 50% -> rgb(100, 0, 0).
    skia.pointer_move(50.0, 20.0);
    skia.advance_animations(0.100);
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(100, 0, 0));

    // Pointer leaves mid-flight: must NOT snap back to rgb(200, 0, 0) or rgb(0, 0, 0)!
    skia.pointer_leave();
    assert!(skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(100, 0, 0));

    // Step 50% back -> rgb(50, 0, 0), then finish -> rgb(0, 0, 0) & settled.
    assert!(skia.advance_animations(0.100));
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(50, 0, 0));
    assert!(!skia.advance_animations(0.100));
    assert!(!skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(0, 0, 0));
}

#[test]
fn skia_reduced_motion_snaps_immediately_without_scheduling_extra_frames() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    skia.set_motion_preference(MotionPreference::Reduce);
    skia.submit_frame(&build_hover_button_frame()).unwrap();
    skia.set_node_state_rules(
        10,
        vec![StateStyleRule::new(
            InteractionState::HOVER,
            StylePatch::new().bg(ColorRgba::rgb(59, 130, 246)).opacity(1.0),
        )],
    );
    skia.set_node_transitions(
        10,
        vec![TransitionSpec::all(250).easing(EasingCurve::EaseOut)],
    );

    skia.pointer_move(50.0, 20.0);
    assert!(!skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(59, 130, 246));
    assert!(first_opacity(&skia).is_none());

    // Switching from Full to Reduce mid-flight snaps active animations immediately too.
    skia.set_motion_preference(MotionPreference::Full);
    skia.pointer_leave();
    assert!(skia.needs_next_frame());
    skia.set_motion_preference(MotionPreference::Reduce);
    assert!(!skia.needs_next_frame());
    assert_eq!(first_fill_rect(&skia).0, ColorRgba::rgb(0, 0, 0));
}
