//! Canonical ARIA/input attribute mapping (`autocomplete`, `inputmode`, `type="password"`,
//! `disabled`, `readonly`), CJK IME composition guards, and programmatic autofill sync tracker
//! shared across live DOM, SSR HTML, and `render_html`.

use std::cell::Cell;
use std::rc::Rc;

use nve_abi::hash::HashMap;
use nve_abi::{ComponentKind, NodeId, RawPropValue};
#[cfg(any(target_arch = "wasm32", test))]
use nve_abi::{EventPayload, NativeEvent, NativeEventKind, SurfaceId};

use crate::html::write_attr;

/// Virtual key code reported by browsers (`KeyboardEvent.keyCode == 229`) during active IME
/// composition, even when `isComposing` lags on WebKit/Safari.
pub const IME_PROCESS_KEY_CODE: u32 = 229;

/// Evaluates whether a [`RawPropValue`] represents a boolean-true flag (e.g. `disabled`,
/// `secureTextEntry`, `readOnly`).
#[inline]
pub fn is_truthy_raw_prop(value: &RawPropValue) -> bool {
    match value {
        RawPropValue::Bool(b) => *b,
        RawPropValue::String(s) => {
            let t = s.trim();
            !t.is_empty()
                && !matches!(
                    t.to_ascii_lowercase().as_str(),
                    "false" | "0" | "no" | "off"
                )
        }
        RawPropValue::I64(n) => *n != 0,
        RawPropValue::F64(n) => *n != 0.0,
        RawPropValue::Null | RawPropValue::Bytes(_) => false,
    }
}

/// Canonical HTML / ARIA attribute mutation produced from an NVE raw prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappedHtmlAttr<'a> {
    Set(&'static str, &'a str),
    Remove(&'static str),
    Disabled(bool),
    ReadOnly(bool),
}

/// Maps NVE accessibility and `<input>` semantics (`autocomplete`, `inputmode`, `secureTextEntry`,
/// `readOnly`, `disabled`, ARIA labels/roles) onto canonical HTML/ARIA attributes.
pub fn map_a11y_or_input_attr<'a>(
    kind: ComponentKind,
    key: &str,
    value: &'a RawPropValue,
) -> Option<MappedHtmlAttr<'a>> {
    match key {
        "accessibilityLabel" | "accessibility_label" => match value {
            RawPropValue::String(s) if !s.is_empty() => Some(MappedHtmlAttr::Set("aria-label", s)),
            _ => Some(MappedHtmlAttr::Remove("aria-label")),
        },
        "accessibilityRole" | "accessibility_role" => match value {
            RawPropValue::String(s) if !s.is_empty() => Some(MappedHtmlAttr::Set("role", s)),
            _ => Some(MappedHtmlAttr::Remove("role")),
        },
        "accessibilityHint" | "accessibility_hint" => match value {
            RawPropValue::String(s) if !s.is_empty() => {
                Some(MappedHtmlAttr::Set("aria-description", s))
            }
            _ => Some(MappedHtmlAttr::Remove("aria-description")),
        },
        "disabled" => Some(MappedHtmlAttr::Disabled(is_truthy_raw_prop(value))),
        // Only a secure input carries `type`; every other element (and a non-secure input, whose
        // default type is text) has none, as in `render_html` and SSR.
        "secureTextEntry" | "secure_text_entry" => Some(
            if kind == ComponentKind::TextInput && is_truthy_raw_prop(value) {
                MappedHtmlAttr::Set("type", "password")
            } else {
                MappedHtmlAttr::Remove("type")
            },
        ),
        "readOnly" | "read_only" | "readonly" => {
            Some(MappedHtmlAttr::ReadOnly(is_truthy_raw_prop(value)))
        }
        "autocomplete" | "autoComplete" | "auto_complete" if kind == ComponentKind::TextInput => {
            match value {
                RawPropValue::String(s) if !s.trim().is_empty() => {
                    Some(MappedHtmlAttr::Set("autocomplete", s.trim()))
                }
                _ => Some(MappedHtmlAttr::Remove("autocomplete")),
            }
        }
        "inputmode" | "inputMode" | "input_mode" if kind == ComponentKind::TextInput => {
            match value {
                RawPropValue::String(s) if !s.trim().is_empty() => {
                    Some(MappedHtmlAttr::Set("inputmode", s.trim()))
                }
                _ => Some(MappedHtmlAttr::Remove("inputmode")),
            }
        }
        _ => None,
    }
}

/// Writes canonical `autocomplete="..."` and `inputmode="..."` attributes for a `TextInput` node
/// into an HTML string (shared by both `render_ssr` and `render_html` for guaranteed parity).
pub(crate) fn write_input_hint_attrs_html(
    raw_props: &HashMap<String, RawPropValue>,
    out: &mut String,
) {
    if let Some(RawPropValue::String(ac)) = raw_props
        .get("autocomplete")
        .or_else(|| raw_props.get("autoComplete"))
        .or_else(|| raw_props.get("auto_complete"))
        && !ac.trim().is_empty()
    {
        write_attr("autocomplete", ac.trim(), out);
    }
    if let Some(RawPropValue::String(im)) = raw_props
        .get("inputmode")
        .or_else(|| raw_props.get("inputMode"))
        .or_else(|| raw_props.get("input_mode"))
        && !im.trim().is_empty()
    {
        write_attr("inputmode", im.trim(), out);
    }
}

/// Shared CJK IME composition state guard for `<input>` elements.
#[derive(Debug, Clone, Default)]
pub struct ImeCompositionGuard {
    composing: Rc<Cell<bool>>,
}

impl ImeCompositionGuard {
    pub fn new() -> Self {
        Self {
            composing: Rc::new(Cell::new(false)),
        }
    }

    #[inline]
    pub fn on_composition_start(&self) {
        self.composing.set(true);
    }

    #[inline]
    pub fn on_composition_end(&self) {
        self.composing.set(false);
    }

    #[inline]
    pub fn is_composing(&self) -> bool {
        self.composing.get()
    }

    #[inline]
    pub fn should_suppress_input(&self) -> bool {
        self.composing.get()
    }

    #[inline]
    pub fn should_suppress_keydown(&self, event_is_composing: bool, key_code: u32) -> bool {
        self.composing.get() || event_is_composing || key_code == IME_PROCESS_KEY_CODE
    }
}

/// Tracks the last synchronized value of each `<input>`/`<textarea>` node so programmatic
/// OS/browser/password-manager autofill mutations (`input` or `change` firing with zero preceding
/// `keydown` events) always emit `NativeEventKind::TextChange` without duplicate emissions when a
/// browser fires both `input` and `change` for the same value.
#[derive(Debug, Default)]
pub struct InputSyncTracker {
    last_values: HashMap<NodeId, String>,
}

impl InputSyncTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a controlled value written by the backend (`SetText` or `SetRawProp("value")`).
    #[inline]
    pub fn on_backend_value_sync(&mut self, node_id: NodeId, value: &str) {
        match self.last_values.get_mut(&node_id) {
            Some(slot) => {
                slot.clear();
                slot.push_str(value);
            }
            None => {
                self.last_values.insert(node_id, value.to_owned());
            }
        }
    }

    /// Processes a DOM `input` event (typing, paste, or autofill with zero `keydown` events).
    /// Returns `Some(value)` to emit as `TextChange`, or `None` while IME composition is active.
    pub fn on_input_event(
        &mut self,
        node_id: NodeId,
        value: String,
        ime_composing: bool,
    ) -> Option<String> {
        if ime_composing {
            return None;
        }
        self.last_values.insert(node_id, value.clone());
        Some(value)
    }

    /// Processes a DOM `change` event (e.g. Safari/iOS/1Password autofill that mutates `.value`
    /// and dispatches `change` without `keydown` or `input`). Deduplicates against the last value
    /// already synced via `input`, `compositionend`, or controlled props.
    pub fn on_change_event(
        &mut self,
        node_id: NodeId,
        value: String,
        ime_composing: bool,
    ) -> Option<String> {
        if ime_composing {
            return None;
        }
        if self.last_values.get(&node_id).map(String::as_str) == Some(value.as_str()) {
            return None;
        }
        self.last_values.insert(node_id, value.clone());
        Some(value)
    }

    /// Processes a DOM `compositionend` event, recording and returning the committed IME text.
    pub fn on_composition_end(&mut self, node_id: NodeId, value: String) -> String {
        self.last_values.insert(node_id, value.clone());
        value
    }

    /// Removes tracked state when `node_id` unmounts.
    #[inline]
    pub fn remove(&mut self, node_id: NodeId) {
        self.last_values.remove(&node_id);
    }
}

/// Pure Web IME composition lifecycle helper updating `guard` and producing unified
/// `NativeEventKind::CompositionStart` / `CompositionUpdate` / `CompositionEnd` (+ `TextChange`)
/// events shared by `wasm32` DOM listeners and native unit tests.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_composition_start(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
    data: String,
) -> NativeEvent {
    guard.on_composition_start();
    NativeEvent {
        surface_id,
        node_id,
        timestamp_us,
        kind: NativeEventKind::CompositionStart,
        payload: EventPayload::Text(data),
    }
}

/// Updates composition state and builds a unified `NativeEventKind::CompositionUpdate` event.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_composition_update(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
    data: String,
) -> NativeEvent {
    guard.on_composition_start();
    NativeEvent {
        surface_id,
        node_id,
        timestamp_us,
        kind: NativeEventKind::CompositionUpdate,
        payload: EventPayload::Text(data),
    }
}

/// Finalizes Web IME composition, clearing `guard`, synchronizing `input_sync`, and returning
/// `[CompositionEnd(committed_piece), TextChange(full_input_value)]`.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_composition_end(
    guard: &ImeCompositionGuard,
    input_sync: &mut InputSyncTracker,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
    committed_piece: String,
    dom_value: String,
) -> [NativeEvent; 2] {
    guard.on_composition_end();
    let text = input_sync.on_composition_end(node_id, dom_value);
    [
        NativeEvent {
            surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::CompositionEnd,
            payload: EventPayload::Text(committed_piece),
        },
        NativeEvent {
            surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(text),
        },
    ]
}

/// Cancels active Web IME composition on blur (`focusout`), returning a clean `CompositionEnd`
/// event when composition was in progress.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_composition_cancel_on_blur(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
) -> Option<NativeEvent> {
    if !guard.is_composing() {
        return None;
    }
    guard.on_composition_end();
    Some(NativeEvent {
        surface_id,
        node_id,
        timestamp_us,
        kind: NativeEventKind::CompositionEnd,
        payload: EventPayload::Text(String::new()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_autocomplete_and_inputmode_only_for_text_inputs() {
        let otp = RawPropValue::String("one-time-code".into());
        let num = RawPropValue::String("numeric".into());
        assert_eq!(
            map_a11y_or_input_attr(ComponentKind::TextInput, "autocomplete", &otp),
            Some(MappedHtmlAttr::Set("autocomplete", "one-time-code"))
        );
        assert_eq!(
            map_a11y_or_input_attr(ComponentKind::TextInput, "autoComplete", &otp),
            Some(MappedHtmlAttr::Set("autocomplete", "one-time-code"))
        );
        assert_eq!(
            map_a11y_or_input_attr(ComponentKind::TextInput, "inputMode", &num),
            Some(MappedHtmlAttr::Set("inputmode", "numeric"))
        );
        assert_eq!(
            map_a11y_or_input_attr(ComponentKind::TextInput, "autocomplete", &RawPropValue::Null),
            Some(MappedHtmlAttr::Remove("autocomplete"))
        );
        assert_eq!(
            map_a11y_or_input_attr(ComponentKind::View, "autocomplete", &otp),
            None
        );
    }

    #[test]
    fn input_sync_tracker_handles_zero_keydown_autofill_and_deduplicates_change() {
        let mut tracker = InputSyncTracker::new();
        // 1. OS autofill fires ONLY `change` (zero `keydown`, zero `input`) -> emits TextChange.
        assert_eq!(
            tracker.on_change_event(7, "482910".into(), false),
            Some("482910".into())
        );
        // Repeated `change` with unchanged value is deduplicated.
        assert_eq!(tracker.on_change_event(7, "482910".into(), false), None);

        // 2. Chrome autofill fires `input` then `change` with identical value (zero `keydown`).
        assert_eq!(
            tracker.on_input_event(8, "alice@example.com".into(), false),
            Some("alice@example.com".into())
        );
        assert_eq!(
            tracker.on_change_event(8, "alice@example.com".into(), false),
            None
        );

        // 3. Controlled backend reset clears field back to ""; subsequent `change` autofill emits.
        tracker.on_backend_value_sync(8, "");
        assert_eq!(
            tracker.on_change_event(8, "alice@example.com".into(), false),
            Some("alice@example.com".into())
        );

        // 4. Active IME composition suppresses both `input` and `change`, then commits on end.
        assert_eq!(tracker.on_input_event(9, "ni".into(), true), None);
        assert_eq!(tracker.on_change_event(9, "ni".into(), true), None);
        assert_eq!(tracker.on_composition_end(9, "你".into()), "你");
        assert_eq!(tracker.on_change_event(9, "你".into(), false), None);
    }

    #[test]
    fn web_composition_helpers_emit_unified_events_and_preserve_guard() {
        let guard = ImeCompositionGuard::new();
        let mut sync = InputSyncTracker::new();

        let start = web_composition_start(&guard, 1, 42, 100, "ni".into());
        assert!(guard.is_composing());
        assert!(guard.should_suppress_input());
        assert!(guard.should_suppress_keydown(false, 13));
        assert_eq!(start.kind, NativeEventKind::CompositionStart);
        assert_eq!(start.payload, EventPayload::Text("ni".into()));

        let update = web_composition_update(&guard, 1, 42, 200, "nihao".into());
        assert!(guard.is_composing());
        assert_eq!(update.kind, NativeEventKind::CompositionUpdate);
        assert_eq!(update.payload, EventPayload::Text("nihao".into()));

        let [end, change] =
            web_composition_end(&guard, &mut sync, 1, 42, 300, "你好".into(), "hi 你好".into());
        assert!(!guard.is_composing());
        assert!(!guard.should_suppress_input());
        assert_eq!(end.kind, NativeEventKind::CompositionEnd);
        assert_eq!(end.payload, EventPayload::Text("你好".into()));
        assert_eq!(change.kind, NativeEventKind::TextChange);
        assert_eq!(change.payload, EventPayload::Text("hi 你好".into()));

        // Blur mid-composition cancels cleanly and resets guard.
        assert!(web_composition_cancel_on_blur(&guard, 1, 42, 400).is_none());
        let _ = web_composition_start(&guard, 1, 42, 500, "ka".into());
        let blur_end = web_composition_cancel_on_blur(&guard, 1, 42, 600).expect("cancel event");
        assert!(!guard.is_composing());
        assert_eq!(blur_end.kind, NativeEventKind::CompositionEnd);
        assert_eq!(blur_end.payload, EventPayload::Text(String::new()));
    }
}
