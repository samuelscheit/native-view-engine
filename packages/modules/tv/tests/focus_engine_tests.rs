//! `SpatialFocusEngine` arena / NodeId semantics, explicit overrides, group memory, traps, wrap,
//! repeat throttling, the gamepad repeater, and the dense action code space.

use nve_tv::{
    DEFAULT_REPEAT_INTERVAL_US, DecimalDigit, FocusDirection, FocusOutcome, GamepadFrame,
    GamepadRepeater, NodeId, RemoteAction, RemoteInput, RepeatThrottle, SpatialFocusEngine,
    SpatialFocusGroupId, SpatialFocusNode, SpatialRect, standard_gamepad_button,
};

fn n(index: u32) -> NodeId {
    NodeId::new(index, 0)
}

fn card(id: NodeId, col: u32, row: u32) -> SpatialFocusNode {
    SpatialFocusNode::new(
        id,
        SpatialRect::new(col as f32 * 120.0, row as f32 * 90.0, 100.0, 80.0),
    )
}

/// One row of `count` cards with ids `first..first+count`.
fn row(engine: &mut SpatialFocusEngine, first: u32, count: u32, row: u32) {
    for i in 0..count {
        engine.register(card(n(first + i), i, row));
    }
}

fn input(action: RemoteAction, repeat: bool, timestamp_us: u64) -> RemoteInput {
    RemoteInput {
        action,
        repeat,
        timestamp_us,
    }
}

const RIGHT: RemoteAction = RemoteAction::Move(FocusDirection::Right);

#[test]
fn explicit_next_override_beats_geometry_and_falls_back_when_ineligible() {
    let mut engine = SpatialFocusEngine::new();
    row(&mut engine, 1, 3, 0);
    engine.register(card(n(9), 0, 4));
    engine.register(card(n(1), 0, 0).with_next(FocusDirection::Right, n(9)));
    assert!(engine.set_focus(n(1)));
    assert_eq!(
        engine.move_focus(FocusDirection::Right),
        Some(n(9)),
        "override wins"
    );

    engine.register(card(n(9), 0, 4).with_disabled(true));
    assert!(engine.set_focus(n(1)));
    assert_eq!(
        engine.move_focus(FocusDirection::Right),
        Some(n(2)),
        "disabled -> geometric"
    );

    engine.register(card(n(1), 0, 0).with_next(FocusDirection::Right, n(77)));
    assert!(engine.set_focus(n(1)));
    assert_eq!(
        engine.move_focus(FocusDirection::Right),
        Some(n(2)),
        "unknown -> geometric"
    );
    assert_eq!(
        engine.move_focus(FocusDirection::Left),
        Some(n(1)),
        "other directions unaffected"
    );
}

#[test]
fn explicit_override_cannot_escape_a_modal_trap() {
    let modal = SpatialFocusGroupId(9);
    let mut engine = SpatialFocusEngine::new();
    engine.register(card(n(1), 0, 0));
    engine.register(card(n(2), 0, 2).with_group(modal).with_traps_focus(true));
    engine.register(
        card(n(3), 1, 2)
            .with_group(modal)
            .with_next(FocusDirection::Up, n(1)),
    );
    assert!(engine.set_focus(n(3)));
    assert_eq!(
        engine.move_focus(FocusDirection::Up),
        None,
        "trap blocks override target"
    );
    assert_eq!(engine.move_focus(FocusDirection::Left), Some(n(2)));
}

#[test]
fn generations_supersede_stale_ids_and_lookups_require_exact_generation() {
    let mut engine = SpatialFocusEngine::new();
    let old = NodeId::new(5, 1);
    let new = NodeId::new(5, 2);
    engine.register(card(old, 0, 0));
    assert!(engine.node(old).is_some());
    engine.register(card(new, 1, 0));
    assert_eq!(engine.len(), 1, "same index replaces in place");
    assert!(
        engine.node(old).is_none(),
        "stale generation no longer resolves"
    );
    assert!(!engine.set_focus(old));
    assert!(engine.set_focus(new));
    assert!(
        engine.unregister(old).is_none(),
        "stale id cannot remove the live node"
    );
    assert_eq!(engine.focused(), Some(new));

    engine.register(card(NodeId::NULL, 2, 0));
    assert_eq!(engine.len(), 1, "NULL is never registered");
    assert!(engine.node(NodeId::NULL).is_none());
}

#[test]
fn unregister_swap_remove_keeps_every_remaining_lookup_valid() {
    let mut engine = SpatialFocusEngine::new();
    row(&mut engine, 1, 6, 0);
    let shelf = SpatialFocusGroupId(1);
    engine.register(card(n(40), 0, 3).with_group(shelf));
    assert!(engine.set_focus(n(40)));
    assert!(engine.set_focus(n(2)));

    let removed = engine.unregister(n(2)).expect("registered");
    assert_eq!(removed.id, n(2));
    assert_eq!(
        engine.focused(),
        None,
        "removing the focused node clears focus"
    );
    assert!(engine.unregister(n(2)).is_none());
    engine.unregister(n(40));
    assert_eq!(
        engine.last_focused_in_group(shelf),
        None,
        "memory of removed node dropped"
    );
    for id in [1, 3, 4, 5, 6] {
        assert_eq!(engine.node(n(id)).map(|node| node.id), Some(n(id)), "{id}");
    }
    assert_eq!(engine.len(), 5);
    assert!(engine.set_focus(n(1)));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(3)));
}

#[test]
fn replace_nodes_keeps_focus_and_group_memory_across_layout_snapshots() {
    let shelf = SpatialFocusGroupId(2);
    let rail = SpatialFocusGroupId(1);
    let layout = |scroll: f32| {
        (0..4u32)
            .map(move |i| {
                SpatialFocusNode::new(
                    n(10 + i),
                    SpatialRect::new(200.0 + i as f32 * 150.0 - scroll, 0.0, 140.0, 100.0),
                )
                .with_group(shelf)
            })
            .chain(std::iter::once(
                SpatialFocusNode::new(n(1), SpatialRect::new(0.0, 0.0, 150.0, 100.0))
                    .with_group(rail),
            ))
    };
    let mut engine = SpatialFocusEngine::new();
    engine.replace_nodes(layout(0.0));
    assert!(engine.set_focus(n(13)));
    assert!(engine.set_focus(n(1)));
    // Shelf scrolled by 300px between snapshots: memory still restores card 13.
    engine.replace_nodes(layout(300.0));
    assert_eq!(engine.len(), 5);
    assert_eq!(engine.focused(), Some(n(1)));
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(13)));

    // A snapshot without the focused node falls back to initial focus.
    engine.replace_nodes(layout(0.0).filter(|node| node.id != n(13)));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(n(1)));
    engine.clear();
    assert!(engine.is_empty());
    assert_eq!(engine.last_focused_in_group(shelf), None);
}

#[test]
fn trapping_node_without_group_confines_focus_to_trapping_nodes() {
    let mut engine = SpatialFocusEngine::new();
    row(&mut engine, 1, 3, 0);
    engine.register(card(n(20), 0, 2).with_traps_focus(true));
    engine.register(card(n(21), 1, 2).with_traps_focus(true));
    assert_eq!(
        engine.move_focus(FocusDirection::Down),
        Some(n(20)),
        "initial focus is trapped"
    );
    assert_eq!(engine.move_focus(FocusDirection::Up), None);
    assert_eq!(engine.move_focus(FocusDirection::Right), Some(n(21)));
}

#[test]
fn wrap_respects_traps_and_prefers_aligned_far_edge() {
    let mut engine = SpatialFocusEngine::new().with_wrap_axis(true);
    row(&mut engine, 1, 4, 0);
    row(&mut engine, 11, 4, 1);
    assert!(engine.set_focus(n(14)));
    assert_eq!(
        engine.move_focus(FocusDirection::Right),
        Some(n(11)),
        "wraps within the row"
    );
    assert_eq!(
        engine.move_focus(FocusDirection::Down),
        Some(n(1)),
        "vertical wrap"
    );
    engine.set_wrap_axis(false);
    assert!(!engine.wrap_axis());
    assert!(engine.set_focus(n(4)));
    assert_eq!(engine.move_focus(FocusDirection::Right), None);
}

#[test]
fn handle_input_throttles_repeats_per_action_and_reports_outcomes() {
    let mut engine = SpatialFocusEngine::new();
    row(&mut engine, 1, 5, 0);
    let interval = DEFAULT_REPEAT_INTERVAL_US;
    assert_eq!(
        engine.handle_input(input(RIGHT, false, 0)),
        FocusOutcome::Moved(n(1))
    );
    assert_eq!(
        engine.handle_input(input(RIGHT, false, 10)),
        FocusOutcome::Moved(n(2))
    );
    assert_eq!(
        engine.handle_input(input(RIGHT, true, 20)),
        FocusOutcome::Throttled
    );
    // A different action is never throttled by Right's window.
    assert_eq!(
        engine.handle_input(input(RemoteAction::Select, false, 30)),
        FocusOutcome::Action(RemoteAction::Select)
    );
    assert_eq!(
        engine.handle_input(input(RIGHT, true, 10 + interval)),
        FocusOutcome::Moved(n(3))
    );
    // Clock reset (timestamp goes backwards) is admitted.
    assert_eq!(
        engine.handle_input(input(RIGHT, true, 5)),
        FocusOutcome::Moved(n(4))
    );
    engine.throttle_mut().set_interval_us(0);
    assert_eq!(
        engine.handle_input(input(RIGHT, true, 6)),
        FocusOutcome::Moved(n(5))
    );
    assert_eq!(
        engine.handle_input(input(RIGHT, true, 7)),
        FocusOutcome::Blocked
    );
}

#[test]
fn repeat_throttle_admits_fresh_presses_and_resets() {
    let mut throttle = RepeatThrottle::new(50);
    assert_eq!(throttle.interval_us(), 50);
    assert!(
        throttle.admit(RIGHT, true, 100),
        "first repeat with no history passes"
    );
    assert!(!throttle.admit(RIGHT, true, 149));
    assert!(
        throttle.admit(RIGHT, false, 149),
        "fresh press always passes"
    );
    assert!(!throttle.admit(RIGHT, true, 150));
    assert!(throttle.admit(RIGHT, true, 199));
    throttle.reset();
    assert!(throttle.admit(RIGHT, true, 200));
}

#[test]
fn gamepad_repeater_detects_edges_holds_and_folds_the_left_stick() {
    let mut pad = GamepadRepeater::new();
    let mut log = Vec::new();
    let mut step = |pad: &mut GamepadRepeater, frame: GamepadFrame, t: u64| {
        pad.update(frame, t, |i| log.push((i.action, i.repeat, i.timestamp_us)));
    };
    let a = GamepadFrame {
        pressed: 1,
        ..Default::default()
    };
    step(&mut pad, a, 0);
    step(&mut pad, a, 16_000);
    assert!(pad.is_active());
    step(&mut pad, GamepadFrame::default(), 32_000);
    assert!(!pad.is_active());
    let stick_down = GamepadFrame {
        axis_y: 0.8,
        ..Default::default()
    };
    for t in [100_000u64, 300_000, 500_000, 560_000, 620_000, 640_000] {
        step(&mut pad, stick_down, t);
    }
    step(
        &mut pad,
        GamepadFrame {
            pressed: 1 << 16,
            axis_x: -0.4,
            ..Default::default()
        },
        700_000,
    );
    let down = RemoteAction::Move(FocusDirection::Down);
    assert_eq!(
        log,
        vec![
            (RemoteAction::Select, false, 0),
            (down, false, 100_000),
            (down, true, 500_000),
            (down, true, 620_000),
        ],
        "home button and sub-threshold stick are ignored"
    );
    assert_eq!(standard_gamepad_button(1), Some(RemoteAction::Back));
    assert_eq!(standard_gamepad_button(9), Some(RemoteAction::Menu));
    assert_eq!(standard_gamepad_button(2), None);
    pad.reset();
    assert!(!pad.is_active());
}

#[test]
fn remote_action_codes_are_dense_round_trip_and_name_stable() {
    for code in 0..RemoteAction::CODE_COUNT as u8 {
        let action = RemoteAction::from_code(code).expect("dense code space");
        assert_eq!(action.code(), code);
        assert_eq!(
            RemoteAction::from_key_name(action.key_name()),
            Some(action),
            "{action:?}"
        );
    }
    assert_eq!(
        RemoteAction::from_code(RemoteAction::CODE_COUNT as u8),
        None
    );
    assert_eq!(RemoteAction::from_code(u8::MAX), None);
    assert_eq!(size_of::<RemoteAction>(), 2);
    assert_eq!(DecimalDigit::new(10), None);
    assert_eq!(DecimalDigit::from_ascii(b'x'), None);
    assert_eq!(
        DecimalDigit::from_ascii(b'4').map(DecimalDigit::get),
        Some(4)
    );
    assert_eq!(RIGHT.direction(), Some(FocusDirection::Right));
    assert_eq!(RemoteAction::Back.direction(), None);
    assert_eq!(RemoteAction::from_key_name(""), None);
    assert_eq!(RemoteAction::from_key_name("x"), None);
}
