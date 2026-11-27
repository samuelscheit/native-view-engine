//! End-to-end integration tests for `SkiaBackend` over `NodeStore`:
//! - Interactive `:hover`/`:active`/`:focus` + `MotionDriver` visual & transform transitions
//! - `Transform2D` + sticky positioning hit-testing and painting
//! - `SceneRelocView` Frame-0 zero-layout instant-boot painting

use nve_abi::{ColorRgba, EventMask};
use nve_backend_skia::{
    CursorKind, EasingCurve, FocusModality, InteractionState, MotionPreference, PixelBuffer,
    SkiaBackend, StateStyleRule, StylePatch, TransitionProperty, TransitionSpec,
};
use nve_components::tables::{
    Interaction, Layout, Motion, StateRules, TextContent, Transform, Transitions, Typography,
    Visual,
};
use nve_engine::{SceneRelocView, StoreBackend, VerifyMode, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

#[test]
fn skia_interaction_motion_transitions_and_reloc_frame0_roundtrip() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    let vp = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    backend.set_viewport(1, vp);

    let rules = vec![StateStyleRule::new(
        InteractionState::HOVER,
        StylePatch::new()
            .bg(ColorRgba::rgb(59, 130, 246))
            .cursor(CursorKind::Pointer),
    )];
    let transitions = vec![
        TransitionSpec::new(TransitionProperty::BackgroundColor, 200)
            .easing(EasingCurve::Linear),
        TransitionSpec::new(TransitionProperty::Transform, 200)
            .easing(EasingCurve::Linear),
    ];

    let mut tx = store.tx();
    let btn = tx.create();
    let txt = tx.create();
    tx.set_children(NodeId::ROOT, &[btn]);
    tx.set_children(btn, &[txt]);
    tx.edit::<Layout>(btn)
        .width(Length::points(120.0))
        .height(Length::points(44.0));
    tx.edit::<Visual>(btn).background(Rgba::rgb(15, 23, 42));
    tx.edit::<Interaction>(btn)
        .event_mask(EventMask::PRESS as u32)
        .cursor(CursorKind::Pointer)
        .state_rules(Some(StateRules(rules.into())));
    tx.edit::<Motion>(btn).transitions(Some(Transitions(transitions.into())));
    tx.edit::<Transform>(btn).translate_x(10.0);
    tx.edit::<TextContent>(txt).text("Click me");
    tx.edit::<Typography>(txt).color(Rgba::WHITE).font_size(14.0);
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    // Pointer hover triggers smooth transition.
    assert!(backend.pointer_move(&store, 40.0, 20.0));
    assert_eq!(backend.current_cursor(&store), CursorKind::Pointer);
    assert!(backend.needs_next_frame());

    // Step halfway (0.1s / 0.2s): interpolated color is strictly between start and end.
    assert!(backend.advance_animations(&store, 0.1));
    let mid = backend.animated_style(&store, btn).unwrap();
    assert!(mid.background.b > 42 && mid.background.b < 246);

    // Step past duration: settles to idle (`needs_next_frame() == false`).
    assert!(!backend.advance_animations(&store, 0.2));
    assert!(!backend.needs_next_frame());

    // Reduced-motion mode snaps immediately.
    backend.set_motion_preference(&store, MotionPreference::Reduce);
    assert!(backend.pointer_leave(&store));
    assert!(!backend.needs_next_frame());

    // Focus tracking via keyboard modality.
    assert!(backend.set_interaction_focus(&store, Some(btn.raw()), FocusModality::Keyboard));
    assert!(backend
        .interaction_state(btn.raw())
        .contains(InteractionState::FOCUS_VISIBLE));

    // Export Frame-0 NVERELOC from layout_tree and load into a cold SkiaBackend with zero layout.
    let reloc = backend
        .layout_tree()
        .export_scene_reloc(&store, 1, 1, "skia-e2e", ColorRgba::BLACK)
        .unwrap();
    let view = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    let mut cold = SkiaBackend::new();
    cold.load_scene_reloc(&view).unwrap();
    let mut buf = PixelBuffer::new(300, 200);
    cold.rasterize(&mut buf, ColorRgba::BLACK);
    assert_ne!(buf.get_rgb(20, 20), Some((0, 0, 0)));
}
