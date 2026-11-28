//! Pointer lifecycle (`PointerDown`, `PointerMove`, `PointerUp`, `PointerEnter`, `PointerLeave`,
//! `PointerCancel`), declarative synchronous pointer capture (`OnPrimaryDown`), `Press`-on-release,
//! and OS window blur cancellation for [`DesktopSession`].

use nve_abi::{
    EventBatchBuf, EventFlags, InteractionEvent, NodeId, TextInputEvent, TimestampUs,
    NVE_POINTER_TYPE_MOUSE,
};
use nve_backend_skia::FocusModality;
use nve_store::NodeStore;
use nve_tables::tables::{Interaction, PointerCaptureMode};
use winit::event::MouseButton;
use winit::keyboard::ModifiersState;

use super::input::hit_test_focusable;
use super::session::{now_us, DesktopSession};
use super::{DesktopApp, DesktopHostError};

const PRIMARY_POINTER_ID: u32 = 1;

#[inline]
pub(super) fn mouse_button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(n) => n.min(u16::from(u8::MAX)) as u8,
    }
}

#[inline]
fn button_mask(button: u8) -> u16 {
    match button {
        0 => 1 << 0,
        1 => 1 << 2,
        2 => 1 << 1,
        3 => 1 << 3,
        4 => 1 << 4,
        _ => 0,
    }
}

#[inline]
fn pointer_flags(mods: ModifiersState, button: u8, extra: EventFlags) -> EventFlags {
    (EventFlags::from_modifiers(
        mods.super_key(),
        mods.control_key(),
        mods.shift_key(),
        mods.alt_key(),
        false,
    ) | extra)
        .with_pointer_button(button)
}

fn find_capture_target(store: &NodeStore, start: NodeId) -> Option<NodeId> {
    let mut cur = start;
    for _ in 0..256 {
        if let Some(inter) = store.try_row::<Interaction>(cur)
            && !inter.disabled()
            && inter.capture_mode() == PointerCaptureMode::OnPrimaryDown
        {
            return Some(cur);
        }
        let Some(parent) = store.parent(cur) else {
            break;
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    None
}

fn has_ancestor_listener(store: &NodeStore, start: NodeId, ev: InteractionEvent) -> bool {
    let mut cur = start;
    for _ in 0..256 {
        if store
            .try_row::<Interaction>(cur)
            .is_some_and(|i| i.listeners().contains(ev))
        {
            return true;
        }
        let Some(parent) = store.parent(cur) else {
            break;
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    false
}

impl<A: DesktopApp> DesktopSession<A> {
    /// Handles pointer button press at logical `(x, y)`: emits `PointerDown`, activates declarative
    /// synchronous capture (`PointerCaptureMode::OnPrimaryDown`), moves pointer-modality focus,
    /// and updates `TextInput` caret/multi-click/drag selection. Does NOT emit `Press` (emitted on
    /// release by [`Self::handle_pointer_up_logical_ex`]).
    pub fn handle_pointer_down_logical(
        &mut self,
        x: f32,
        y: f32,
        button: u8,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        self.modifiers = modifiers;
        self.pointer_buttons |= button_mask(button);

        let prev_focus = self.replica.backend().focused_node();
        let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
        let next_focus = if button == 0 {
            hit_test_focusable(self.replica.store(), self.replica.backend(), x, y)
        } else {
            prev_focus
        };
        let hit = self
            .replica
            .backend()
            .hit_test_interaction(self.replica.store(), x, y)
            .or_else(|| self.replica.backend().hit_test(self.replica.store(), x, y));

        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(6, 0);
        let mut visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_down(store, x, y)
        };

        let mut captured_id = None;
        if button == 0
            && let Some(hit_id) = hit
            && let Some(cap_id) = find_capture_target(self.replica.store(), hit_id)
        {
            self.replica
                .backend_mut()
                .set_pointer_capture(PRIMARY_POINTER_ID, cap_id);
            captured_id = Some(cap_id);
        }

        if let Some(node_id) = hit {
            let extra = if captured_id.is_some() {
                EventFlags::CAPTURE
            } else {
                EventFlags::EMPTY
            };
            events.push_pointer_ex(
                self.surface_id,
                node_id,
                ts,
                InteractionEvent::PointerDown,
                x,
                y,
                PRIMARY_POINTER_ID,
                self.pointer_buttons,
                NVE_POINTER_TYPE_MOUSE,
                pointer_flags(modifiers, button, extra),
            );
        }

        if button == 0 {
            if prev_focus != next_focus
                || (next_focus.is_some()
                    && self.replica.backend().interaction_tracker().focus_modality()
                        != FocusModality::Pointer)
            {
                let (store, backend) = self.replica.store_and_backend_mut();
                backend.set_focused_node_with_modality(store, next_focus, FocusModality::Pointer);
                visual_changed = true;
            }
            let resolved_next = self.replica.backend().focused_node();
            if resolved_next != prev_focus {
                visual_changed = true;
                if let Some(old_id) = prev_focus {
                    if had_preedit {
                        self.push_composition_event(
                            &mut events,
                            old_id,
                            ts,
                            TextInputEvent::CompositionEnd,
                            "",
                        );
                    }
                    events.push_blur(self.surface_id, old_id, ts);
                }
                if let Some(new_id) = resolved_next {
                    events.push_focus(self.surface_id, new_id, ts);
                }
            }
            if self.update_text_click_selection(x, y, next_focus) {
                visual_changed = true;
            }
        }

        self.finalize_input_turn(&events, visual_changed)
    }

    /// Performs a complete primary click (`PointerDown` followed by `PointerUp` + `Press`).
    #[cfg(test)]
    pub fn handle_click_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        let mods = self.modifiers;
        let down = self.handle_pointer_down_logical(x, y, 0, mods)?;
        let up = self.handle_pointer_up_logical_ex(x, y, 0, mods)?;
        Ok(down || up)
    }

    /// Updates hover/drag state at logical `(x, y)`, emitting non-bubbling `PointerLeave` /
    /// `PointerEnter` transitions and captured or listened `PointerMove` events.
    pub fn handle_pointer_move_logical_with_mods(
        &mut self,
        x: f32,
        y: f32,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        self.modifiers = modifiers;
        let mut visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_move(store, x, y)
        };

        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(8, 0);
        let store = self.replica.store();
        let tracker = self.replica.backend().interaction_tracker();
        let nb_flags = pointer_flags(modifiers, 0, EventFlags::NO_BUBBLE);

        for &ex_id in tracker.exited() {
            if store
                .try_row::<Interaction>(ex_id)
                .is_some_and(|i| i.listeners().contains(InteractionEvent::PointerLeave))
            {
                events.push_pointer_ex(
                    self.surface_id,
                    ex_id,
                    ts,
                    InteractionEvent::PointerLeave,
                    x,
                    y,
                    PRIMARY_POINTER_ID,
                    self.pointer_buttons,
                    NVE_POINTER_TYPE_MOUSE,
                    nb_flags,
                );
            }
        }
        for &en_id in tracker.entered() {
            if store
                .try_row::<Interaction>(en_id)
                .is_some_and(|i| i.listeners().contains(InteractionEvent::PointerEnter))
            {
                events.push_pointer_ex(
                    self.surface_id,
                    en_id,
                    ts,
                    InteractionEvent::PointerEnter,
                    x,
                    y,
                    PRIMARY_POINTER_ID,
                    self.pointer_buttons,
                    NVE_POINTER_TYPE_MOUSE,
                    nb_flags,
                );
            }
        }
        if let Some(cap_id) = tracker.captured_node() {
            events.push_pointer_ex(
                self.surface_id,
                cap_id,
                ts,
                InteractionEvent::PointerMove,
                x,
                y,
                PRIMARY_POINTER_ID,
                self.pointer_buttons,
                NVE_POINTER_TYPE_MOUSE,
                pointer_flags(modifiers, 0, EventFlags::CAPTURE | EventFlags::NO_BUBBLE),
            );
        } else if let Some(hit_id) = tracker.hovered()
            && has_ancestor_listener(store, hit_id, InteractionEvent::PointerMove)
        {
            events.push_pointer_ex(
                self.surface_id,
                hit_id,
                ts,
                InteractionEvent::PointerMove,
                x,
                y,
                PRIMARY_POINTER_ID,
                self.pointer_buttons,
                NVE_POINTER_TYPE_MOUSE,
                pointer_flags(modifiers, 0, EventFlags::EMPTY),
            );
        }

        if let Some(node_id) = self.dragging_text_node {
            let is_valid = self.replica.backend().focused_node() == Some(node_id)
                && self
                    .replica
                    .backend()
                    .is_text_input(self.replica.store(), node_id);
            if !is_valid {
                self.dragging_text_node = None;
            } else if let Some(next_caret) = self
                .replica
                .backend()
                .cursor_index_at_x(self.replica.store(), node_id, x)
                && next_caret != self.replica.backend().focus_state().caret_byte_index
            {
                let anchor = self.replica.backend().focus_state().selection_anchor;
                let (store, backend) = self.replica.store_and_backend_mut();
                backend.set_caret_and_selection(store, next_caret, anchor);
                self.mark_a11y_dirty(node_id);
                visual_changed = true;
            }
        }

        self.finalize_input_turn(&events, visual_changed)
    }

    #[cfg(test)]
    pub fn handle_pointer_move_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        let mods = self.modifiers;
        self.handle_pointer_move_logical_with_mods(x, y, mods)
    }

    /// Clears hover when the cursor leaves the window (preserving active pointer capture).
    pub fn handle_pointer_leave(&mut self) -> bool {
        let (x, y) = self.cursor_logical;
        let visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_leave(store)
        };
        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(4, 0);
        let store = self.replica.store();
        let nb_flags = pointer_flags(self.modifiers, 0, EventFlags::NO_BUBBLE);
        for &ex_id in self.replica.backend().interaction_tracker().exited() {
            if store
                .try_row::<Interaction>(ex_id)
                .is_some_and(|i| i.listeners().contains(InteractionEvent::PointerLeave))
            {
                events.push_pointer_ex(
                    self.surface_id,
                    ex_id,
                    ts,
                    InteractionEvent::PointerLeave,
                    x,
                    y,
                    PRIMARY_POINTER_ID,
                    self.pointer_buttons,
                    NVE_POINTER_TYPE_MOUSE,
                    nb_flags,
                );
            }
        }
        self.finalize_input_turn(&events, visual_changed)
            .unwrap_or(visual_changed)
    }

    /// Cancels active pointer capture and hover on OS window blur (`WindowEvent::Focused(false)`),
    /// emitting `PointerCancel` (`CAPTURE | NO_BUBBLE`) to the captured node.
    pub fn handle_window_blur(&mut self) -> Result<bool, DesktopHostError> {
        let (x, y) = self.cursor_logical;
        self.dragging_text_node = None;
        self.pointer_buttons = 0;
        let captured = self.replica.backend().captured_node();
        let visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_cancel(store)
        };
        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(4, 0);
        if let Some(cap_id) = captured {
            events.push_pointer_cancel(
                self.surface_id,
                cap_id,
                ts,
                x,
                y,
                PRIMARY_POINTER_ID,
                pointer_flags(
                    self.modifiers,
                    0,
                    EventFlags::CAPTURE | EventFlags::NO_BUBBLE,
                ),
            );
        }
        let store = self.replica.store();
        let nb_flags = pointer_flags(self.modifiers, 0, EventFlags::NO_BUBBLE);
        for &ex_id in self.replica.backend().interaction_tracker().exited() {
            if store
                .try_row::<Interaction>(ex_id)
                .is_some_and(|i| i.listeners().contains(InteractionEvent::PointerLeave))
            {
                events.push_pointer_ex(
                    self.surface_id,
                    ex_id,
                    ts,
                    InteractionEvent::PointerLeave,
                    x,
                    y,
                    PRIMARY_POINTER_ID,
                    0,
                    NVE_POINTER_TYPE_MOUSE,
                    nb_flags,
                );
            }
        }
        self.finalize_input_turn(&events, visual_changed)
    }

    /// Releases a pointer button, emitting `PointerUp` (routed to the captured node if captured)
    /// and `Press` on primary button release when `pointer_up_clicked` resolves a target.
    pub fn handle_pointer_up_logical_ex(
        &mut self,
        x: f32,
        y: f32,
        button: u8,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        self.modifiers = modifiers;
        self.pointer_buttons &= !button_mask(button);
        self.dragging_text_node = None;

        let captured = self.replica.backend().captured_node();
        let hit = self
            .replica
            .backend()
            .hit_test_interaction(self.replica.store(), x, y)
            .or_else(|| self.replica.backend().hit_test(self.replica.store(), x, y));
        let (clicked, visual_changed) = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_up_clicked(store, x, y)
        };

        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(4, 0);
        if let Some(cap_id) = captured {
            events.push_pointer_ex(
                self.surface_id,
                cap_id,
                ts,
                InteractionEvent::PointerUp,
                x,
                y,
                PRIMARY_POINTER_ID,
                self.pointer_buttons,
                NVE_POINTER_TYPE_MOUSE,
                pointer_flags(
                    modifiers,
                    button,
                    EventFlags::CAPTURE | EventFlags::NO_BUBBLE,
                ),
            );
        } else if let Some(target_id) = hit {
            events.push_pointer_ex(
                self.surface_id,
                target_id,
                ts,
                InteractionEvent::PointerUp,
                x,
                y,
                PRIMARY_POINTER_ID,
                self.pointer_buttons,
                NVE_POINTER_TYPE_MOUSE,
                pointer_flags(modifiers, button, EventFlags::EMPTY),
            );
        }
        if button == 0
            && let Some(clicked_id) = clicked
        {
            events.push_press(self.surface_id, clicked_id, ts, x, y);
        }
        self.finalize_input_turn(&events, visual_changed)
    }

    /// Finishes any in-progress mouse drag selection and releases primary press state.
    #[cfg(test)]
    pub fn handle_pointer_up_with_redraw(&mut self) -> bool {
        let (x, y) = self.cursor_logical;
        let mods = self.modifiers;
        self.handle_pointer_up_logical_ex(x, y, 0, mods)
            .unwrap_or(false)
    }

    /// Finishes any in-progress mouse drag selection and releases active press state.
    #[cfg(test)]
    pub fn handle_pointer_up_logical(&mut self) {
        let _ = self.handle_pointer_up_with_redraw();
    }
}
