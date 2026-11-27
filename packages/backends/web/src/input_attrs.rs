//! Web IME composition guard, controlled-input echo suppression (`InputSyncTracker`), and
//! `application/x-www-form-urlencoded` form submission decoder.

use std::cell::Cell;
use std::collections::BTreeMap;

use nve_abi::hash::HashMap;
use nve_abi::{EventPayload, NativeEvent, NativeEventKind, NodeId, SurfaceId};

/// Standard W3C UI Events `keyCode` for IME composition keystrokes (`Process` key).
pub const IME_PROCESS_KEY_CODE: u32 = 229;

/// Tracks active CJK/dead-key IME composition state so `input` / `keydown` Enter commits do not
/// prematurely trigger search/submit callbacks.
#[derive(Debug, Default)]
pub struct ImeCompositionGuard {
    composing: Cell<bool>,
}

impl ImeCompositionGuard {
    #[inline]
    pub fn new() -> Self {
        Self {
            composing: Cell::new(false),
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

/// Tracks the last value observed per `<input>` / `<textarea>` node so programmatic backend
/// value syncs never echo spurious `ChangeText` events and caret positions stay stable.
#[derive(Debug, Default)]
pub struct InputSyncTracker {
    last_values: HashMap<NodeId, String>,
}

impl InputSyncTracker {
    #[inline]
    pub fn new() -> Self {
        Self {
            last_values: HashMap::default(),
        }
    }

    #[inline]
    pub fn on_backend_value_sync(&mut self, node_id: NodeId, value: &str) {
        if let Some(prev) = self.last_values.get_mut(&node_id) {
            prev.clear();
            prev.push_str(value);
        } else {
            self.last_values.insert(node_id, value.to_owned());
        }
    }

    /// Commits a backend-driven control value. Adopted SSR controls still awaiting 3-way
    /// hydration reconciliation (`deferred`) keep the user's pre-hydration DOM value, so neither
    /// the DOM nor this echo baseline may take `value` yet (else typing the server value back
    /// would be swallowed as an echo). Returns whether the caller should write `value` to the DOM.
    #[inline]
    pub fn on_backend_value_commit(&mut self, node_id: NodeId, value: &str, deferred: bool) -> bool {
        if deferred {
            return false;
        }
        self.on_backend_value_sync(node_id, value);
        true
    }

    #[inline]
    pub fn on_input_event(
        &mut self,
        node_id: NodeId,
        value: String,
        is_composing: bool,
    ) -> Option<String> {
        if is_composing {
            return None;
        }
        if self.last_values.get(&node_id).is_some_and(|p| p == &value) {
            return None;
        }
        self.last_values.insert(node_id, value.clone());
        Some(value)
    }

    #[inline]
    pub fn on_change_event(&mut self, node_id: NodeId, value: String) -> Option<String> {
        if self.last_values.get(&node_id).is_some_and(|p| p == &value) {
            return None;
        }
        self.last_values.insert(node_id, value.clone());
        Some(value)
    }

    #[inline]
    pub fn on_composition_end(&mut self, node_id: NodeId, value: String) -> String {
        self.last_values.insert(node_id, value.clone());
        value
    }

    #[inline]
    pub fn remove(&mut self, node_id: NodeId) {
        self.last_values.remove(&node_id);
    }
}

pub fn web_composition_start(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
) -> NativeEvent {
    guard.on_composition_start();
    NativeEvent {
        kind: NativeEventKind::CompositionStart,
        surface_id,
        node_id,
        timestamp_us,
        payload: EventPayload::Empty,
    }
}

pub fn web_composition_update(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
    preedit: String,
) -> NativeEvent {
    if !guard.is_composing() {
        guard.on_composition_start();
    }
    NativeEvent {
        kind: NativeEventKind::CompositionUpdate,
        surface_id,
        node_id,
        timestamp_us,
        payload: EventPayload::Text(preedit),
    }
}

pub fn web_composition_end(
    guard: &ImeCompositionGuard,
    tracker: &mut InputSyncTracker,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp_us: u64,
    committed: String,
    full_value: String,
) -> [NativeEvent; 2] {
    guard.on_composition_end();
    let synced = tracker.on_composition_end(node_id, full_value);
    [
        NativeEvent {
            kind: NativeEventKind::CompositionEnd,
            surface_id,
            node_id,
            timestamp_us,
            payload: EventPayload::Text(committed),
        },
        NativeEvent {
            kind: NativeEventKind::TextChange,
            surface_id,
            node_id,
            timestamp_us,
            payload: EventPayload::Text(synced),
        },
    ]
}

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
        kind: NativeEventKind::CompositionEnd,
        surface_id,
        node_id,
        timestamp_us,
        payload: EventPayload::Empty,
    })
}

/// Decodes an `application/x-www-form-urlencoded` form submission payload into a deterministic map.
pub fn decode_form_urlencoded_map(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        out.insert(percent_decode_plus(k), percent_decode_plus(v));
    }
    out
}

fn percent_decode_plus(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[inline]
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
