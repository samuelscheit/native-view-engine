//! Interactive pointer, drag/double-click/triple-click range selection, focus/blur, keyboard,
//! clipboard (`Ctrl/Cmd+C/X/V`), UAX #29 grapheme-safe UTF-8 editing, BiDi visual navigation,
//! unified IME composition events, and direct typed `KeyDown`/`Submit` dispatch for
//! [`DesktopSession`].

use std::time::{Duration, Instant};

use nve_abi::{
    native_events_to_direct, pack_key_modifiers, EventPayload, NativeEvent, NativeEventKind,
    NodeId, NveDirectEvent,
};
use winit::keyboard::{Key, ModifiersState};

use super::session::{now_us, DesktopSession};
pub use super::text_edit::{
    clamp_grapheme_boundary, classify_focused_key, classify_unfocused_keydown, delete_backward,
    delete_forward, format_keydown_shorthand, insert_text_at, is_printable_text,
    next_grapheme_boundary, normalize_line_endings, paragraph_bounds_at, prev_grapheme_boundary,
    sanitize_pasted_multiline, sanitize_pasted_single_line, word_bounds_at, FocusedKeyAction,
};
use super::{DesktopApp, DesktopHostError};

const MULTI_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(400);
const MULTI_CLICK_MAX_DIST_SQ: f32 = 36.0; // 6.0 logical px

impl<A: DesktopApp> DesktopSession<A> {
    pub(super) fn reset_multi_click(&mut self) {
        self.last_click_time = None;
        self.last_click_node = None;
        self.last_click_count = 0;
    }

    /// Programmatically moves focus to `next_focus` (or blurs when `None`), emitting balanced
    /// `CompositionEnd` (when blurring mid-composition) + `Blur`/`Focus` events and re-rendering.
    pub fn focus_node(&mut self, next_focus: Option<NodeId>) -> Result<bool, DesktopHostError> {
        let prev_focus = self.backend.focused_node();
        let had_preedit = self.backend.ime_preedit().is_some();
        let focus_changed = self.backend.set_focused_node(next_focus);
        let resolved_next = self.backend.focused_node();
        if !focus_changed {
            return Ok(false);
        }
        self.dragging_text_node = None;
        self.reset_multi_click();
        let ts = now_us();
        let mut events = Vec::with_capacity(3);
        if let Some(old_id) = prev_focus {
            if had_preedit {
                self.push_composition_event(
                    &mut events,
                    old_id,
                    ts,
                    NativeEventKind::CompositionEnd,
                    String::new(),
                );
            }
            events.push(NativeEvent {
                surface_id: self.surface_id,
                node_id: old_id,
                timestamp_us: ts,
                kind: NativeEventKind::Blur,
                payload: EventPayload::Empty,
            });
        }
        if let Some(new_id) = resolved_next {
            events.push(NativeEvent {
                surface_id: self.surface_id,
                node_id: new_id,
                timestamp_us: ts,
                kind: NativeEventKind::Focus,
                payload: EventPayload::Empty,
            });
        }
        self.dispatch_and_redraw(&events)?;
        Ok(true)
    }

    /// Translates a logical mouse press into focus transitions (`CompositionEnd`/`Blur`/`Focus`),
    /// `TextInput` caret/anchor placement (double-click word select, triple-click paragraph select),
    /// and hit-tested `Press` events.
    pub fn handle_click_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let prev_focus = self.backend.focused_node();
        let had_preedit = self.backend.ime_preedit().is_some();
        let next_focus = self.backend.hit_test_focusable(x, y);
        let press_target = self.backend.hit_test(x, y);

        let ts = now_us();
        let mut events = Vec::with_capacity(4);
        let mut visual_changed = !self.backend.pointer_down(x, y).is_empty();

        if prev_focus != next_focus {
            let changed = self.backend.set_focused_node(next_focus);
            let resolved_next = self.backend.focused_node();
            if changed {
                visual_changed = true;
                if let Some(old_id) = prev_focus {
                    if had_preedit {
                        self.push_composition_event(
                            &mut events,
                            old_id,
                            ts,
                            NativeEventKind::CompositionEnd,
                            String::new(),
                        );
                    }
                    events.push(NativeEvent {
                        surface_id: self.surface_id,
                        node_id: old_id,
                        timestamp_us: ts,
                        kind: NativeEventKind::Blur,
                        payload: EventPayload::Empty,
                    });
                }
                if let Some(new_id) = resolved_next {
                    events.push(NativeEvent {
                        surface_id: self.surface_id,
                        node_id: new_id,
                        timestamp_us: ts,
                        kind: NativeEventKind::Focus,
                        payload: EventPayload::Empty,
                    });
                }
            }
        }

        if let Some(text_id) = self
            .backend
            .focused_node()
            .filter(|&id| self.backend.is_text_input(id))
            && next_focus == Some(text_id)
        {
            let now = Instant::now();
            let dx = x - self.last_click_pos.0;
            let dy = y - self.last_click_pos.1;
            let in_chain = self.last_click_node == Some(text_id)
                && self
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t) <= MULTI_CLICK_MAX_INTERVAL)
                && (dx * dx + dy * dy) <= MULTI_CLICK_MAX_DIST_SQ;

            let click_count = if in_chain {
                match self.last_click_count {
                    1 => 2,
                    2 => 3,
                    _ => 1,
                }
            } else {
                1
            };

            let hit_idx = self
                .backend
                .cursor_index_at_x(text_id, x)
                .unwrap_or_else(|| self.backend.caret_byte_index());
            let prev_sel = (
                self.backend.selection_anchor_byte_index(),
                self.backend.caret_byte_index(),
            );

            match click_count {
                2 => {
                    let val = self.backend.text_input_value(text_id).unwrap_or_default();
                    let (w_start, w_end) = word_bounds_at(&val, hit_idx);
                    self.backend.set_selection_range(w_start, w_end);
                    self.dragging_text_node = None;
                }
                3 => {
                    let val = self.backend.text_input_value(text_id).unwrap_or_default();
                    let (p_start, p_end) = paragraph_bounds_at(&val, hit_idx);
                    self.backend.set_selection_range(p_start, p_end);
                    self.dragging_text_node = None;
                }
                _ => {
                    self.backend.set_selection_range(hit_idx, hit_idx);
                    self.dragging_text_node = Some(text_id);
                }
            }
            self.last_click_time = Some(now);
            self.last_click_pos = (x, y);
            self.last_click_node = Some(text_id);
            self.last_click_count = click_count;

            let next_sel = (
                self.backend.selection_anchor_byte_index(),
                self.backend.caret_byte_index(),
            );
            if next_sel != prev_sel {
                visual_changed = true;
            }
        } else {
            self.dragging_text_node = None;
            self.reset_multi_click();
        }

        if let Some(node_id) = press_target {
            events.push(NativeEvent {
                surface_id: self.surface_id,
                node_id,
                timestamp_us: ts,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x, y },
            });
        }

        if events.is_empty() && !visual_changed {
            return Ok(false);
        }

        let handled = if events.is_empty() {
            false
        } else {
            let mut direct = Vec::with_capacity(events.len());
            native_events_to_direct(&events, &mut direct);
            // SAFETY: `direct` borrows solely from `events`, which outlives `dispatch_events`.
            let batch = unsafe { nve_abi::NveEventBatchRef::from_slice(&direct) };
            self.app.dispatch_events(batch)
        };
        if handled {
            self.render_and_rasterize()?;
        } else if visual_changed {
            self.rasterize_current();
        }
        Ok(handled || visual_changed)
    }

    /// Updates hover/drag state at logical `(x, y)` and, while dragging inside a focused
    /// `TextInput`, extends the active range selection from the press anchor to the hovered
    /// UAX #29 grapheme boundary.
    pub fn handle_pointer_move_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let mut visual_changed = !self.backend.pointer_move(x, y).is_empty();
        if let Some(node_id) = self.dragging_text_node {
            if self.backend.focused_node() != Some(node_id) || !self.backend.is_text_input(node_id)
            {
                self.dragging_text_node = None;
            } else if let Some(next_caret) = self.backend.cursor_index_at_x(node_id, x)
                && next_caret != self.backend.caret_byte_index()
            {
                let anchor = self.backend.selection_anchor_byte_index();
                self.backend.set_selection_range(anchor, next_caret);
                visual_changed = true;
            }
        }
        if visual_changed {
            self.rasterize_current();
        }
        Ok(visual_changed)
    }

    /// Clears pointer hover/active state when the cursor leaves the window.
    pub fn handle_pointer_leave(&mut self) -> bool {
        let dirty = !self.backend.pointer_leave().is_empty();
        if dirty {
            self.rasterize_current();
        }
        dirty
    }

    /// Finishes any in-progress mouse drag selection and releases active press state, returning
    /// `true` when visual state changed and pixels were re-rasterized.
    pub fn handle_pointer_up_with_redraw(&mut self) -> bool {
        self.dragging_text_node = None;
        let (x, y) = self.cursor_logical;
        let dirty = !self.backend.pointer_up(x, y).is_empty();
        if dirty {
            self.rasterize_current();
        }
        dirty
    }

    /// Finishes any in-progress mouse drag selection and releases active press state.
    pub fn handle_pointer_up_logical(&mut self) {
        let _ = self.handle_pointer_up_with_redraw();
    }

    /// Dispatches a direct typed `NativeEventKind::KeyDown` event.
    pub fn handle_keydown(
        &mut self,
        key: &str,
        meta: bool,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Result<bool, DesktopHostError> {
        if key.is_empty() {
            return Ok(false);
        }
        let node_id = match self.backend.focused_node() {
            Some(focused) => self.backend.find_custom_event_target_from(focused),
            None => self.backend.find_custom_event_target(),
        };
        let event = [NveDirectEvent::key_down(
            self.surface_id,
            node_id,
            now_us(),
            key,
            "",
            pack_key_modifiers(meta, ctrl, shift, alt, false),
            0,
            false,
        )];
        // SAFETY: `key` and `""` remain valid for the duration of `dispatch_events`.
        let batch = unsafe { nve_abi::NveEventBatchRef::from_slice(&event) };
        let handled = self.app.dispatch_events(batch);
        if handled {
            self.render_and_rasterize()?;
        }
        Ok(handled)
    }

    /// Handles a `winit` keyboard press: when a `TextInput` is focused, drives UAX #29
    /// grapheme-safe editing, BiDi visual caret/selection movement (`Shift+Left/Right/Home/End`),
    /// clipboard (`Ctrl/Cmd+C/X/V`), `Cmd/Ctrl+A`, `Enter` -> `"submit"`, and `Escape` ->
    /// cancel preedit or blur/keydown.
    pub fn handle_keyboard_input(
        &mut self,
        key: &Key,
        text: Option<&str>,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
        self.reset_multi_click();
        let meta = modifiers.super_key();
        let ctrl = modifiers.control_key();
        let shift = modifiers.shift_key();
        let alt = modifiers.alt_key();

        let Some(focused_id) = self
            .backend
            .focused_node()
            .filter(|&id| self.backend.is_text_input(id))
        else {
            return if let Some(mapped) = classify_unfocused_keydown(key, modifiers) {
                self.handle_keydown(mapped, meta, ctrl, shift, alt)
            } else {
                Ok(false)
            };
        };

        let current = self.backend.text_input_value(focused_id).unwrap_or_default();
        let caret = self.backend.caret_byte_index();
        let anchor = self.backend.selection_anchor_byte_index();
        let selection = self.backend.selection_range();

        match classify_focused_key(key, text, modifiers) {
            FocusedKeyAction::Insert(inserted) => {
                let comp_end = self.backend.ime_preedit().is_some().then(String::new);
                let (next_val, next_caret) = insert_text_at(&current, caret, selection, &inserted);
                self.emit_text_change(focused_id, next_val, next_caret, comp_end)
            }
            FocusedKeyAction::Backspace => {
                if self.backend.ime_preedit().is_some() {
                    self.backend.set_ime_preedit(None, None);
                    let mut events = Vec::with_capacity(1);
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        now_us(),
                        NativeEventKind::CompositionEnd,
                        String::new(),
                    );
                    self.dispatch_and_redraw(&events)?;
                    return Ok(true);
                }
                let Some((next_val, next_caret)) = delete_backward(&current, caret, selection)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret, None)
            }
            FocusedKeyAction::Delete => {
                let Some((next_val, next_caret)) = delete_forward(&current, caret, selection)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret, None)
            }
            FocusedKeyAction::MoveLeft => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = match selection {
                    Some((s, _)) => s,
                    None => self.backend.move_caret_visual(focused_id, caret, -1),
                };
                let changed = next_caret != caret || selection.is_some();
                self.backend.set_caret_byte_index(next_caret);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveRight => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = match selection {
                    Some((_, e)) => e,
                    None => self.backend.move_caret_visual(focused_id, caret, 1),
                };
                let changed = next_caret != caret || selection.is_some();
                self.backend.set_caret_byte_index(next_caret);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveHome => {
                let changed = caret != 0 || selection.is_some();
                self.backend.set_caret_byte_index(0);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveEnd => {
                let end = current.len();
                let changed = caret != end || selection.is_some();
                self.backend.set_caret_byte_index(end);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::SelectLeft => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = self.backend.move_caret_visual(focused_id, caret, -1);
                if next_caret == caret {
                    return Ok(false);
                }
                self.backend.set_selection_range(anchor, next_caret);
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectRight => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = self.backend.move_caret_visual(focused_id, caret, 1);
                if next_caret == caret {
                    return Ok(false);
                }
                self.backend.set_selection_range(anchor, next_caret);
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectHome => {
                if caret == 0 {
                    return Ok(false);
                }
                self.backend.set_selection_range(anchor, 0);
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectEnd => {
                let end = current.len();
                if caret == end {
                    return Ok(false);
                }
                self.backend.set_selection_range(anchor, end);
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectAll => {
                self.backend.select_all_text();
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::Copy => {
                if self.backend.is_secure_text_input(focused_id) {
                    return Ok(false);
                }
                if let Some(selected) = self.backend.selected_text() {
                    self.clipboard.set_text(&selected);
                    return Ok(true);
                }
                Ok(false)
            }
            FocusedKeyAction::Cut => {
                let Some((s, e)) = selection else {
                    return Ok(false);
                };
                if !self.backend.is_secure_text_input(focused_id)
                    && let Some(selected) = self.backend.selected_text()
                {
                    self.clipboard.set_text(&selected);
                }
                let Some((next_val, next_caret)) = delete_backward(&current, caret, Some((s, e)))
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret, None)
            }
            FocusedKeyAction::Paste => {
                let Some(raw_clip) = self.clipboard.get_text() else {
                    return Ok(false);
                };
                let sanitized = if self.backend.is_multiline_text_input(focused_id) {
                    sanitize_pasted_multiline(&raw_clip)
                } else {
                    sanitize_pasted_single_line(&raw_clip)
                };
                if sanitized.is_empty() {
                    return Ok(false);
                }
                let comp_end = self.backend.ime_preedit().is_some().then(String::new);
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, selection, &sanitized);
                self.emit_text_change(focused_id, next_val, next_caret, comp_end)
            }
            FocusedKeyAction::Submit => {
                let had_preedit = self.backend.ime_preedit().is_some();
                self.backend.set_ime_preedit(None, None);
                let ts = now_us();
                let mut events = Vec::with_capacity(2);
                if had_preedit {
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        ts,
                        NativeEventKind::CompositionEnd,
                        String::new(),
                    );
                }
                events.push(NativeEvent {
                    surface_id: self.surface_id,
                    node_id: focused_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::Submit,
                    payload: EventPayload::Text(current),
                });
                let mut direct = Vec::with_capacity(events.len());
                native_events_to_direct(&events, &mut direct);
                // SAFETY: `direct` borrows solely from `events`, which outlives `dispatch_events`.
                let batch = unsafe { nve_abi::NveEventBatchRef::from_slice(&direct) };
                if self.app.dispatch_events(batch) {
                    self.render_and_rasterize()?;
                    Ok(true)
                } else {
                    self.handle_keydown("Enter", meta, ctrl, shift, alt)
                }
            }
            FocusedKeyAction::Escape => {
                if self.backend.ime_preedit().is_some() {
                    self.backend.set_ime_preedit(None, None);
                    let mut events = Vec::with_capacity(1);
                    self.push_composition_event(
                        &mut events,
                        focused_id,
                        now_us(),
                        NativeEventKind::CompositionEnd,
                        String::new(),
                    );
                    self.dispatch_and_redraw(&events)?;
                    return Ok(true);
                }
                let has_custom = self.backend.has_custom_event_listener();
                let custom_target = self.backend.find_custom_event_target_from(focused_id);
                self.backend.set_focused_node(None);
                self.dragging_text_node = None;

                let ts = now_us();
                let mut events = Vec::with_capacity(2);
                events.push(NativeEvent {
                    surface_id: self.surface_id,
                    node_id: focused_id,
                    timestamp_us: ts,
                    kind: NativeEventKind::Blur,
                    payload: EventPayload::Empty,
                });
                if has_custom {
                    events.push(NativeEvent {
                        surface_id: self.surface_id,
                        node_id: custom_target,
                        timestamp_us: ts,
                        kind: NativeEventKind::KeyDown,
                        payload: EventPayload::Key {
                            key: "Escape".to_string(),
                            code: String::new(),
                            key_code: 0,
                            modifiers: pack_key_modifiers(meta, ctrl, shift, alt, false),
                            is_composing: false,
                        },
                    });
                }
                self.dispatch_and_redraw(&events)?;
                Ok(true)
            }
            FocusedKeyAction::ForwardKeydown(mapped) => {
                self.handle_keydown(mapped, meta, ctrl, shift, alt)
            }
            FocusedKeyAction::None => Ok(false),
        }
    }
}
