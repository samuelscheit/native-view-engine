//! End-to-end integration tests for `SkiaBackend` over `NodeStore`:
//! - Interactive `:hover`/`:active`/`:focus` state styling and cursor resolution
//! - `Transform2D` + sticky positioning hit-testing and painting
//! - `SceneRelocView` Frame-0 zero-layout instant-boot painting

use nve_abi::{ColorRgba, FrameId, InteractionEvent, SurfaceId};
use nve_backend_skia::{
    CursorKind, FocusModality, InteractionState, PixelBuffer, RowPatch, SkiaBackend,
    StateStyleRule, StylePatchExt,
};
use nve_tables::tables::{
    Display, InputKind, Interaction, Layout, StateRules, TextContent, TextInput, Transform,
    Typography, Visual,
};
use nve_engine::{SceneRelocView, StoreBackend, VerifyMode, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

#[test]
fn skia_interaction_state_focus_and_reloc_frame0_roundtrip() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    let vp = Viewport {
        width: 300.0,
        height: 200.0,
        scale_factor: 1.0,
    };
    backend.set_viewport(SurfaceId::ROOT, vp);

    let rules = vec![StateStyleRule::new(
        InteractionState::HOVER,
        RowPatch::new()
            .bg(ColorRgba::rgb(59, 130, 246))
            .cursor(CursorKind::Pointer),
    )];

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
        .listen(InteractionEvent::Press)
        .cursor(CursorKind::Pointer)
        .state_rules(Some(StateRules(rules.into())));
    tx.edit::<Transform>(btn).translate_x(10.0);
    tx.edit::<TextContent>(txt).text("Click me");
    tx.edit::<Typography>(txt).color(Rgba::WHITE).font_size(14.0);
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    // Pointer hover applies the state rule instantly (no animation without a frame hook).
    assert!(backend.pointer_move(&store, 40.0, 20.0));
    assert_eq!(backend.current_cursor(&store), CursorKind::Pointer);
    assert_eq!(
        backend.animated_style(&store, btn).unwrap().background,
        ColorRgba::rgb(59, 130, 246)
    );
    assert!(!backend.needs_next_frame());
    assert!(backend.pointer_leave(&store));

    // Focus tracking via keyboard modality.
    assert!(backend.set_interaction_focus(&store, Some(btn), FocusModality::Keyboard));
    assert!(backend
        .interaction_state(btn)
        .contains(InteractionState::FOCUS_VISIBLE));

    // Export Frame-0 NVERELOC from layout_tree and load into a cold SkiaBackend with zero layout.
    let reloc = backend
        .layout_tree()
        .export_scene_reloc(&store, 1, FrameId(1), "skia-e2e", ColorRgba::BLACK)
        .unwrap();
    let view = SceneRelocView::from_bytes(&reloc, VerifyMode::Full).unwrap();
    let mut cold = SkiaBackend::new();
    cold.load_scene_reloc(&view).unwrap();
    let mut buf = PixelBuffer::new(300, 200);
    cold.rasterize(&mut buf, ColorRgba::BLACK);
    assert_ne!(buf.get_rgb(20, 20), Some((0, 0, 0)));
}

#[test]
fn hidden_text_input_has_zero_layout_zero_paint_ops_no_hit_and_refuses_focus() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let hidden = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<TextInput>(n)
            .input_kind(InputKind::Hidden)
            .value("secret");
        tx.edit::<Visual>(n).background(Rgba::rgb(255, 0, 0));
        tx.edit::<Interaction>(n)
            .listen(InteractionEvent::Press)
            .listen(InteractionEvent::Focus)
            .listen(InteractionEvent::Blur);
        tx.commit_into(&mut changes);
        n
    };
    backend.commit(&store, &changes).unwrap();

    let rect = backend.layout_of(hidden).expect("hidden input layout entry");
    assert_eq!((rect.width, rect.height), (0.0, 0.0));
    assert!(
        backend.display_list().ops.is_empty(),
        "InputKind::Hidden must emit 0 paint ops, got {:?}",
        backend.display_list().ops
    );
    assert_eq!(backend.hit_test(&store, 1.0, 1.0), None);
    assert_eq!(backend.hit_test_interaction(&store, 1.0, 1.0), None);
    assert!(!backend.is_text_input(&store, hidden));
    backend.set_focused_node(&store, Some(hidden));
    assert_eq!(backend.focused_node(), None);

    // Mutating an already-focused visible input to Hidden automatically clears focus and paint ops.
    let visible = {
        let mut tx = store.tx();
        let v = tx.create();
        tx.set_children(NodeId::ROOT, &[hidden, v]);
        tx.edit::<TextInput>(v)
            .input_kind(InputKind::Text)
            .value("visible");
        tx.edit::<Layout>(v)
            .width(Length::points(180.0))
            .height(Length::points(36.0));
        tx.commit_into(&mut changes);
        v
    };
    backend.commit(&store, &changes).unwrap();
    assert!(backend.is_text_input(&store, visible));
    backend.set_focused_node(&store, Some(visible));
    assert_eq!(backend.focused_node(), Some(visible));
    assert!(!backend.display_list().ops.is_empty());

    {
        let mut tx = store.tx();
        tx.edit::<TextInput>(visible).input_kind(InputKind::Hidden);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).unwrap();
    assert_eq!(backend.focused_node(), None);
    assert!(!backend.is_text_input(&store, visible));
    assert!(backend.display_list().ops.is_empty());
    let vr = backend.layout_of(visible).expect("now-hidden layout");
    assert_eq!((vr.width, vr.height), (0.0, 0.0));
}

#[test]
fn ancestor_display_none_clears_and_rejects_child_text_input_focus() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );

    let (container, child) = {
        let mut tx = store.tx();
        let parent = tx.create();
        let input = tx.create();
        tx.set_children(NodeId::ROOT, &[parent]);
        tx.set_children(parent, &[input]);
        tx.edit::<Layout>(parent)
            .width(Length::points(220.0))
            .height(Length::points(60.0));
        tx.edit::<TextInput>(input)
            .input_kind(InputKind::Text)
            .value("nested");
        tx.edit::<Layout>(input)
            .width(Length::points(180.0))
            .height(Length::points(36.0));
        tx.commit_into(&mut changes);
        (parent, input)
    };
    backend.commit(&store, &changes).unwrap();

    backend.set_focused_node(&store, Some(child));
    assert_eq!(backend.focused_node(), Some(child));

    // Hiding the ancestor clears focus on the focused child TextInput during commit.
    {
        let mut tx = store.tx();
        tx.edit::<Layout>(container).display(Display::None);
        tx.commit_into(&mut changes);
    }
    backend.commit(&store, &changes).unwrap();
    assert_eq!(
        backend.focused_node(),
        None,
        "hiding an ancestor with Display::None must clear focus on child TextInput"
    );

    // Direct set_focused_node on a child under a Display::None ancestor is also rejected.
    backend.set_focused_node(&store, Some(child));
    assert_eq!(
        backend.focused_node(),
        None,
        "set_focused_node must reject focusing a child under a Display::None parent"
    );
}

#[test]
fn skia_hover_solid_bg_and_explicit_none_clear_base_gradient_and_hostile_40_table_wire_rules_do_not_panic() {
    use nve_backend_skia::PaintOp;
    use nve_tables::paint::{BackgroundFill, GradientStop};
    use nve_tables::tables::BackgroundFillRes;
    use nve_store::wire::{WireDecoder, WireEncoder};
    use nve_store::{FlatResource, ResourceId};

    let mut prod = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (n_solid_clear, n_none_clear, n_nested_grad) = {
        let mut tx = prod.tx();
        let a = tx.create();
        let b = tx.create();
        let c = tx.create();
        tx.set_children(NodeId::ROOT, &[a, b, c]);

        let base_grad = BackgroundFillRes(BackgroundFill::linear(
            90.0,
            vec![
                GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)),
                GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255)),
            ],
        ));
        for &n in &[a, b, c] {
            tx.edit::<Layout>(n)
                .width(Length::points(100.0))
                .height(Length::points(40.0));
            tx.edit::<Visual>(n)
                .background(Rgba::rgb(10, 20, 30))
                .background_fill(Some(base_grad.clone()));
        }

        // a: hover solid `.bg(...)` clears base gradient.
        tx.edit::<Interaction>(a).state_rules(Some(StateRules::new(vec![
            StateStyleRule::new(
                InteractionState::HOVER,
                RowPatch::new().bg(ColorRgba::rgb(0, 200, 0)),
            ),
        ])));
        // b: explicit `Visual::BACKGROUND_FILL = ResourceId::NONE` clears base gradient.
        tx.edit::<Interaction>(b).state_rules(Some(StateRules::new(vec![
            StateStyleRule::new(
                InteractionState::HOVER,
                RowPatch::new().background_fill_id(ResourceId::NONE),
            ),
        ])));
        // c: nested gradient inside `StateRules` replaces base gradient across NVE2 wire!
        tx.with_temp_resources(|scope| {
            let hover_grad_id = scope.intern(&BackgroundFillRes(BackgroundFill::linear(
                180.0,
                vec![
                    GradientStop::new(0.0, ColorRgba::rgb(0, 255, 0)),
                    GradientStop::new(1.0, ColorRgba::rgb(255, 255, 0)),
                ],
            )));
            scope.tx().edit::<Interaction>(c).state_rules(Some(StateRules::new(vec![
                StateStyleRule::new(
                    InteractionState::HOVER,
                    RowPatch::new().background_fill_id(hover_grad_id),
                ),
            ])));
        });
        tx.commit_into(&mut changes);
        (a, b, c)
    };

    // Round-trip through NVE2 wire into replica SkiaBackend.
    let mut wire = Vec::new();
    WireEncoder::default().encode(&prod, &changes, &mut wire);
    let mut replica = NodeStore::replica();
    let mut rep_changes = ChangeSet::default();
    {
        let mut rtx = replica.tx();
        WireDecoder::default().apply(&mut rtx, &wire).unwrap();
        rtx.commit_into(&mut rep_changes);
    }

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        },
    );
    skia.commit(&replica, &rep_changes).unwrap();
    // Before hover: all 3 nodes paint gradients.
    let grad_count = |ops: &[PaintOp]| {
        ops.iter()
            .filter(|op| matches!(op, PaintOp::FillRoundedRectGradient { .. }))
            .count()
    };
    assert_eq!(grad_count(&skia.display_list().ops), 3);

    // Hover node `a` (solid `.bg(GREEN)` override) -> gradient cleared for `a` (only 2 gradients remain).
    let rect_a = skia.layout_of(n_solid_clear).unwrap();
    skia.pointer_move(&replica, rect_a.x + 5.0, rect_a.y + 5.0);
    assert_eq!(
        grad_count(&skia.display_list().ops),
        2,
        "hover solid bg override must clear base gradient"
    );
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRect { color, .. } if *color == ColorRgba::rgb(0, 200, 0)
    )));

    // Hover node `b` (explicit `BACKGROUND_FILL = NONE`) -> gradient cleared for `b` (only 2 gradients remain).
    let rect_b = skia.layout_of(n_none_clear).unwrap();
    skia.pointer_move(&replica, rect_b.x + 5.0, rect_b.y + 5.0);
    assert_eq!(
        grad_count(&skia.display_list().ops),
        2,
        "explicit ResourceId::NONE background_fill override must clear base gradient"
    );

    // Hover node `c` (nested wire gradient) -> gradient angle switches to 180deg.
    let rect_c = skia.layout_of(n_nested_grad).unwrap();
    skia.pointer_move(&replica, rect_c.x + 5.0, rect_c.y + 5.0);
    assert!(skia.display_list().ops.iter().any(|op| matches!(
        op,
        PaintOp::FillRoundedRectGradient {
            gradient: BackgroundFill::Linear(lg),
            ..
        } if (lg.angle_deg - 180.0).abs() < 1e-3
    )));

    // Hostile 2x20-table valid wire StateRules (40 distinct tables > MAX_PATCH_TABLES):
    // Skia + Headless must NEVER panic when hovering and rendering!
    let make_20_patch = |base_key: u64| -> Vec<u32> {
        let mut w = vec![20u32];
        for i in 0..20u64 {
            let k = nve_abi::Key64(base_key + i);
            w.extend_from_slice(&[k.lo(), k.hi(), 1, 0, u32::MAX, i as u32]);
        }
        w
    };
    let p1 = make_20_patch(0x5000);
    let p2 = make_20_patch(0x6000);
    let mut sr_words = vec![2u32];
    sr_words.push(u32::from(InteractionState::HOVER.bits()));
    sr_words.push(p1.len() as u32);
    sr_words.extend_from_slice(&p1);
    sr_words.push(u32::from(InteractionState::HOVER.bits()));
    sr_words.push(p2.len() as u32);
    sr_words.extend_from_slice(&p2);
    assert!(StateRules::validate(&sr_words));

    let mut h_store = NodeStore::new();
    let mut h_changes = ChangeSet::default();
    let hn = {
        let mut tx = h_store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Layout>(n)
            .width(Length::points(120.0))
            .height(Length::points(40.0));
        tx.edit::<Interaction>(n).state_rules_with(|out| {
            out.extend_from_slice(&sr_words);
            true
        });
        tx.commit_into(&mut h_changes);
        n
    };
    let mut h_wire = Vec::new();
    WireEncoder::default().encode(&h_store, &h_changes, &mut h_wire);
    let mut h_rep = NodeStore::replica();
    let mut hr_changes = ChangeSet::default();
    {
        let mut rtx = h_rep.tx();
        WireDecoder::default().apply(&mut rtx, &h_wire).unwrap();
        rtx.commit_into(&mut hr_changes);
    }
    let mut h_skia = SkiaBackend::new();
    h_skia.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 200.0,
            height: 100.0,
            scale_factor: 1.0,
        },
    );
    h_skia.commit(&h_rep, &hr_changes).unwrap();
    let h_rect = h_skia.layout_of(hn).unwrap();
    h_skia.pointer_move(&h_rep, h_rect.x + 5.0, h_rect.y + 5.0);
    let mut merged = RowPatch::new();
    h_skia
        .layout_tree()
        .visit_effective_state_patches(&h_rep, hn, |p| merged.merge_view(p));
    assert_eq!(merged.table_count(), 40);
}
