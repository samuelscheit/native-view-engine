//! Web IME composition guard, controlled-input echo suppression (`InputSyncTracker`), and live
//! `<input>` / `<textarea>` / `<select>` `.value` access.

use std::cell::Cell;

use nve_abi::hash::HashMap;
use nve_abi::{EventBatchBuf, NodeId, SurfaceId, TimestampUs};

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
    timestamp: TimestampUs,
    out: &mut EventBatchBuf,
) {
    guard.on_composition_start();
    out.push_composition(
        surface_id,
        node_id,
        timestamp,
        nve_abi::TextInputEvent::CompositionStart,
        "",
    );
}

pub fn web_composition_update(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp: TimestampUs,
    preedit: &str,
    out: &mut EventBatchBuf,
) {
    if !guard.is_composing() {
        guard.on_composition_start();
    }
    out.push_composition(
        surface_id,
        node_id,
        timestamp,
        nve_abi::TextInputEvent::CompositionUpdate,
        preedit,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn web_composition_end(
    guard: &ImeCompositionGuard,
    tracker: &mut InputSyncTracker,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp: TimestampUs,
    committed: &str,
    full_value: String,
    out: &mut EventBatchBuf,
) {
    guard.on_composition_end();
    let synced = tracker.on_composition_end(node_id, full_value);
    out.push_composition(
        surface_id,
        node_id,
        timestamp,
        nve_abi::TextInputEvent::CompositionEnd,
        committed,
    );
    out.push_text_change(surface_id, node_id, timestamp, &synced);
}

pub fn web_composition_cancel_on_blur(
    guard: &ImeCompositionGuard,
    surface_id: SurfaceId,
    node_id: NodeId,
    timestamp: TimestampUs,
    out: &mut EventBatchBuf,
) -> bool {
    if !guard.is_composing() {
        return false;
    }
    guard.on_composition_end();
    out.push_composition(
        surface_id,
        node_id,
        timestamp,
        nve_abi::TextInputEvent::CompositionEnd,
        "",
    );
    true
}

/// Live `.value` of an `<input>`, `<textarea>`, or `<select>` (`None` for any other element).
#[cfg(target_arch = "wasm32")]
pub(crate) fn read_dom_control_value(el: &web_sys::Element) -> Option<String> {
    use wasm_bindgen::JsCast;
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        Some(inp.value())
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        Some(ta.value())
    } else {
        el.dyn_ref::<web_sys::HtmlSelectElement>().map(web_sys::HtmlSelectElement::value)
    }
}

/// Writes `.value` on a live `<input>`, `<textarea>`, or `<select>` only when it differs, so an
/// unchanged value never resets the caret or selection.
#[cfg(target_arch = "wasm32")]
pub(crate) fn write_dom_control_value(el: &web_sys::Element, val: &str) {
    use wasm_bindgen::JsCast;
    if let Some(inp) = el.dyn_ref::<web_sys::HtmlInputElement>() {
        if inp.value() != val {
            inp.set_value(val);
        }
    } else if let Some(ta) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        if ta.value() != val {
            ta.set_value(val);
        }
    } else if let Some(sel) = el.dyn_ref::<web_sys::HtmlSelectElement>()
        && sel.value() != val
    {
        sel.set_value(val);
    }
}
