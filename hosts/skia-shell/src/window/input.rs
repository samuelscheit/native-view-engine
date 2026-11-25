//! Interactive pointer, drag/double-click range selection, scroll, focus/blur, keyboard,
//! clipboard (`Ctrl/Cmd+C/X/V`), UTF-8 text editing, IME, and zero-JSON compact keydown shorthand
//! dispatch for [`DesktopSession`].

use std::time::{Duration, Instant};

use nve_abi::events::encode_keydown_bytes;
use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, NodeId};
use winit::event::Ime;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::session::{now_us, DesktopSession};
use super::{DesktopApp, DesktopHostError};

const DOUBLE_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_MAX_DIST_SQ: f32 = 36.0; // 6.0 logical px

/// Formats a keyboard event into NVE's zero-JSON compact modifier shorthand
/// (e.g. `"Cmd+k"`, `"Ctrl+Shift+Enter"`, `"ArrowDown"`, `"Escape"`).
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Word,
    Whitespace,
    Punct,
}

#[inline]
fn classify_char(c: char) -> CharClass {
    if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Whitespace
    } else {
        CharClass::Punct
    }
}

/// Computes the `(start, end)` UTF-8 byte range of the word (or contiguous whitespace/punctuation
/// run) surrounding `byte_index` for double-click selection.
pub fn word_bounds_at(s: &str, byte_index: usize) -> (usize, usize) {
    if s.is_empty() {
        return (0, 0);
    }
    let mut idx = clamp_char_boundary(s, byte_index);
    if idx == s.len() {
        idx = prev_char_boundary(s, idx);
    }
    let Some(target_ch) = s[idx..].chars().next() else {
        return (0, 0);
    };
    let target_class = classify_char(target_ch);

    let mut start = idx;
    while start > 0 {
        let prev = prev_char_boundary(s, start);
        let ch = s[prev..start].chars().next().unwrap();
        if classify_char(ch) != target_class {
            break;
        }
        start = prev;
    }

    let mut end = idx;
    while end < s.len() {
        let next = next_char_boundary(s, end);
        let ch = s[end..next].chars().next().unwrap();
        if classify_char(ch) != target_class {
            break;
        }
        end = next;
    }

    (start, end)
}

pub fn insert_text_at(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
    inserted: &str,
) -> (String, usize) {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_char_boundary(current, raw_s);
        let e = clamp_char_boundary(current, raw_e.max(raw_s));
        let mut out = String::with_capacity(current.len() - (e - s) + inserted.len());
        out.push_str(&current[..s]);
        out.push_str(inserted);
        out.push_str(&current[e..]);
        return (out, s + inserted.len());
    }
    let idx = clamp_char_boundary(current, caret);
    let mut out = String::with_capacity(current.len() + inserted.len());
    out.push_str(&current[..idx]);
    out.push_str(inserted);
    out.push_str(&current[idx..]);
    (out, idx + inserted.len())
}

pub fn delete_backward(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
) -> Option<(String, usize)> {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_char_boundary(current, raw_s);
        let e = clamp_char_boundary(current, raw_e.max(raw_s));
        if s < e {
            let mut out = String::with_capacity(current.len() - (e - s));
            out.push_str(&current[..s]);
            out.push_str(&current[e..]);
            return Some((out, s));
        }
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

pub fn delete_forward(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
) -> Option<(String, usize)> {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_char_boundary(current, raw_s);
        let e = clamp_char_boundary(current, raw_e.max(raw_s));
        if s < e {
            let mut out = String::with_capacity(current.len() - (e - s));
            out.push_str(&current[..s]);
            out.push_str(&current[e..]);
            return Some((out, s));
        }
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

fn sanitize_pasted_single_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_cr = false;
    for ch in raw.chars() {
        if ch == '\r' {
            out.push(' ');
            prev_cr = true;
        } else if ch == '\n' {
            if !prev_cr {
                out.push(' ');
            }
            prev_cr = false;
        } else {
            prev_cr = false;
            if ch == '\t' {
                out.push(' ');
            } else if !ch.is_control() {
                out.push(ch);
            }
        }
    }
    out
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
    SelectLeft,
    SelectRight,
    SelectHome,
    SelectEnd,
    SelectAll,
    Copy,
    Cut,
    Paste,
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
        Key::Named(NamedKey::ArrowLeft) => match (meta, shift) {
            (true, true) => FocusedKeyAction::SelectHome,
            (true, false) => FocusedKeyAction::MoveHome,
            (false, true) => FocusedKeyAction::SelectLeft,
            (false, false) => FocusedKeyAction::MoveLeft,
        },
        Key::Named(NamedKey::ArrowRight) => match (meta, shift) {
            (true, true) => FocusedKeyAction::SelectEnd,
            (true, false) => FocusedKeyAction::MoveEnd,
            (false, true) => FocusedKeyAction::SelectRight,
            (false, false) => FocusedKeyAction::MoveRight,
        },
        Key::Named(NamedKey::Home) => {
            if shift {
                FocusedKeyAction::SelectHome
            } else {
                FocusedKeyAction::MoveHome
            }
        }
        Key::Named(NamedKey::End) => {
            if shift {
                FocusedKeyAction::SelectEnd
            } else {
                FocusedKeyAction::MoveEnd
            }
        }
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
            if primary && !alt && !shift {
                if ch.eq_ignore_ascii_case("a") {
                    return FocusedKeyAction::SelectAll;
                }
                if ch.eq_ignore_ascii_case("c") {
                    return FocusedKeyAction::Copy;
                }
                if ch.eq_ignore_ascii_case("x") {
                    return FocusedKeyAction::Cut;
                }
                if ch.eq_ignore_ascii_case("v") {
                    return FocusedKeyAction::Paste;
                }
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
            if !primary
                && let Some(t) = text.filter(|s| is_printable_text(s))
            {
                return FocusedKeyAction::Insert(t.to_owned());
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
        self.dragging_text_node = None;
        self.last_click_time = None;
        self.last_click_node = None;
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

    /// Translates a logical mouse press into focus transitions (`Blur`/`Focus`), `TextInput`
    /// caret/anchor placement (or double-click word selection), and hit-tested `Press` events.
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
        }

        // Position caret/anchor or select word on double-click when pressing inside a TextInput.
        if let Some(text_id) = self
            .backend
            .focused_node()
            .filter(|&id| self.backend.is_text_input(id))
            && next_focus == Some(text_id)
        {
            let now = Instant::now();
            let dx = x - self.last_click_pos.0;
            let dy = y - self.last_click_pos.1;
            let is_double_click = self.last_click_node == Some(text_id)
                && self
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK_MAX_INTERVAL)
                && (dx * dx + dy * dy) <= DOUBLE_CLICK_MAX_DIST_SQ;

            let hit_idx = self
                .backend
                .cursor_index_at_x(text_id, x)
                .unwrap_or_else(|| self.backend.caret_byte_index());
            let prev_sel = (
                self.backend.selection_anchor_byte_index(),
                self.backend.caret_byte_index(),
            );

            if is_double_click {
                let val = self.backend.text_input_value(text_id).unwrap_or_default();
                let (w_start, w_end) = word_bounds_at(&val, hit_idx);
                self.backend.set_selection_range(w_start, w_end);
                self.dragging_text_node = None;
                self.last_click_time = None;
                self.last_click_node = None;
            } else {
                self.backend.set_selection_range(hit_idx, hit_idx);
                self.dragging_text_node = Some(text_id);
                self.last_click_time = Some(now);
                self.last_click_pos = (x, y);
                self.last_click_node = Some(text_id);
            }

            let next_sel = (
                self.backend.selection_anchor_byte_index(),
                self.backend.caret_byte_index(),
            );
            if next_sel != prev_sel {
                visual_changed = true;
            }
        } else {
            self.dragging_text_node = None;
            self.last_click_time = None;
            self.last_click_node = None;
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

    /// Updates cursor position and, while dragging inside a focused `TextInput`, extends the
    /// active range selection from the press anchor to the hovered character boundary.
    pub fn handle_pointer_move_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let Some(node_id) = self.dragging_text_node else {
            return Ok(false);
        };
        if self.backend.focused_node() != Some(node_id) || !self.backend.is_text_input(node_id) {
            self.dragging_text_node = None;
            return Ok(false);
        }
        let Some(next_caret) = self.backend.cursor_index_at_x(node_id, x) else {
            return Ok(false);
        };
        let anchor = self.backend.selection_anchor_byte_index();
        if next_caret == self.backend.caret_byte_index() {
            return Ok(false);
        }
        self.backend.set_selection_range(anchor, next_caret);
        self.rasterize_current();
        Ok(true)
    }

    /// Finishes any in-progress mouse drag selection.
    pub fn handle_pointer_up_logical(&mut self) {
        self.dragging_text_node = None;
    }

    /// Dispatches a keyboard shortcut/navigation event using zero-JSON binary `NK1` framing.
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
    /// caret/selection movement (`Shift+Left/Right/Home/End`), clipboard (`Ctrl/Cmd+C/X/V`),
    /// `Cmd/Ctrl+A`, `Enter` -> `"submit"`, and `Escape` -> blur/keydown.
    pub fn handle_keyboard_input(
        &mut self,
        key: &Key,
        text: Option<&str>,
        modifiers: ModifiersState,
    ) -> Result<bool, DesktopHostError> {
        self.last_click_time = None;
        self.last_click_node = None;
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
                let (next_val, next_caret) = insert_text_at(&current, caret, selection, &inserted);
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::Backspace => {
                if self.backend.ime_preedit().is_some() {
                    self.backend.set_ime_preedit(None, None);
                    self.rasterize_current();
                    return Ok(true);
                }
                let Some((next_val, next_caret)) = delete_backward(&current, caret, selection)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::Delete => {
                let Some((next_val, next_caret)) = delete_forward(&current, caret, selection)
                else {
                    return Ok(false);
                };
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::MoveLeft => {
                if self.backend.ime_preedit().is_some() {
                    return Ok(false);
                }
                let next_caret = match selection {
                    Some((s, _)) => s,
                    None => prev_char_boundary(&current, caret),
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
                    None => next_char_boundary(&current, caret),
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
                let next_caret = prev_char_boundary(&current, caret);
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
                let next_caret = next_char_boundary(&current, caret);
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
                self.emit_text_change(focused_id, next_val, next_caret)
            }
            FocusedKeyAction::Paste => {
                let Some(raw_clip) = self.clipboard.get_text() else {
                    return Ok(false);
                };
                let sanitized = sanitize_pasted_single_line(&raw_clip);
                if sanitized.is_empty() {
                    return Ok(false);
                }
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, selection, &sanitized);
                self.emit_text_change(focused_id, next_val, next_caret)
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
        self.last_click_time = None;
        self.last_click_node = None;
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
                let selection = self.backend.selection_range();
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, selection, committed);
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
