//! `MotionDriver` driving visual & transform transitions through `SkiaBackend`'s frame hook.

use nve_abi::{ColorRgba, InteractionEvent, SurfaceId};
use nve_backend_skia::{
    CursorKind, InteractionState, RowPatch, SkiaBackend, StateStyleRule, StylePatchExt,
};
use nve_tables::tables::{Interaction, Layout, StateRules, Transform, Visual};
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
    use nve_tables::snapshots::VisualSnapshot;
    use nve_tables::transform::Transform2D;

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

fn set_bg(store: &mut NodeStore, skia: &mut SkiaBackend, btn: NodeId, bg: Rgba) {
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.edit::<Visual>(btn).background(bg);
    tx.commit_into(&mut changes);
    skia.commit(store, &changes).expect("commit");
}

fn set_transitions(store: &mut NodeStore, skia: &mut SkiaBackend, btn: NodeId, specs: &[TransitionSpec]) {
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    nve_motion::write_transitions(&mut tx, btn, specs);
    tx.commit_into(&mut changes);
    skia.commit(store, &changes).expect("commit");
}

#[test]
fn zero_dt_and_in_delay_ticks_change_nothing_visible() {
    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);
    set_transitions(&mut store, &mut skia, btn, &[TransitionSpec::new(TransitionProperty::BackgroundColor, 100)
        .delay_ms(100)
        .easing(EasingCurve::Linear)]);
    set_bg(&mut store, &mut skia, btn, Rgba::rgb(255, 0, 0));
    assert!(skia.needs_next_frame());
    let before = skia.animated_style(&store, btn).expect("live").background;
    skia.advance_animations(&store, 0.0);
    skia.advance_animations(&store, 0.05);
    assert_eq!(skia.animated_style(&store, btn).expect("live").background, before, "still in delay");
    skia.advance_animations(&store, 0.1);
    assert_ne!(skia.animated_style(&store, btn).expect("live").background, before);
}

#[test]
fn detach_then_reattach_does_not_replay_a_stale_transition() {
    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);
    set_transitions(&mut store, &mut skia, btn, &[]);
    set_bg(&mut store, &mut skia, btn, Rgba::rgb(255, 0, 0));
    assert!(!skia.needs_next_frame(), "no Motion: instant");
    set_transitions(&mut store, &mut skia, btn, &[TransitionSpec::all(100).easing(EasingCurve::Linear)]);
    assert!(!skia.needs_next_frame(), "reattaching seeds the current value, it does not animate from a stale one");
    set_bg(&mut store, &mut skia, btn, Rgba::rgb(0, 255, 0));
    assert!(skia.needs_next_frame());
    skia.advance_animations(&store, 0.05);
    let mid = skia.animated_style(&store, btn).expect("live").background;
    assert!(mid.r > 0 && mid.g > 0, "animates red -> green, got {mid:?}");
}

#[test]
fn reduced_motion_keeps_transitions_that_opt_out_of_instant() {
    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);
    set_transitions(&mut store, &mut skia, btn, &[TransitionSpec::new(TransitionProperty::BackgroundColor, 200)
        .easing(EasingCurve::Linear)
        .reduced_motion_instant(false)]);
    set_bg(&mut store, &mut skia, btn, Rgba::rgb(255, 0, 0));
    skia.set_reduced_motion(&store, true);
    assert!(skia.needs_next_frame(), "opted-out transition keeps running under reduced motion");
    set_transitions(&mut store, &mut skia, btn, &[TransitionSpec::new(TransitionProperty::BackgroundColor, 200)]);
    set_bg(&mut store, &mut skia, btn, Rgba::rgb(0, 0, 255));
    assert!(!skia.needs_next_frame(), "instant transitions snap under reduced motion");
}

#[test]
fn spring_transition_overshoots_into_the_display_list() {
    use nve_backend_skia::PaintOp;
    use nve_motion::SpringSpec;

    let mut store = NodeStore::new();
    let (btn, changes) = hover_button(&mut store, true);
    let mut skia = backend(&store, &changes, true);
    set_transitions(&mut store, &mut skia, btn, &[TransitionSpec::spring(TransitionProperty::Transform, SpringSpec::bouncy())]);
    let mut changes = ChangeSet::default();
    let mut tx = store.tx();
    tx.edit::<Transform>(btn).translate_x(110.0);
    tx.commit_into(&mut changes);
    skia.commit(&store, &changes).expect("commit");
    let mut peak = f32::MIN;
    while skia.advance_animations(&store, 1.0 / 60.0) {
        for op in &skia.materialize_display_list().ops {
            if let PaintOp::PushTransform { matrix } = op {
                peak = peak.max(matrix[4]);
            }
        }
    }
    assert!(peak > 110.5, "bouncy spring overshoots the 110px target in painted output (peak {peak})");
}

#[test]
fn motion_driver_compositor_scene_composite_and_paint_oracle_parity() {
    use nve_backend_skia::raster::{rasterize_display_list, PixelBuffer};
    use nve_backend_skia::retained::SceneDirtyKind;
    use nve_tables::tables::{TextContent, Typography};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (comp_node, paint_node) = {
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(240.0))
            .height(Length::points(120.0));
        let comp = tx.create();
        tx.edit::<Layout>(comp)
            .width(Length::points(100.0))
            .height(Length::points(50.0));
        tx.edit::<Visual>(comp)
            .background(Rgba::rgb(59, 130, 246))
            .opacity(1.0);
        tx.edit::<Transform>(comp).translate_x(0.0);
        tx.edit::<Motion>(comp).transitions(Some(Transitions::new([
            TransitionSpec::new(TransitionProperty::Opacity, 200).easing(EasingCurve::Linear),
            TransitionSpec::new(TransitionProperty::Transform, 200).easing(EasingCurve::Linear),
        ])));

        let paint = tx.create();
        tx.edit::<Layout>(paint)
            .width(Length::points(100.0))
            .height(Length::points(50.0));
        tx.edit::<Visual>(paint)
            .background(Rgba::rgb(20, 40, 80))
            .border_radius(2.0);
        tx.edit::<Typography>(paint)
            .color(Rgba::rgb(255, 255, 255))
            .font_size(14.0);
        tx.edit::<TextContent>(paint).text("Anim");
        tx.edit::<Motion>(paint).transitions(Some(Transitions::new([
            TransitionSpec::new(TransitionProperty::BackgroundColor, 200)
                .easing(EasingCurve::Linear),
            TransitionSpec::new(TransitionProperty::Color, 200).easing(EasingCurve::Linear),
            TransitionSpec::new(TransitionProperty::BorderRadius, 200)
                .easing(EasingCurve::Linear),
        ])));

        tx.set_children(NodeId::ROOT, &[comp, paint]);
        tx.commit_into(&mut changes);
        (comp, paint)
    };

    let mut skia = backend(&store, &changes, true);
    let assert_oracle = |skia: &mut SkiaBackend, label: &str| {
        let clear = ColorRgba::rgb(10, 15, 25);
        let mut scene_buf = PixelBuffer::new(240, 120);
        skia.rasterize(&mut scene_buf, clear);
        let dl = skia.materialize_display_list().clone();
        let mut dl_buf = PixelBuffer::new(240, 120);
        rasterize_display_list(&dl, &mut dl_buf, 1.0, clear);
        assert_eq!(scene_buf.pixels(), dl_buf.pixels(), "oracle mismatch at {label}");
    };

    // 1. Trigger composite-only (opacity + transform) transition: ticks MUST stay CompositeOnly
    //    with 0 re-emitted paint chunks.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(comp_node).opacity(0.35);
        tx.edit::<Transform>(comp_node).translate_x(40.0);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("trigger composite");
    let emitted_before = skia.scene().stats().chunks_emitted;
    assert!(skia.advance_animations(&store, 0.08));
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly),
        "opacity+transform tick must take CompositeOnly fast path"
    );
    assert_eq!(
        skia.scene().stats().chunks_emitted,
        emitted_before,
        "composite-only motion tick must emit 0 paint chunks"
    );
    assert_oracle(&mut skia, "composite mid-flight");

    // 2. Trigger paint-channel (bg + text color + border_radius) transition: ticks update via
    //    incremental PaintOnly and match materialized oracle.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(paint_node)
            .background(Rgba::rgb(220, 80, 30))
            .border_radius(14.0);
        tx.edit::<Typography>(paint_node).color(Rgba::rgb(30, 240, 180));
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("trigger paint");
    assert!(skia.advance_animations(&store, 0.08));
    assert_oracle(&mut skia, "composite+paint mid-flight");

    // 3. Reduced motion snaps both active composite & paint transitions in-place cleanly.
    skia.set_reduced_motion(&store, true);
    assert!(!skia.needs_next_frame());
    assert_oracle(&mut skia, "reduced motion snap");
}
