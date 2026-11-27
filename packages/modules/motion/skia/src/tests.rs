//! `MotionDriver` driving visual & transform transitions through `SkiaBackend`'s frame hook.

use nve_abi::{ColorRgba, InteractionEvent, SurfaceId};
use nve_backend_skia::{
    CursorKind, InteractionState, RowPatch, SkiaBackend, StateStyleRule, StylePatchExt,
};
use nve_components::tables::{Interaction, Layout, StateRules, Transform, Visual};
use nve_engine::{StoreBackend, Viewport};
use nve_motion::{EasingCurve, Motion, TransitionProperty, TransitionSpec, Transitions};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

use crate::MotionDriver;

fn hover_button(store: &mut NodeStore, with_motion: bool) -> (NodeId, ChangeSet) {
    let rules = vec![StateStyleRule::new(
        InteractionState::HOVER,
        RowPatch::new().bg(ColorRgba::rgb(59, 130, 246)).cursor(CursorKind::Pointer),
    )];
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    let btn = tx.create();
    tx.set_children(NodeId::ROOT, &[btn]);
    tx.edit::<Layout>(btn).width(Length::points(120.0)).height(Length::points(44.0));
    tx.edit::<Visual>(btn).background(Rgba::rgb(15, 23, 42));
    tx.edit::<Interaction>(btn)
        .listen(InteractionEvent::Press)
        .state_rules(Some(StateRules(rules.into())));
    tx.edit::<Transform>(btn).translate_x(10.0);
    if with_motion {
        tx.edit::<Motion>(btn).transitions(Some(Transitions::new([
            TransitionSpec::new(TransitionProperty::BackgroundColor, 200).easing(EasingCurve::Linear),
            TransitionSpec::new(TransitionProperty::Transform, 200).easing(EasingCurve::Linear),
        ])));
    }
    tx.commit_into(&mut changes);
    (btn, changes)
}

fn backend(store: &NodeStore, changes: &ChangeSet, hook: bool) -> SkiaBackend {
    let mut skia = SkiaBackend::new();
    if hook {
        skia.register_frame_hook(MotionDriver::new());
    }
    skia.set_viewport(SurfaceId::ROOT, Viewport { width: 300.0, height: 200.0, scale_factor: 1.0 });
    skia.commit(store, changes).expect("commit");
    skia
}

#[test]
fn hover_transition_interpolates_settles_and_snaps_under_reduced_motion() {
    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);

    assert!(skia.pointer_move(&store, 40.0, 20.0));
    assert!(skia.needs_next_frame());
    assert!(skia.advance_animations(&store, 0.1));
    let mid = skia.animated_style(&store, btn).expect("live");
    assert!(mid.background.b > 42 && mid.background.b < 246, "mid-flight color");
    assert!(!skia.advance_animations(&store, 0.2));
    assert!(!skia.needs_next_frame(), "settled: display link may sleep");
    assert_eq!(skia.animated_style(&store, btn).expect("live").background, ColorRgba::rgb(59, 130, 246));

    skia.set_reduced_motion(&store, true);
    assert!(skia.pointer_leave(&store));
    assert!(!skia.needs_next_frame(), "reduced motion snaps instantly");
}

#[test]
fn transform_change_animates_from_painted_value() {
    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.edit::<Transform>(btn).translate_x(110.0);
    tx.commit_into(&mut changes);
    skia.commit(&store, &changes).expect("commit");
    assert!(skia.needs_next_frame());
    assert!(skia.advance_animations(&store, 0.1));
    assert!(!skia.advance_animations(&store, 0.2));
    assert!(!skia.needs_next_frame());
}

#[test]
fn driver_interpolates_transform_from_previous_target() {
    use nve_backend_skia::{NodeTargets, SkiaFrameHook};
    use nve_components::snapshots::VisualSnapshot;
    use nve_components::transform::Transform2D;

    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut driver = MotionDriver::new();
    let at = |x: f32| move |_| NodeTargets {
        visual: VisualSnapshot::default(),
        transform: Transform2D::translate(x, 0.0),
    };
    driver.after_commit(&store, &changes, &at(10.0));
    assert!(!driver.needs_next_frame(), "first sighting seeds the baseline only");
    driver.sync_nodes(&store, &[btn], &at(110.0));
    assert!(driver.needs_next_frame());
    driver.advance(0.1);
    let mid = driver.resolve_transform(btn, None).expect("animating");
    assert!((mid.translate_x - 60.0).abs() < 1e-3, "linear midpoint, got {}", mid.translate_x);
    // Retargeting mid-flight starts from the painted value, not the old target.
    driver.sync_nodes(&store, &[btn], &at(0.0));
    let restart = driver.resolve_transform(btn, None).expect("animating");
    assert!((restart.translate_x - 60.0).abs() < 1e-3);
    driver.advance(0.25);
    assert_eq!(driver.resolve_transform(btn, Some(Transform2D::translate(0.0, 0.0))), Some(Transform2D::translate(0.0, 0.0)));
    driver.reset();
    assert!(!driver.needs_next_frame());
}

#[test]
fn without_the_hook_or_motion_table_nothing_animates() {
    let mut store = NodeStore::new();
    let (_, changes) = hover_button(&mut store, true);
    let mut bare = backend(&store, &changes, false);
    assert!(bare.pointer_move(&store, 40.0, 20.0));
    assert!(!bare.needs_next_frame(), "core skia has no motion of its own");

    let mut store = NodeStore::new();
    let (_, changes) = hover_button(&mut store, false);
    let mut hooked = backend(&store, &changes, true);
    assert!(hooked.pointer_move(&store, 40.0, 20.0));
    assert!(!hooked.needs_next_frame(), "nodes without `Motion` snap");
}
