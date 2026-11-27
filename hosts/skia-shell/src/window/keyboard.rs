//! Keyboard shortcut dispatch (`KeyDown`/`Submit`/`Escape`), focused `TextInput` UAX #29
//! editing, BiDi visual caret movement, `Shift+Arrow`/`Shift+Home`/`Shift+End` selection,
//! and clipboard (`Ctrl/Cmd+C/X/V`) handling for [`DesktopSession`].

use nve_abi::{
    native_events_to_direct, pack_key_modifiers, EventMask, EventPayload, NativeEvent,
    NativeEventKind, NodeId, NveDirectEvent, ROOT_NODE_ID,
};
use nve_backend_skia::SkiaBackend;
use nve_components::tables::Interaction;
use nve_store::NodeStore;
use winit::keyboard::{Key, ModifiersState};

use super::session::{now_us, DesktopSession};
use super::text_edit::{
    classify_focused_key, classify_unfocused_keydown, delete_backward, delete_forward,
    insert_text_at, sanitize_pasted_multiline, sanitize_pasted_single_line, FocusedKeyAction,
};
use super::{DesktopApp, DesktopHostError};

fn has_keydown_listener(store: &NodeStore, id: NodeId) -> bool {
    store.try_row::<Interaction>(id).is_some_and(|i| {
        (u64::from(i.event_mask()) & (EventMask::KEY_DOWN | EventMask::KEY_UP)) != 0
    })
}

fn find_keydown_target_from(store: &NodeStore, start: NodeId) -> NodeId {
    let mut cur = start;
    for _ in 0..256 {
        if store.is_live(cur) && has_keydown_listener(store, cur) {
            return cur;
        }
        let Some(parent) = store.parent(cur) else {
            break;
        };
        if parent == cur {
            break;
        }
        cur = parent;
    }
    find_keydown_target(store)
}

fn find_keydown_target(store: &NodeStore) -> NodeId {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if !store.is_live(id) {
            continue;
        }
        if has_keydown_listener(store, id) {
            return id;
        }
        for &cid in store.children(id).iter().rev() {
            stack.push(cid);
        }
    }
    ROOT_NODE_ID
}

fn has_any_keydown_listener(store: &NodeStore) -> bool {
    store.live_nodes().any(|id| has_keydown_listener(store, id))
}

fn move_caret_visual(backend: &SkiaBackend, text: &str, caret: usize, delta: i32) -> usize {
    let shaped = backend.text_engine().shape_single_line(
        text,
        14.0,
        nve_backend_skia::DEFAULT_FONT_WEIGHT,
    );
    shaped.move_caret_visual(text, caret, delta)
}

impl<A: DesktopApp> DesktopSession<A> {
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
        let node_id = match self.replica.backend().focused_node() {
            Some(focused) => find_keydown_target_from(self.replica.store(), focused),
            None => find_keydown_target(self.replica.store()),
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

    /// Handles a `winit` keyboard press: focused `TextInput` UAX #29 editing, BiDi visual
    /// caret movement, `Shift+Arrow`/`Shift+Home`/`Shift+End` selection,
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
            .replica
            .backend()
            .focused_node()
            .filter(|&id| {
                self.replica
                    .backend()
                    .is_text_input(self.replica.store(), id)
            })
        else {
            return if let Some(mapped) = classify_unfocused_keydown(key, modifiers) {
                self.handle_keydown(mapped, meta, ctrl, shift, alt)
            } else {
                Ok(false)
            };
        };

        let sid = focused_id;
        let current = self
            .replica
            .backend()
            .text_input_value(self.replica.store(), sid)
            .unwrap_or_default();
        let caret = self.replica.backend().focus_state().caret_byte_index;
        let anchor = self.replica.backend().focus_state().selection_anchor;
        let selection = self.replica.backend().focus_state().selection_range();
        let selected_slice = selection
            .and_then(|(s, e)| current.get(s..e))
            .map(str::to_owned);

        match classify_focused_key(key, text, modifiers) {
            FocusedKeyAction::Insert(inserted) => {
                let comp_end = self
                    .replica
                    .backend()
                    .focus_state()
                    .ime_preedit
                    .is_some()
                    .then(String::new);
                let (next_val, next_caret) = insert_text_at(&current, caret, selection, &inserted);
                self.emit_text_change(focused_id, next_val, next_caret, comp_end)
            }
            FocusedKeyAction::Backspace => {
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    {
                        let (store, backend) = self.replica.store_and_backend_mut();
                        backend.set_ime_preedit(store, None, None);
                    }
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
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    return Ok(false);
                }
                let next_caret = match selection {
                    Some((s, _)) => s,
                    None => move_caret_visual(self.replica.backend(), &current, caret, -1),
                };
                let changed = next_caret != caret || selection.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, next_caret, next_caret);
                }
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveRight => {
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    return Ok(false);
                }
                let next_caret = match selection {
                    Some((_, e)) => e,
                    None => move_caret_visual(self.replica.backend(), &current, caret, 1),
                };
                let changed = next_caret != caret || selection.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, next_caret, next_caret);
                }
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveHome => {
                let changed = caret != 0 || selection.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, 0, 0);
                }
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::MoveEnd => {
                let end = current.len();
                let changed = caret != end || selection.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, end, end);
                }
                if changed {
                    self.rasterize_current();
                }
                Ok(changed)
            }
            FocusedKeyAction::SelectLeft => {
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    return Ok(false);
                }
                let next_caret = move_caret_visual(self.replica.backend(), &current, caret, -1);
                if next_caret == caret {
                    return Ok(false);
                }
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, next_caret, anchor);
                }
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectRight => {
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    return Ok(false);
                }
                let next_caret = move_caret_visual(self.replica.backend(), &current, caret, 1);
                if next_caret == caret {
                    return Ok(false);
                }
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, next_caret, anchor);
                }
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectHome => {
                if caret == 0 {
                    return Ok(false);
                }
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, 0, anchor);
                }
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectEnd => {
                let end = current.len();
                if caret == end {
                    return Ok(false);
                }
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, end, anchor);
                }
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::SelectAll => {
                let end = current.len();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, end, 0);
                }
                self.rasterize_current();
                Ok(true)
            }
            FocusedKeyAction::Copy => {
                if self
                    .replica
                    .backend()
                    .is_secure_text_input(self.replica.store(), sid)
                {
                    return Ok(false);
                }
                if let Some(selected) = selected_slice {
                    self.clipboard.set_text(&selected);
                    return Ok(true);
                }
                Ok(false)
            }
            FocusedKeyAction::Cut => {
                let Some((s, e)) = selection else {
                    return Ok(false);
                };
                if !self
                    .replica
                    .backend()
                    .is_secure_text_input(self.replica.store(), sid)
                    && let Some(selected) = selected_slice
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
                let sanitized = if self
                    .replica
                    .backend()
                    .is_multiline_text_input(self.replica.store(), sid)
                {
                    sanitize_pasted_multiline(&raw_clip)
                } else {
                    sanitize_pasted_single_line(&raw_clip)
                };
                if sanitized.is_empty() {
                    return Ok(false);
                }
                let comp_end = self
                    .replica
                    .backend()
                    .focus_state()
                    .ime_preedit
                    .is_some()
                    .then(String::new);
                let (next_val, next_caret) =
                    insert_text_at(&current, caret, selection, &sanitized);
                self.emit_text_change(focused_id, next_val, next_caret, comp_end)
            }
            FocusedKeyAction::Submit => {
                let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_ime_preedit(store, None, None);
                }
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
                if self.replica.backend().focus_state().ime_preedit.is_some() {
                    {
                        let (store, backend) = self.replica.store_and_backend_mut();
                        backend.set_ime_preedit(store, None, None);
                    }
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
                let has_custom = has_any_keydown_listener(self.replica.store());
                let custom_target = find_keydown_target_from(self.replica.store(), focused_id);
                {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_focused_node(store, None);
                }
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
