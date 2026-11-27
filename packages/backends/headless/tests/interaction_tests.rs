//! Deep integration tests for `:hover`, `:focus`, `:focus-visible`, `:active`, `:disabled`,
//! and [`CursorKind`] resolution inside [`HeadlessBackend`] over `NodeStore` word tables.

use nve_abi::{ColorRgba, InteractionEvent, SurfaceId};
use nve_backend_headless::{
    CursorKind, FocusModality, HeadlessBackend, InteractionState, RowPatch, StateStyleRule,
    StylePatchExt,
};
use nve_components::tables::{Interaction, Layout, StateRules, TextInput, Visual};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

#[test]
fn headless_interaction_hover_active_focus_disabled_and_cursor() {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let mut backend = HeadlessBackend::new();
    backend.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 400.0,
            height: 300.0,
            scale_factor: 1.0,
        },
    );

    let rules = vec![
        StateStyleRule::new(
            InteractionState::HOVER,
            RowPatch::new()
                .bg(ColorRgba::rgb(30, 64, 175))
                .cursor(CursorKind::Pointer),
        ),
        StateStyleRule::new(
            InteractionState::ACTIVE,
            RowPatch::new().bg(ColorRgba::rgb(29, 78, 216)),
        ),
        StateStyleRule::new(
            InteractionState::DISABLED,
            RowPatch::new()
                .opacity(0.4)
                .cursor(CursorKind::NotAllowed),
        ),
    ];

    let mut tx = store.tx();
    let btn = tx.create();
    let input = tx.create();
    tx.set_children(NodeId::ROOT, &[btn, input]);

    tx.edit::<Layout>(btn)
        .width(Length::points(120.0))
        .height(Length::points(40.0));
    tx.edit::<Visual>(btn).background(Rgba::rgb(15, 23, 42));
    tx.edit::<Interaction>(btn)
        .listen(InteractionEvent::Press)
        .cursor(CursorKind::Pointer)
        .state_rules(Some(StateRules(rules.into())));

    tx.edit::<Layout>(input)
        .width(Length::points(200.0))
        .height(Length::points(36.0));
    tx.edit::<TextInput>(input).placeholder("Search...");
    tx.commit_into(&mut changes);

    backend.commit(&store, &changes).unwrap();

    // Base state: pointer on btn, text cursor on input.
    assert_eq!(backend.effective_cursor(&store, btn), CursorKind::Pointer);
    assert_eq!(backend.effective_cursor(&store, input), CursorKind::Text);
    assert_eq!(
        backend.effective_visual(&store, btn).background,
        ColorRgba::rgb(15, 23, 42)
    );

    // Hover btn -> hover rule activates.
    let dirty = backend.dispatch_pointer_move(&store, 50.0, 20.0);
    assert_eq!(dirty, vec![btn]);
    assert!(backend.interaction_state(btn).contains(InteractionState::HOVER));
    assert_eq!(
        backend.effective_visual(&store, btn).background,
        ColorRgba::rgb(30, 64, 175)
    );

    // Pointer down -> active rule overrides hover.
    let _ = backend.dispatch_pointer_down(&store, 50.0, 20.0);
    assert_eq!(
        backend.effective_visual(&store, btn).background,
        ColorRgba::rgb(29, 78, 216)
    );
    let _ = backend.dispatch_pointer_up(&store, 50.0, 20.0);

    // Keyboard focus -> focus + focus-visible.
    let _ = backend.dispatch_focus(Some(btn), FocusModality::Keyboard);
    assert!(backend
        .interaction_state(btn)
        .contains(InteractionState::FOCUS_VISIBLE));

    // Disable via NodeStore Interaction table -> disabled rule + NotAllowed cursor.
    let mut tx = store.tx();
    tx.edit::<Interaction>(btn).disabled(true);
    tx.commit_into(&mut changes);
    backend.commit(&store, &changes).unwrap();

    assert!(backend
        .interaction_state(btn)
        .contains(InteractionState::DISABLED));
    assert_eq!(backend.effective_cursor(&store, btn), CursorKind::NotAllowed);
    assert!((backend.effective_visual(&store, btn).opacity - 0.4).abs() < 1e-4);
}
