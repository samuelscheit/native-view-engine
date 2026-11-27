//! Pure UAX #29 grapheme-safe UTF-8 text editing, word/paragraph selection, paste line-ending
//! normalization, key-action classification, and `winit` cursor icon mapping helpers shared by
//! `hosts/skia-shell/src/window/`.

use nve_backend_skia::CursorKind;
pub use nve_backend_skia::{
    clamp_grapheme_boundary, next_grapheme_boundary, normalize_line_endings, paragraph_bounds_at,
    prev_grapheme_boundary, word_bounds_at,
};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::CursorIcon;

/// Translates an NVE [`CursorKind`] into the corresponding native desktop `winit::window::CursorIcon`.
#[inline]
pub fn cursor_kind_to_winit(kind: CursorKind) -> CursorIcon {
    match kind {
        CursorKind::Auto | CursorKind::Default => CursorIcon::Default,
        CursorKind::Pointer => CursorIcon::Pointer,
        CursorKind::Text => CursorIcon::Text,
        CursorKind::Grab => CursorIcon::Grab,
        CursorKind::Grabbing => CursorIcon::Grabbing,
        CursorKind::NotAllowed => CursorIcon::NotAllowed,
        CursorKind::ColResize => CursorIcon::ColResize,
        CursorKind::RowResize => CursorIcon::RowResize,
        CursorKind::Move => CursorIcon::Move,
        CursorKind::Crosshair => CursorIcon::Crosshair,
        CursorKind::Progress => CursorIcon::Progress,
        CursorKind::Wait => CursorIcon::Wait,
    }
}

/// Formats a keyboard event into NVE's binary compact modifier shorthand
/// (e.g. `"Cmd+k"`, `"Ctrl+Shift+Enter"`, `"ArrowDown"`, `"Escape"`).
#[cfg(test)]
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

/// Inserts `inserted` at a UAX #29 grapheme-clamped `caret` (or replacing `selection`), returning
/// `(new_text, new_caret_byte_index)`.
pub fn insert_text_at(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
    inserted: &str,
) -> (String, usize) {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_grapheme_boundary(current, raw_s);
        let e = clamp_grapheme_boundary(current, raw_e.max(raw_s));
        let mut out = String::with_capacity(current.len() - (e - s) + inserted.len());
        out.push_str(&current[..s]);
        out.push_str(inserted);
        out.push_str(&current[e..]);
        return (out, s + inserted.len());
    }
    let idx = clamp_grapheme_boundary(current, caret);
    let mut out = String::with_capacity(current.len() + inserted.len());
    out.push_str(&current[..idx]);
    out.push_str(inserted);
    out.push_str(&current[idx..]);
    (out, idx + inserted.len())
}

/// Deletes the active selection or the single UAX #29 extended grapheme cluster immediately
/// before `caret`. Never splits ZWJ emoji, regional flags, skin-tone modifiers, or combining marks.
pub fn delete_backward(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
) -> Option<(String, usize)> {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_grapheme_boundary(current, raw_s);
        let e = clamp_grapheme_boundary(current, raw_e.max(raw_s));
        if s < e {
            let mut out = String::with_capacity(current.len() - (e - s));
            out.push_str(&current[..s]);
            out.push_str(&current[e..]);
            return Some((out, s));
        }
    }
    let idx = clamp_grapheme_boundary(current, caret);
    if idx == 0 {
        return None;
    }
    let prev = prev_grapheme_boundary(current, idx);
    let mut out = String::with_capacity(current.len() - (idx - prev));
    out.push_str(&current[..prev]);
    out.push_str(&current[idx..]);
    Some((out, prev))
}

/// Deletes the active selection or the single UAX #29 extended grapheme cluster immediately
/// after `caret`. Never splits ZWJ emoji, regional flags, skin-tone modifiers, or combining marks.
pub fn delete_forward(
    current: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
) -> Option<(String, usize)> {
    if let Some((raw_s, raw_e)) = selection {
        let s = clamp_grapheme_boundary(current, raw_s);
        let e = clamp_grapheme_boundary(current, raw_e.max(raw_s));
        if s < e {
            let mut out = String::with_capacity(current.len() - (e - s));
            out.push_str(&current[..s]);
            out.push_str(&current[e..]);
            return Some((out, s));
        }
    }
    let idx = clamp_grapheme_boundary(current, caret);
    if idx >= current.len() {
        return None;
    }
    let next = next_grapheme_boundary(current, idx);
    let mut out = String::with_capacity(current.len() - (next - idx));
    out.push_str(&current[..idx]);
    out.push_str(&current[next..]);
    Some((out, idx))
}

#[inline]
pub fn is_printable_text(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control())
}

/// Normalizes `\r\n` and `\r` to `\n`, collapses line breaks and tabs into single spaces for
/// single-line `TextInput` paste, and strips non-printable control characters.
pub fn sanitize_pasted_single_line(raw: &str) -> String {
    let normalized = normalize_line_endings(raw);
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch == '\n' || ch == '\t' {
            out.push(' ');
        } else if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// Normalizes `\r\n` and `\r` into `\n` for multiline `TextInput` paste while preserving `\n` and
/// `\t` and stripping other non-printable control characters.
pub fn sanitize_pasted_multiline(raw: &str) -> String {
    let normalized = normalize_line_endings(raw);
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
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
