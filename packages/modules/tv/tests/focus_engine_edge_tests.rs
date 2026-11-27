//! Engine edge cases: self-pinning overrides, trap eligibility and pull-in, generation
//! replacement, bounded group memory, viewport-first initial focus, non-repeating actions, and
//! gamepad stick dominance / hysteresis.

use nve_tv::{
    DecimalDigit, FocusDirection, FocusOutcome, GROUP_MEMORY_CAPACITY, GamepadFrame,
    GamepadRepeater, NodeId, RemoteAction, RemoteInput, RepeatThrottle, SpatialFocusEngine,
    SpatialFocusGroupId, SpatialFocusNode, SpatialRect,
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

fn press(action: RemoteAction, repeat: bool, timestamp_us: u64) -> RemoteInput {
    RemoteInput {
        action,
        repeat,
        timestamp_us,
    }
}

const RIGHT: FocusDirection = FocusDirection::Right;

#[test]
fn override_naming_the_node_itself_pins_focus() {
    let mut engine = SpatialFocusEngine::new();
    engine.register(card(n(1), 0, 0).with_next(RIGHT, n(1)));
    engine.register(card(n(2), 1, 0));
    assert!(engine.set_focus(n(1)));
    assert_eq!(
        engine.move_focus(RIGHT),
        None,
        "self-override blocks, never re-focuses"
    );
    assert_eq!(
        engine.handle_input(press(RemoteAction::Move(RIGHT), false, 0)),
        FocusOutcome::Blocked
    );
    assert_eq!(engine.focused(), Some(n(1)));
}

#[test]
fn ineligible_trapping_nodes_never_confine_focus() {
    let modal = SpatialFocusGroupId(7);
    let mut engine = SpatialFocusEngine::new();
    engine.register(card(n(1), 0, 0).with_group(modal));
    engine.register(card(n(2), 1, 0));
    // The group's only trapping node is disabled / collapsed: the group is not a trap.
    engine.register(
        card(n(3), 0, 1)
            .with_group(modal)
            .with_traps_focus(true)
            .with_disabled(true),
    );
    engine.register(
        SpatialFocusNode::new(n(4), SpatialRect::new(0.0, 0.0, 0.0, 0.0))
            .with_group(modal)
            .with_traps_focus(true),
    );
    assert!(engine.set_focus(n(1)));
    assert_eq!(engine.move_focus(RIGHT), Some(n(2)));
}

#[test]
fn focus_behind_an_active_trap_is_pulled_into_it() {
    let modal = SpatialFocusGroupId(9);
    let mut engine = SpatialFocusEngine::new();
    for i in 0..3 {
        engine.register(card(n(1 + i), i, 0));
    }
    assert!(engine.set_focus(n(1)));
    // A modal opens while focus stays on the background row.
    engine.register(card(n(10), 1, 2).with_group(modal).with_traps_focus(true));
    engine.register(card(n(11), 2, 2).with_group(modal));
    assert_eq!(
        engine.move_focus(RIGHT),
        Some(n(10)),
        "enters the trap, not n(2)"
    );
    assert_eq!(engine.move_focus(RIGHT), Some(n(11)));
    assert_eq!(
        engine.move_focus(FocusDirection::Up),
        None,
        "cannot leave the trap"
    );
}

#[test]
fn registering_a_new_generation_drops_stale_focus_and_group_memory() {
    let shelf = SpatialFocusGroupId(1);
    let mut engine = SpatialFocusEngine::new();
    let old = NodeId::new(5, 1);
    engine.register(card(old, 0, 0).with_group(shelf));
    engine.register(card(n(6), 1, 0));
    assert!(engine.set_focus(old));
    assert_eq!(engine.last_focused_in_group(shelf), Some(old));
    engine.register(card(NodeId::new(5, 2), 0, 0).with_group(shelf));
    assert_eq!(engine.focused(), None);
    assert_eq!(engine.last_focused_in_group(shelf), None);
    // Same id re-registered (layout refresh) keeps focus.
    assert!(engine.set_focus(n(6)));
    engine.register(card(n(6), 1, 1));
    assert_eq!(engine.focused(), Some(n(6)));
}

#[test]
fn group_memory_is_a_bounded_lru() {
    let mut engine = SpatialFocusEngine::new();
    let groups = GROUP_MEMORY_CAPACITY as u32 + 8;
    for g in 0..groups {
        engine.register(card(n(g + 1), g, 0).with_group(SpatialFocusGroupId(g)));
    }
    assert!(engine.set_focus(n(1)));
    for g in 1..groups {
        assert!(engine.set_focus(n(g + 1)));
        if g == GROUP_MEMORY_CAPACITY as u32 - 1 {
            // Touch group 0 again so it is the most recent, not the eviction victim.
            assert!(engine.set_focus(n(1)));
        }
    }
    let remembered = (0..groups)
        .filter(|&g| {
            engine
                .last_focused_in_group(SpatialFocusGroupId(g))
                .is_some()
        })
        .count();
    assert_eq!(remembered, GROUP_MEMORY_CAPACITY);
    assert_eq!(
        engine.last_focused_in_group(SpatialFocusGroupId(0)),
        Some(n(1))
    );
    assert_eq!(
        engine.last_focused_in_group(SpatialFocusGroupId(1)),
        None,
        "LRU evicted"
    );
    assert_eq!(
        engine.last_focused_in_group(SpatialFocusGroupId(groups - 1)),
        Some(n(groups))
    );
}

#[test]
fn initial_focus_prefers_the_viewport_on_scrolled_pages() {
    let mut engine = SpatialFocusEngine::new();
    // Scrolled down: row 0 sits above the viewport (negative client y).
    engine.register(SpatialFocusNode::new(
        n(1),
        SpatialRect::new(0.0, -400.0, 100.0, 80.0),
    ));
    engine.register(SpatialFocusNode::new(
        n(2),
        SpatialRect::new(200.0, 40.0, 100.0, 80.0),
    ));
    engine.register(SpatialFocusNode::new(
        n(3),
        SpatialRect::new(0.0, 160.0, 100.0, 80.0),
    ));
    engine.set_viewport(Some(SpatialRect::new(0.0, 0.0, 1280.0, 720.0)));
    assert_eq!(engine.move_focus(FocusDirection::Down), Some(n(2)));
    engine.clear_focus();
    engine.set_viewport(None);
    assert_eq!(
        engine.move_focus(FocusDirection::Down),
        Some(n(1)),
        "no viewport: top-left"
    );
}

#[test]
fn only_auto_repeating_actions_repeat() {
    let digit = RemoteAction::Digit(DecimalDigit::new(4).unwrap_or_else(|| unreachable!()));
    for action in [
        RemoteAction::Move(RIGHT),
        RemoteAction::ChannelUp,
        RemoteAction::ChannelDown,
        RemoteAction::FastForward,
        RemoteAction::Rewind,
        digit,
    ] {
        assert!(action.auto_repeats(), "{action:?}");
    }
    for code in 0..RemoteAction::CODE_COUNT as u8 {
        let Some(action) = RemoteAction::from_code(code) else {
            continue;
        };
        let mut throttle = RepeatThrottle::new(0);
        assert!(throttle.admit(action, false, 0), "fresh {action:?}");
        assert_eq!(
            throttle.would_admit(action, true, 1_000_000),
            action.auto_repeats(),
            "{action:?} repeat with rate limiting disabled"
        );
    }
    let mut engine = SpatialFocusEngine::new();
    let held_select = press(RemoteAction::Select, true, 500_000);
    assert!(!engine.would_admit(&held_select));
    assert_eq!(engine.handle_input(held_select), FocusOutcome::Throttled);
    let first = press(RemoteAction::Move(RIGHT), true, 0);
    assert!(engine.would_admit(&first));
    assert!(engine.would_admit(&first), "would_admit records nothing");
}

#[test]
fn gamepad_stick_uses_the_dominant_axis_with_release_hysteresis() {
    let mut pad = GamepadRepeater::new();
    let mut log = Vec::new();
    let mut step = |pad: &mut GamepadRepeater, axis_x: f32, axis_y: f32, t: u64| {
        let frame = GamepadFrame {
            pressed: 0,
            axis_x,
            axis_y,
        };
        pad.update(frame, t, |i| log.push((i.action, i.repeat)));
    };
    step(&mut pad, 0.7, 0.6, 0); // diagonal: only Right
    step(&mut pad, 0.35, 0.1, 16_000); // drifts below press, above release: still held
    step(&mut pad, 0.35, 0.1, 450_000); // ... and auto-repeats
    step(&mut pad, 0.25, 0.1, 470_000); // below release
    step(&mut pad, 0.45, 0.0, 480_000); // below press threshold again: no re-press
    step(&mut pad, 0.0, -0.9, 500_000);
    step(&mut pad, f32::NAN, 0.0, 520_000);
    let mv = RemoteAction::Move;
    assert_eq!(
        log,
        [
            (mv(RIGHT), false),
            (mv(RIGHT), true),
            (mv(FocusDirection::Up), false),
        ]
    );
    assert!(!pad.is_active(), "NaN axes release everything");
    // Saturating deadlines near the clock's end never overflow.
    pad.update(
        GamepadFrame {
            pressed: 1,
            ..GamepadFrame::default()
        },
        u64::MAX - 1,
        |_| {},
    );
    assert!(pad.is_active());
}
