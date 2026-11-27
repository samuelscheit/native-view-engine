//! Interactive pointer, drag/double-click/triple-click range selection, and focus/blur dispatch
//! for [`DesktopSession`]. Keyboard shortcut and focused `TextInput` editing live in
//! [`super::keyboard`].

use std::time::{Duration, Instant};

use nve_abi::{
    native_events_to_direct, EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId,
};
use nve_backend_skia::SkiaBackend;
use nve_components::tables::{is_subtree_display_none, Interaction, TextInput};
use nve_store::{NodeId as StoreNodeId, NodeStore};

use super::session::{now_us, DesktopSession};
pub(super) use super::text_edit::{paragraph_bounds_at, word_bounds_at};
#[cfg(test)]
pub(super) use super::text_edit::{
    format_keydown_shorthand, normalize_line_endings, sanitize_pasted_multiline,
    sanitize_pasted_single_line,
};
use super::{DesktopApp, DesktopHostError};

const MULTI_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(400);
const MULTI_CLICK_MAX_DIST_SQ: f32 = 36.0; // 6.0 logical px

pub(super) fn is_focusable(store: &NodeStore, id: StoreNodeId) -> bool {
    if !store.is_live(id) || is_subtree_display_none(store, id) {
        return false;
    }
    if store.has::<TextInput>(id) {
        return true;
    }
    store.try_row::<Interaction>(id).is_some_and(|i| {
        (u64::from(i.event_mask()) & (EventMask::FOCUS | EventMask::BLUR)) != 0
    })
}

fn hit_test_focusable(store: &NodeStore, backend: &SkiaBackend, x: f32, y: f32) -> Option<NodeId> {
    let mut cur = backend.hit_test_interaction(store, x, y).or_else(|| backend.hit_test(store, x, y))?;
    for _ in 0..256 {
        let sid = StoreNodeId(cur);
        if is_focusable(store, sid) {
            return Some(cur);
        }
        let Some(parent) = store.parent(sid) else {
            break;
        };
        if parent.raw() == cur {
            break;
        }
        cur = parent.raw();
    }
    None
}


impl<A: DesktopApp> DesktopSession<A> {
    pub(super) fn reset_multi_click(&mut self) {
        self.last_click_time = None;
        self.last_click_node = None;
        self.last_click_count = 0;
    }

    /// Programmatically moves focus to `next_focus` (or blurs when `None`), emitting balanced
    /// `CompositionEnd` (when blurring mid-composition) + `Blur`/`Focus` events and re-rendering.
    #[cfg(test)]
    pub fn focus_node(&mut self, next_focus: Option<NodeId>) -> Result<bool, DesktopHostError> {
        let prev_focus = self.replica.backend().focused_node();
        let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
        let next_valid = next_focus.filter(|&id| is_focusable(self.replica.store(), StoreNodeId(id)));
        {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.set_focused_node(store, next_valid);
        }
        let resolved_next = self.replica.backend().focused_node();
        if resolved_next == prev_focus {
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
        let prev_focus = self.replica.backend().focused_node();
        let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
        let next_focus = hit_test_focusable(self.replica.store(), self.replica.backend(), x, y);
        let press_target = self
            .replica
            .backend()
            .hit_test_interaction(self.replica.store(), x, y)
            .or_else(|| self.replica.backend().hit_test(self.replica.store(), x, y));

        let ts = now_us();
        let mut events = Vec::with_capacity(4);
        let mut visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_down(store, x, y)
        };

        if prev_focus != next_focus {
            {
                let (store, backend) = self.replica.store_and_backend_mut();
                backend.set_focused_node(store, next_focus);
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
            .replica
            .backend()
            .focused_node()
            .filter(|&id| {
                self.replica
                    .backend()
                    .is_text_input(self.replica.store(), StoreNodeId(id))
            })
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
                .replica
                .backend()
                .cursor_index_at_x(self.replica.store(), StoreNodeId(text_id), x)
                .unwrap_or_else(|| self.replica.backend().focus_state().caret_byte_index);
            let prev_sel = (
                self.replica.backend().focus_state().selection_anchor,
                self.replica.backend().focus_state().caret_byte_index,
            );

            match click_count {
                2 => {
                    let val = self
                        .replica
                        .backend()
                        .text_input_value(self.replica.store(), StoreNodeId(text_id))
                        .unwrap_or_default();
                    let (w_start, w_end) = word_bounds_at(&val, hit_idx);
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, w_end, w_start);
                    self.dragging_text_node = None;
                }
                3 => {
                    let val = self
                        .replica
                        .backend()
                        .text_input_value(self.replica.store(), StoreNodeId(text_id))
                        .unwrap_or_default();
                    let (p_start, p_end) = paragraph_bounds_at(&val, hit_idx);
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, p_end, p_start);
                    self.dragging_text_node = None;
                }
                _ => {
                    let (store, backend) = self.replica.store_and_backend_mut();
                    backend.set_caret_and_selection(store, hit_idx, hit_idx);
                    self.dragging_text_node = Some(text_id);
                }
            }
            self.last_click_time = Some(now);
            self.last_click_pos = (x, y);
            self.last_click_node = Some(text_id);
            self.last_click_count = click_count;

            let next_sel = (
                self.replica.backend().focus_state().selection_anchor,
                self.replica.backend().focus_state().caret_byte_index,
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
        let mut visual_changed = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_move(store, x, y)
        };
        if let Some(node_id) = self.dragging_text_node {
            let is_valid = self.replica.backend().focused_node() == Some(node_id)
                && self
                    .replica
                    .backend()
                    .is_text_input(self.replica.store(), StoreNodeId(node_id));
            if !is_valid {
                self.dragging_text_node = None;
            } else if let Some(next_caret) = self
                .replica
                .backend()
                .cursor_index_at_x(self.replica.store(), StoreNodeId(node_id), x)
                && next_caret != self.replica.backend().focus_state().caret_byte_index
            {
                let anchor = self.replica.backend().focus_state().selection_anchor;
                let (store, backend) = self.replica.store_and_backend_mut();
                backend.set_caret_and_selection(store, next_caret, anchor);
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
        let dirty = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_leave(store)
        };
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
        let dirty = {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.pointer_up(store, x, y)
        };
        if dirty {
            self.rasterize_current();
        }
        dirty
    }

    /// Finishes any in-progress mouse drag selection and releases active press state.
    #[cfg(test)]
    pub fn handle_pointer_up_logical(&mut self) {
        let _ = self.handle_pointer_up_with_redraw();
    }
}
