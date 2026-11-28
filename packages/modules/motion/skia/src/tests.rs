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
    use nve_backend_skia::raster::PixelBuffer;
    use nve_backend_skia::retained::SceneDirtyKind;
    use nve_tables::tables::{TextContent, Typography};

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (comp_node, paint_node, static_node) = {
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

        let static_node = tx.create();
        tx.edit::<Layout>(static_node)
            .width(Length::points(40.0))
            .height(Length::points(20.0));
        tx.edit::<Visual>(static_node)
            .background(Rgba::rgb(80, 90, 100));

        tx.set_children(NodeId::ROOT, &[comp, paint, static_node]);
        tx.commit_into(&mut changes);
        (comp, paint, static_node)
    };

    let mut skia = backend(&store, &changes, true);
    let clear = ColorRgba::rgb(10, 15, 25);

    // Independent ground-truth oracle: builds a fresh NodeStore + fresh SkiaBackend from scratch
    // (no MotionDriver, no shared retained CompositorScene) with the exact animated overrides
    // written as static rows, and compares full pixel buffers against `skia.rasterize()`.
    let assert_ground_truth_oracle = |skia: &SkiaBackend, store: &NodeStore, label: &str| {
        let mut scene_buf = PixelBuffer::new(240, 120);
        skia.rasterize(&mut scene_buf, clear);

        let comp_vis = skia.animated_style(store, comp_node).expect("comp visual");
        let comp_tf = skia.animated_transform(store, comp_node).expect("comp tf");
        let paint_vis = skia.animated_style(store, paint_node).expect("paint visual");
        let static_row = store.row::<Visual>(static_node);
        let (static_bg, static_opacity) = (static_row.background(), static_row.opacity());

        let mut oracle_store = NodeStore::new();
        let mut oracle_changes = ChangeSet::default();
        {
            let mut tx = oracle_store.tx();
            tx.edit::<Layout>(NodeId::ROOT)
                .width(Length::points(240.0))
                .height(Length::points(120.0));

            let o_comp = tx.create();
            tx.edit::<Layout>(o_comp)
                .width(Length::points(100.0))
                .height(Length::points(50.0));
            tx.edit::<Visual>(o_comp)
                .background(Rgba(comp_vis.background.to_u32()))
                .opacity(comp_vis.opacity);
            tx.edit::<Transform>(o_comp)
                .translate_x(comp_tf.translate_x)
                .translate_y(comp_tf.translate_y)
                .scale_x(comp_tf.scale_x)
                .scale_y(comp_tf.scale_y)
                .rotate_deg(comp_tf.rotate_deg);

            let o_paint = tx.create();
            tx.edit::<Layout>(o_paint)
                .width(Length::points(100.0))
                .height(Length::points(50.0));
            tx.edit::<Visual>(o_paint)
                .background(Rgba(paint_vis.background.to_u32()))
                .border_radius(paint_vis.border_radius);
            tx.edit::<Typography>(o_paint)
                .color(Rgba(paint_vis.text_color.to_u32()))
                .font_size(14.0);
            tx.edit::<TextContent>(o_paint).text("Anim");

            let o_static = tx.create();
            tx.edit::<Layout>(o_static)
                .width(Length::points(40.0))
                .height(Length::points(20.0));
            tx.edit::<Visual>(o_static)
                .background(static_bg)
                .opacity(static_opacity);

            tx.set_children(NodeId::ROOT, &[o_comp, o_paint, o_static]);
            tx.commit_into(&mut oracle_changes);
        }
        let oracle_backend = backend(&oracle_store, &oracle_changes, false);
        let mut oracle_buf = PixelBuffer::new(240, 120);
        oracle_backend.rasterize(&mut oracle_buf, clear);

        assert_eq!(
            scene_buf.pixels(),
            oracle_buf.pixels(),
            "ground-truth fresh-store oracle mismatch at {label}"
        );
        scene_buf
    };

    let initial_buf = assert_ground_truth_oracle(&skia, &store, "initial");

    // 1. Trigger composite-only (opacity + transform) transition, THEN perform a PaintOnly commit
    //    on `static_node` so `last_dirty_kind` is strictly `PaintOnly` right before the composite
    //    animation tick (making the `CompositeOnly` assertion genuinely falsifiable!).
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(comp_node).opacity(0.35);
        tx.edit::<Transform>(comp_node).translate_x(40.0);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("trigger composite");
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(static_node)
            .background(Rgba::rgb(90, 100, 110));
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("preceding paint commit");
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly),
        "preceding commit must set last_dirty_kind=PaintOnly before testing CompositeOnly tick"
    );

    let emitted_before = skia.scene().stats().chunks_emitted;
    assert!(skia.advance_animations(&store, 0.08));
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly),
        "opacity+transform tick must update last_dirty_kind to CompositeOnly"
    );
    assert_eq!(
        skia.scene().stats().chunks_emitted,
        emitted_before,
        "composite-only motion tick must emit 0 paint chunks"
    );
    let mid_comp_vis = skia.animated_style(&store, comp_node).unwrap();
    let mid_comp_tf = skia.animated_transform(&store, comp_node).unwrap();
    assert!(
        (mid_comp_vis.opacity - 0.74).abs() < 1e-3 && (mid_comp_tf.translate_x - 16.0).abs() < 1e-2,
        "analytic 40% linear midpoint expected (op=0.74, tx=16.0), got ({}, {})",
        mid_comp_vis.opacity,
        mid_comp_tf.translate_x
    );
    let mid_comp_buf = assert_ground_truth_oracle(&skia, &store, "composite mid-flight");
    assert_ne!(
        mid_comp_buf.pixels(),
        initial_buf.pixels(),
        "mid-flight composite tick must change rendered pixels"
    );

    // 2. Trigger paint-channel (bg + text color + border_radius) transition, precede tick with a
    //    CompositeOnly commit on `static_node` so the PaintOnly tick assertion is falsifiable.
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(paint_node)
            .background(Rgba::rgb(220, 80, 30))
            .border_radius(14.0);
        tx.edit::<Typography>(paint_node).color(Rgba::rgb(30, 240, 180));
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("trigger paint");
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(static_node).opacity(0.9);
        tx.commit_into(&mut changes);
    }
    skia.commit(&store, &changes).expect("preceding composite commit");
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::CompositeOnly)
    );

    assert!(skia.advance_animations(&store, 0.08));
    assert_eq!(
        skia.scene().stats().last_dirty_kind,
        Some(SceneDirtyKind::PaintOnly),
        "paint+composite tick must update last_dirty_kind to PaintOnly"
    );
    let mid_paint_buf = assert_ground_truth_oracle(&skia, &store, "composite+paint mid-flight");
    assert_ne!(mid_paint_buf.pixels(), mid_comp_buf.pixels());

    // 3. Reduced motion snaps both active composite & paint transitions to store targets cleanly:
    //    verify against a brand-new unmodified `SkiaBackend` built directly from `store`!
    skia.set_reduced_motion(&store, true);
    assert!(!skia.needs_next_frame());
    let snapped_buf = assert_ground_truth_oracle(&skia, &store, "reduced motion snap");
    let mut fresh_changes = ChangeSet::default();
    let fresh_store_backend = backend(&store, &fresh_changes, false);
    let _ = &mut fresh_changes;
    let mut direct_buf = PixelBuffer::new(240, 120);
    fresh_store_backend.rasterize(&mut direct_buf, clear);
    assert_eq!(
        snapped_buf.pixels(),
        direct_buf.pixels(),
        "reduced-motion snap must match fresh unmodified store rasterization pixel-for-pixel"
    );
}

fn build_redefine_resource_frame<R: nve_store::FlatResource>(
    rid: nve_store::ResourceId,
    value: &R,
) -> Vec<u8> {
    use nve_store::wire::{Op, MAGIC, VERSION};
    let mut payload_words = Vec::new();
    value.write_words(&mut payload_words);
    let byte_len = (payload_words.len() * 4) as u32;
    let mut body = Vec::with_capacity(24 + payload_words.len() * 4);
    body.extend_from_slice(&rid.0.to_le_bytes());
    body.extend_from_slice(&(R::KIND.0 as u32).to_le_bytes());
    body.extend_from_slice(&((R::KIND.0 >> 32) as u32).to_le_bytes());
    body.extend_from_slice(&1u32.to_le_bytes()); // ephemeral = true
    body.extend_from_slice(&0u32.to_le_bytes()); // child_count = 0
    body.extend_from_slice(&byte_len.to_le_bytes());
    for w in &payload_words {
        body.extend_from_slice(&w.to_le_bytes());
    }
    let mut frame = Vec::with_capacity(32 + body.len());
    frame.extend_from_slice(&MAGIC.to_le_bytes());
    frame.extend_from_slice(&VERSION.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&1u64.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    let total_words = (6 + 2 + body.len() / 4) as u32;
    frame.extend_from_slice(&total_words.to_le_bytes());
    let head = (Op::DefineResources as u32) | (1u32 << 16);
    frame.extend_from_slice(&head.to_le_bytes());
    frame.extend_from_slice(&((body.len() / 4) as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

#[test]
fn in_place_redefine_state_rules_and_transitions_triggers_live_transition() {
    use nve_backend_skia::FocusModality;
    use nve_store::wire::WireDecoder;

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let btn = {
        let mut tx = store.tx();
        let b = tx.create();
        tx.set_children(NodeId::ROOT, &[b]);
        tx.edit::<Layout>(b)
            .width(Length::points(120.0))
            .height(Length::points(44.0));
        tx.edit::<Visual>(b)
            .background(Rgba::rgb(15, 23, 42))
            .opacity(1.0);
        tx.edit::<Interaction>(b)
            .focusable(true)
            .state_rules_ephemeral(StateRules::new(vec![
                StateStyleRule::new(
                    InteractionState::HOVER,
                    RowPatch::new().bg(ColorRgba::rgb(20, 120, 240)),
                ),
                StateStyleRule::new(
                    InteractionState::FOCUS,
                    RowPatch::new().bg(ColorRgba::rgb(40, 180, 100)),
                ),
            ]));
        // Start with 0ms instant transition so we also prove in-place `Transitions` redefinition
        // takes effect immediately alongside `StateRules`.
        tx.edit::<Motion>(b)
            .transitions_ephemeral(Transitions::new([TransitionSpec::new(
                TransitionProperty::BackgroundColor,
                0,
            )]));
        tx.commit_into(&mut changes);
        b
    };

    let mut skia = backend(&store, &changes, true);
    // Hover button and settle (0ms -> instant).
    assert!(skia.pointer_move(&store, 40.0, 20.0));
    assert!(!skia.needs_next_frame());
    assert_eq!(
        skia.animated_style(&store, btn).unwrap().background,
        ColorRgba::rgb(20, 120, 240)
    );

    // 1. Redefine BOTH `Transitions` (200ms linear bg+opacity) and `StateRules` (:hover bg +
    //    opacity) in-place via `Op::DefineResources` with ZERO created/touched nodes!
    let rules_rid = store.row::<Interaction>(btn).state_rules();
    let trans_rid = store.row::<Motion>(btn).transitions();
    let new_rules = StateRules::new(vec![
        StateStyleRule::new(
            InteractionState::HOVER,
            RowPatch::new()
                .bg(ColorRgba::rgb(240, 40, 40))
                .opacity(0.4),
        ),
        StateStyleRule::new(
            InteractionState::FOCUS,
            RowPatch::new().bg(ColorRgba::rgb(40, 180, 100)),
        ),
    ]);
    let new_trans = Transitions::new([
        TransitionSpec::new(TransitionProperty::BackgroundColor, 200).easing(EasingCurve::Linear),
        TransitionSpec::new(TransitionProperty::Opacity, 200).easing(EasingCurve::Linear),
    ]);
    let f_rules = build_redefine_resource_frame(rules_rid, &new_rules);
    let f_trans = build_redefine_resource_frame(trans_rid, &new_trans);
    let mut dec = WireDecoder::default();
    {
        let mut tx = store.tx();
        dec.apply(&mut tx, &f_trans).expect("redefine Transitions");
        dec.apply(&mut tx, &f_rules).expect("redefine StateRules");
        tx.commit_into(&mut changes);
    }
    assert!(changes.created.is_empty() && changes.touched.is_empty());
    assert!(changes.resources_defined.len() >= 2);

    skia.commit(&store, &changes).expect("commit in-place redef");
    assert!(
        skia.needs_next_frame(),
        "in-place StateRules+Transitions redefinition on hovered node must start live transition"
    );
    assert!(skia.advance_animations(&store, 0.1));
    let mid = skia.animated_style(&store, btn).unwrap();
    assert!(
        mid.background.r > 100 && mid.background.r < 160 && mid.opacity > 0.6 && mid.opacity < 0.8,
        "mid-flight color/opacity expected near (130,80,140) op=0.7, got {:?} op={}",
        mid.background,
        mid.opacity
    );
    assert!(!skia.advance_animations(&store, 0.15));
    assert!(!skia.needs_next_frame());
    let end = skia.animated_style(&store, btn).unwrap();
    assert_eq!(end.background, ColorRgba::rgb(240, 40, 40));
    assert!((end.opacity - 0.4).abs() < 1e-3);

    // 2. Focus node (leave hover first) and redefine ONLY `StateRules` in-place -> live transition!
    assert!(skia.pointer_leave(&store));
    let _ = skia.advance_animations(&store, 0.3);
    assert!(skia.set_interaction_focus(&store, Some(btn), FocusModality::Keyboard));
    let _ = skia.advance_animations(&store, 0.3);
    assert!(!skia.needs_next_frame());
    assert_eq!(
        skia.animated_style(&store, btn).unwrap().background,
        ColorRgba::rgb(40, 180, 100)
    );

    let focus_rules_v2 = StateRules::new(vec![StateStyleRule::new(
        InteractionState::FOCUS,
        RowPatch::new().bg(ColorRgba::rgb(200, 20, 220)),
    )]);
    let f_focus = build_redefine_resource_frame(rules_rid, &focus_rules_v2);
    {
        let mut tx = store.tx();
        dec.apply(&mut tx, &f_focus).expect("redefine focus StateRules");
        tx.commit_into(&mut changes);
    }
    assert!(changes.created.is_empty() && changes.touched.is_empty());
    skia.commit(&store, &changes).expect("commit focus redef");
    assert!(
        skia.needs_next_frame(),
        "in-place StateRules redefinition on focused node must start live transition"
    );
    assert!(skia.advance_animations(&store, 0.1));
    let mid_f = skia.animated_style(&store, btn).unwrap().background;
    assert!(
        mid_f.r > 80 && mid_f.r < 160 && mid_f.b > 130 && mid_f.b < 190,
        "focused mid-flight bg expected between (40,180,100) and (200,20,220), got {mid_f:?}"
    );
}
