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
    Display, InputKind, Interaction, Layout, Motion, StateRules, TextContent, TextInput, Transform,
    Transitions, Typography, Visual,
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

#[test]
fn hidden_text_input_has_zero_layout_zero_paint_ops_no_hit_and_refuses_focus() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
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
            .event_mask((EventMask::PRESS | EventMask::FOCUS | EventMask::BLUR) as u32);
        tx.commit_into(&mut changes);
        n
    };
    backend.commit(&store, &changes).unwrap();

    let rect = backend.layout_of(hidden.raw()).expect("hidden input layout entry");
    assert_eq!((rect.width, rect.height), (0.0, 0.0));
    assert!(
        backend.display_list().ops.is_empty(),
        "InputKind::Hidden must emit 0 paint ops, got {:?}",
        backend.display_list().ops
    );
    assert_eq!(backend.hit_test(&store, 1.0, 1.0), None);
    assert_eq!(backend.hit_test_interaction(&store, 1.0, 1.0), None);
    assert!(!backend.is_text_input(&store, hidden));
    backend.set_focused_node(&store, Some(hidden.raw()));
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
    backend.set_focused_node(&store, Some(visible.raw()));
    assert_eq!(backend.focused_node(), Some(visible.raw()));
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
    let vr = backend.layout_of(visible.raw()).expect("now-hidden layout");
    assert_eq!((vr.width, vr.height), (0.0, 0.0));
}

#[test]
fn ancestor_display_none_clears_and_rejects_child_text_input_focus() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(
        1,
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

    backend.set_focused_node(&store, Some(child.raw()));
    assert_eq!(backend.focused_node(), Some(child.raw()));

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
    backend.set_focused_node(&store, Some(child.raw()));
    assert_eq!(
        backend.focused_node(),
        None,
        "set_focused_node must reject focusing a child under a Display::None parent"
    );
}
