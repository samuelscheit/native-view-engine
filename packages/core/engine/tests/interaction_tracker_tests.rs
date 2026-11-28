//! `InteractionTracker` hover / press-drag-cancel / touch / focus-modality / disabled / unmount
//! transitions (moved with the tracker from `nve-tables`).

use nve_abi::NodeId;
use nve_tables::interaction::{FocusModality, InteractionState};
use nve_engine::InteractionTracker;

#[test]
fn interaction_tracker_handles_hover_drag_cancel_touch_focus_modality_disabled_and_unmount() {
    let mut t = InteractionTracker::new();
    let n10 = NodeId::new(10, 0);
    let n20 = NodeId::new(20, 0);
    let n30 = NodeId::new(30, 0);

    // 1. Hover enter, repeat (no dirty), switch, and leave.
    assert_eq!(t.pointer_move(Some(n10)), vec![n10]);
    assert_eq!(t.state(n10), InteractionState::HOVER);
    assert!(t.pointer_move(Some(n10)).is_empty());
    assert_eq!(t.pointer_move(Some(n20)), vec![n10, n20]);
    assert_eq!(t.state(n10), InteractionState::NONE);
    assert_eq!(t.state(n20), InteractionState::HOVER);

    // 2. Press + click completion on same node.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.state(n20), InteractionState::HOVER | InteractionState::ACTIVE);
    assert_eq!(t.active_cursor_node(), Some(n20));
    let (clicked, dirty) = t.pointer_up_clicked(Some(n20));
    assert_eq!(clicked, Some(n20));
    assert_eq!(dirty, vec![n20]);
    assert_eq!(t.state(n20), InteractionState::HOVER);

    // 3. Press-cancel on drag-off.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.active(), Some(n20));
    assert_eq!(t.pointer_move(Some(n30)), vec![n20, n30]);
    assert_eq!(t.active(), None, "dragging off node 20 cancels active press");
    assert_eq!(t.state(n20), InteractionState::NONE);
    assert_eq!(t.state(n30), InteractionState::HOVER);
    let (clicked_after_drag, dirty_up) = t.pointer_up_clicked(Some(n30));
    assert_eq!(clicked_after_drag, None);
    assert!(dirty_up.is_empty());

    // 4. Touch sticky-hover prevention on hybrid devices.
    assert_eq!(t.hovered(), Some(n30));
    assert_eq!(t.touch_tap(Some(n30)), vec![n30]);
    assert_eq!(t.hovered(), None, "touch tap must clear hovered so no sticky hover remains");
    assert_eq!(t.active(), None);

    assert_eq!(t.pointer_move(Some(n10)), vec![n10]);
    assert_eq!(t.touch_start(Some(n20)), vec![n10, n20]);
    assert_eq!(t.hovered(), None);
    assert_eq!(t.active(), Some(n20));
    assert_eq!(t.touch_end(), vec![n20]);
    assert_eq!(t.state(n20), InteractionState::NONE);

    // 5. Focus modality: Pointer -> FOCUS only; Keyboard -> FOCUS | FOCUS_VISIBLE.
    assert_eq!(t.focus_pointer(Some(n10)), vec![n10]);
    assert_eq!(t.focus_modality(), FocusModality::Pointer);
    assert_eq!(t.state(n10), InteractionState::FOCUS);
    assert_eq!(t.focus_keyboard(Some(n10)), vec![n10]);
    assert_eq!(t.focus_modality(), FocusModality::Keyboard);
    assert_eq!(t.state(n10), InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE);
    assert_eq!(t.focus_pointer(Some(n10)), vec![n10]);
    assert_eq!(t.state(n10), InteractionState::FOCUS);
    assert_eq!(t.focus_keyboard(Some(n20)), vec![n10, n20]);
    assert_eq!(t.state(n10), InteractionState::NONE);
    assert_eq!(t.state(n20), InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE);

    // 6. Disabled clears active & focus and rejects new press/focus while keeping hover cursor target.
    assert_eq!(t.pointer_down(Some(n20)), vec![n20]);
    assert_eq!(t.set_disabled(n20, true), vec![n20]);
    assert_eq!(t.active(), None);
    assert_eq!(t.focused(), None);
    assert_eq!(t.state(n20), InteractionState::HOVER | InteractionState::DISABLED);

    assert!(t.pointer_down(Some(n20)).is_empty(), "disabled node cannot become active");
    assert_eq!(t.active(), None);
    assert_eq!(t.focus_keyboard(Some(n10)), vec![n10]);
    assert!(t.focus_keyboard(Some(n20)).is_empty(), "focusing disabled node must not steal focus");
    assert_eq!(t.focused(), Some(n10));

    // 7. Unmount cleanly clears hovered, active, focused, and disabled state.
    assert_eq!(t.remove_node(n10), vec![n10]);
    assert_eq!(t.focused(), None);
    assert_eq!(t.state(n10), InteractionState::NONE);

    assert_eq!(t.remove_node(n20), vec![n20]);
    assert_eq!(t.hovered(), None);
    assert!(!t.is_disabled(n20));
    assert_eq!(t.state(n20), InteractionState::NONE);
    assert!(t.remove_node(NodeId::new(999, 0)).is_empty());
}
