//! Interactive pointer, scroll, focus/blur, keyboard, UTF-8 text editing, IME, and zero-JSON
//! compact keydown shorthand dispatch for [`DesktopSession`].

use nve_abi::events::encode_keydown_bytes;
use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, NodeId};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::session::{now_us, DesktopSession};
use super::{DesktopApp, DesktopHostError};

/// Formats a keyboard event into NVE's zero-JSON compact modifier shorthand
/// (e.g. `"Cmd+k"`, `"Ctrl+Shift+Enter"`, `"ArrowDown"`, `"Escape"`), which both
/// `nve-dioxus` and `studio-workspace` parse directly with zero JSON overhead.
pub fn format_keydown_shorthand(
    key: &str,
    meta: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> String {
    let mut out = String::with_capacity(key.len() + 24);
    if meta {
        out.push_str("Cmd+");
    }
    if ctrl {
        out.push_str("Ctrl+");
    }
    if shift {
        out.push_str("Shift+");
    }
    if alt {
        out.push_str("Alt+");
    }
    out.push_str(key);
    out
}

#[inline]
pub fn clamp_char_boundary(s: &str, byte_index: usize) -> usize {
    let mut idx = byte_index.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[inline]
pub fn prev_char_boundary(s: &str, byte_index: usize) -> usize {
    let idx = clamp_char_boundary(s, byte_index);
    if idx == 0 {
        return 0;
    }
    let mut prev = idx - 1;
    while prev > 0 && !s.is_char_boundary(prev) {
        prev -= 1;
    }
    prev
}

#[inline]
pub fn next_char_boundary(s: &str, byte_index: usize) -> usize {
    let idx = clamp_char_boundary(s, byte_index);
    if idx >= s.len() {
        return s.len();
    }
    let mut next = idx + 1;
    while next < s.len() && !s.is_char_boundary(next) {
        next += 1;
    }
    next
}

pub fn insert_text_at(
    current: &str,
    caret: usize,
    select_all: bool,
    inserted: &str,
) -> (String, usize) {
    if select_all {
        return (inserted.to_owned(), inserted.len());
    }
    let idx = clamp_char_boundary(current, caret);
    let mut out = String::with_capacity(current.len() + inserted.len());
    out.push_str(&current[..idx]);
    out.push_str(inserted);
    out.push_str(&current[idx..]);
    (out, idx + inserted.len())
}

pub fn delete_backward(current: &str, caret: usize, select_all: bool) -> Option<(String, usize)> {
    if select_all {
        return (!current.is_empty()).then(|| (String::new(), 0));
    }
    let idx = clamp_char_boundary(current, caret);
    if idx == 0 {
        return None;
    }
    let prev = prev_char_boundary(current, idx);
    let mut out = String::with_capacity(current.len() - (idx - prev));
    out.push_str(&current[..prev]);
    out.push_str(&current[idx..]);
    Some((out, prev))
}

pub fn delete_forward(current: &str, caret: usize, select_all: bool) -> Option<(String, usize)> {
    if select_all {
        return (!current.is_empty()).then(|| (String::new(), 0));
    }
    let idx = clamp_char_boundary(current, caret);
    if idx >= current.len() {
        return None;
    }
    let next = next_char_boundary(current, idx);
    let mut out = String::with_capacity(current.len() - (next - idx));
    out.push_str(&current[..idx]);
    out.push_str(&current[next..]);
    Some((out, idx))
}

#[inline]
pub fn is_printable_text(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusedKeyAction {
    Insert(String),
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    SelectAll,
    Submit,
    Escape,
    ForwardKeydown(&'static str),
    None,
}

pub fn classify_focused_key(
    key: &Key,
    text: Option<&str>,
    mods: ModifiersState,
) -> FocusedKeyAction {
    let meta = mods.super_key();
    let ctrl = mods.control_key();
    let shift = mods.shift_key();
    let alt = mods.alt_key();
    let primary = meta || ctrl;

    match key {
        Key::Named(NamedKey::Backspace) => FocusedKeyAction::Backspace,
        Key::Named(NamedKey::Delete) => FocusedKeyAction::Delete,
        Key::Named(NamedKey::ArrowLeft) => {
            if meta {
                FocusedKeyAction::MoveHome
            } else {
                FocusedKeyAction::MoveLeft
            }
        }
        Key::Named(NamedKey::ArrowRight) => {
            if meta {
                FocusedKeyAction::MoveEnd
            } else {
                FocusedKeyAction::MoveRight
            }
        }
        Key::Named(NamedKey::Home) => FocusedKeyAction::MoveHome,
        Key::Named(NamedKey::End) => FocusedKeyAction::MoveEnd,
        Key::Named(NamedKey::Enter) => {
            if primary {
                FocusedKeyAction::ForwardKeydown("Enter")
            } else {
                FocusedKeyAction::Submit
            }
        }
        Key::Named(NamedKey::Escape) => FocusedKeyAction::Escape,
        Key::Named(NamedKey::ArrowUp) => FocusedKeyAction::ForwardKeydown("ArrowUp"),
        Key::Named(NamedKey::ArrowDown) => FocusedKeyAction::ForwardKeydown("ArrowDown"),
        Key::Named(NamedKey::Space) => {
            if primary {
                FocusedKeyAction::None
            } else {
                FocusedKeyAction::Insert(" ".to_string())
            }
        }
        Key::Character(ch) => {
            if primary && !alt && !shift && ch.eq_ignore_ascii_case("a") {
                return FocusedKeyAction::SelectAll;
            }
            if primary && ch.eq_ignore_ascii_case("k") {
                return FocusedKeyAction::ForwardKeydown("k");
            }
            if primary {
                return FocusedKeyAction::None;
            }
            if let Some(t) = text.filter(|s| is_printable_text(s)) {
                FocusedKeyAction::Insert(t.to_owned())
            } else if is_printable_text(ch.as_str()) {
                FocusedKeyAction::Insert(ch.to_string())
            } else {
                FocusedKeyAction::None
            }
        }
        _ => {
            if !primary {
                if let Some(t) = text.filter(|s| is_printable_text(s)) {
                    return FocusedKeyAction::Insert(t.to_owned());
                }
            }
            FocusedKeyAction::None
        }
    }
}

pub fn classify_unfocused_keydown(key: &Key, mods: ModifiersState) -> Option<&'static str> {
    let primary = mods.super_key() || mods.control_key();
    match key {
        Key::Named(NamedKey::ArrowUp) => Some("ArrowUp"),
        Key::Named(NamedKey::ArrowDown) => Some("ArrowDown"),
        Key::Named(NamedKey::Enter) => Some("Enter"),
        Key::Named(NamedKey::Escape) => Some("Escape"),
        Key::Character(s) if s.eq_ignore_ascii_case("k") && primary => Some("k"),
        _ => None,
    }
}

impl<A: DesktopApp> DesktopSession<A> {
    /// Programmatically moves focus to `next_focus` (or blurs when `None`), emitting batched
    /// `Blur`/`Focus` events and immediately re-rendering / re-rasterizing when focus changes.
    pub fn focus_node(&mut self, next_focus: Option<NodeId>) -> Result<bool, DesktopHostError> {
        let prev_focus = self.backend.focused_node();
        let focus_changed = self.backend.set_focused_node(next_focus);
        let resolved_next = self.backend.focused_node();
        if !focus_changed {
            return Ok(false);
        }
        let ts = now_us();
        let mut events = Vec::with_capacity(2);
        if let Some(old_id) = prev_focus {
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
        let encoded = encode_events(&events);
        if self.app.dispatch_events(&encoded) {
            self.render_and_rasterize()?;
        } else {
            self.rasterize_current();
        }
        Ok(true)
    }

    /// Translates a logical mouse click into focus transitions (`Blur`/`Focus`) and hit-tested
    /// `NativeEventKind::Press`, batched into one `encode_events` -> `dispatch_events` crossing.
    pub fn handle_click_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let prev_focus = self.backend.focused_node();
        let next_focus = self.backend.hit_test_focusable(x, y);
        let press_target = self.backend.hit_test(x, y);

        let ts = now_us();
        let mut events = Vec::with_capacity(3);
        let mut visual_changed = false;

        if prev_focus != next_focus {
            let changed = self.backend.set_focused_node(next_focus);
            let resolved_next = self.backend.focused_node();
            if changed {
                visual_changed = true;
                if let Some(old_id) = prev_focus {
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
        } else if next_focus.is_some() && self.backend.select_all() {
            self.backend.set_select_all(false);
            visual_changed = true;
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
            let encoded = encode_events(&events);
            self.app.dispatch_events(&encoded)
        };
        if handled {
            self.render_and_rasterize()?;
        } else if visual_changed {
            self.rasterize_current();
        }
        Ok(handled || visual_changed)
    }

    /// Dispatches a keyboard shortcut/navigation event using zero-JSON binary `NK1` framing
    /// (`nve_abi::encode_keydown_bytes`) as `NativeEventKind::Custom { name: "keydown", .. }` and
    /// re-renders when handled.
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
        let payload = encode_keydown_bytes(key, meta, ctrl, shift, alt);
        let node_id = match self.backend.focused_node() {
            Some(focused) => self.backend.find_custom_event_target_from(focused),
            None => self.backend.find_custom_event_target(),
        };
        let event = NativeEvent {
            surface_id: self.surface_id,
            node_id,
            timestamp_us: now_us(),
            kind: NativeEventKind::Custom,
            payload: EventPayload::Custom {
                name: "keydown".to_string(),
                data: payload,
            },
        };
        let encoded = encode_events(&[event]);
        let handled = self.app.dispatch_events(&encoded);
        if handled {
            self.render_and_rasterize()?;
        }
        Ok(handled)
    }

    fn emit_text_change(
        &mut self,
        node_id: NodeId,
        new_value: String,
        new_caret: usize,
    ) -> Result<bool, DesktopHostError> {
        self.backend.set_ime_preedit(None, None);
        self.backend
            .set_focused_text_override(Some(new_value.clone()), new_caret);
        let event = NativeEvent {
            surface_id: self.surface_id,
            node_id,
            timestamp_us: now_us(),
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(new_value),
        };
        let encoded = encode_events(&[event]);
        if self.app.dispatch_events(&encoded) {
            self.render_and_rasterize()?;
        } else {
            self.rasterize_current();
        }
        Ok(true)
    }

    /// Handles a `winit` keyboard press: when a `TextInput` is focused, drives full UTF-8 editing,
    /// caret movement, `Cmd/Ctrl+A`, `Enter` -> `"submit"`, and `Escape` -> blur/keydown; when
    /// unfocused, forwards global shortcuts/navigation via [`Self::handle_keydown`].
    pub fn handle_keyboard_input(
        &mut self,
        key: &Key,
        text: Option<&str>,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
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
        let select_all = self.backend.select_all();

        match classify_focused_key(key, text, modifiers) {
            FocusedKeyAction::Insert(inserted) => {
                let (next_val, next_caret) = insert_text_at(&current, caret, select_all, &inserted);
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::Backspace => {
                if self.backend.ime_preedit().is_some() {
                    self.backend.set_ime_preedit(None, None);
                    self.rasterize_current();
                    return Ok(true);
                }
                let Some((next_val, next_caret)) = delete_backward(&current, caret, select_all)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::Delete => {
                let Some((next_val, next_caret)) = delete_forward(&current, caret, select_all)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::MoveLeft => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = if select_all {
                    0
                } else {
                    prev_char_boundary(&current, caret)
                };
                let changed = next_caret != caret || select_all;
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
                let next_caret = if select_all {
                    current.len()
                } else {
                    next_char_boundary(&current, caret)
                };
                let changed = next_caret != caret || select_all;
                self.backend.set_caret_byte_index(next_caret);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveHome => {
                let changed = caret != 0 || select_all;
                self.backend.set_caret_byte_index(0);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveEnd => {
                let end = current.len();
                let changed = caret != end || select_all;
                self.backend.set_caret_byte_index(end);
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::SelectAll => {
                self.backend.set_select_all(true);
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::Submit => {
                self.backend.set_ime_preedit(None, None);
                let submit_event = NativeEvent {
                    surface_id: self.surface_id,
                    node_id: focused_id,
                    timestamp_us: now_us(),
                    kind: NativeEventKind::Custom,
                    payload: EventPayload::Custom {
                        name: "submit".to_string(),
                        data: current.into_bytes(),
                    },
                };
                let encoded = encode_events(&[submit_event]);
                if self.app.dispatch_events(&encoded) {
                    self.render_and_rasterize()?;
                    Ok(true)
                } else {
                    self.handle_keydown("Enter", meta, ctrl, shift, alt)
                }
            }
            FocusedKeyAction::Escape => {
                if self.backend.ime_preedit().is_some() {
                    self.backend.set_ime_preedit(None, None);
                    self.rasterize_current();
                    return Ok(true);
                }
                let has_custom = self.backend.has_custom_event_listener();
                let custom_target = self.backend.find_custom_event_target_from(focused_id);
                self.backend.set_focused_node(None);

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
                    let payload = encode_keydown_bytes("Escape", meta, ctrl, shift, alt);
                    events.push(NativeEvent {
                        surface_id: self.surface_id,
                        node_id: custom_target,
                        timestamp_us: ts,
                        kind: NativeEventKind::Custom,
                        payload: EventPayload::Custom {
                            name: "keydown".to_string(),
                            data: payload,
                        },
                    });
                }
                let encoded = encode_events(&events);
                if self.app.dispatch_events(&encoded) {
                    self.render_and_rasterize()?;
                } else {
                    self.rasterize_current();
                }
                Ok(true)
            }
            FocusedKeyAction::ForwardKeydown(mapped) => {
                self.handle_keydown(mapped, meta, ctrl, shift, alt)
            }
            FocusedKeyAction::None => Ok(false),
        }
    }

    /// Handles `winit::event::Ime` (`Preedit` / `Commit` / `Disabled`) for the focused `TextInput`.
    pub fn handle_ime(&mut self, ime: &Ime) -> Result<bool, DesktopHostError> {
        let Some(focused_id) = self
            .backend
            .focused_node()
            .filter(|&id| self.backend.is_text_input(id))
        else {
            return Ok(false);
        };

        match ime {
            Ime::Enabled => Ok(false),
            Ime::Preedit(text, cursor) => {
                if text.is_empty() {
                    let had = self.backend.ime_preedit().is_some();
                    self.backend.set_ime_preedit(None, None);
                    if had {
                        self.rasterize_current();
                    }
                    Ok(had)
                } else {
                    self.backend.set_ime_preedit(Some(text.clone()), *cursor);
                    self.rasterize_current();
                    Ok(true)
                }
            }
            Ime::Commit(committed) => {
                let had_preedit = self.backend.ime_preedit().is_some();
                if committed.is_empty() {
                    self.backend.set_ime_preedit(None, None);
                    if had_preedit {
                        self.rasterize_current();
                    }
                    return Ok(had_preedit);
                }
                let current = self.backend.text_input_value(focused_id).unwrap_or_default();
                let caret = self.backend.caret_byte_index();
                let select_all = self.backend.select_all();
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, select_all, committed);
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            Ime::Disabled => {
                let had = self.backend.ime_preedit().is_some();
                self.backend.set_ime_preedit(None, None);
                if had {
                    self.rasterize_current();
                }
                Ok(had)
            }
        }
    }
}
