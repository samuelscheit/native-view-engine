//! Focus/blur dispatch, `TextInput` click/multi-click selection, and scroll input for
//! [`DesktopSession`]. Pointer lifecycle (`PointerDown`/`Move`/`Up`/`Enter`/`Leave`/`Cancel`,
//! declarative capture, and `Press`-on-release) lives in [`super::pointer`].

use std::time::{Duration, Instant};

use nve_abi::{EventBatchBuf, InteractionEvent, NodeId, TextInputEvent, TimestampUs};
use nve_backend_skia::FocusModality;
use nve_layout::is_overlay_closed;
use nve_store::NodeStore;
use nve_tables::tables::{is_subtree_display_none, InputKind, Interaction, TextInput};

use super::session::{now_us, DesktopSession, ShellSkiaBackend};
pub(super) use super::text_edit::{paragraph_bounds_at, word_bounds_at};
#[cfg(test)]
pub(super) use super::text_edit::{
    format_keydown_shorthand, normalize_line_endings, sanitize_pasted_multiline,
    sanitize_pasted_single_line,
};
use super::{DesktopApp, DesktopHostError};

const MULTI_CLICK_MAX_INTERVAL: Duration = Duration::from_millis(400);
const MULTI_CLICK_MAX_DIST_SQ: f32 = 36.0; // 6.0 logical px

pub(super) fn is_subtree_closed_overlay(store: &NodeStore, id: NodeId) -> bool {
    let mut cur = Some(id);
    for _ in 0..256 {
        let Some(nid) = cur else {
            break;
        };
        if is_overlay_closed(store, nid) {
            return true;
        }
        cur = store.parent(nid).filter(|&p| p != nid);
    }
    false
}

pub(super) fn is_focusable(store: &NodeStore, id: NodeId) -> bool {
    if !store.is_live(id)
        || is_subtree_display_none(store, id)
        || is_subtree_closed_overlay(store, id)
    {
        return false;
    }
    if store
        .try_row::<Interaction>(id)
        .is_some_and(|i| i.disabled())
    {
        return false;
    }
    if store
        .try_row::<TextInput>(id)
        .is_some_and(|t| t.input_kind() != InputKind::Hidden)
    {
        return true;
    }
    store.try_row::<Interaction>(id).is_some_and(|i| {
        let l = i.listeners();
        i.focusable()
            || i.tab_index().is_some()
            || l.contains(InteractionEvent::Focus)
            || l.contains(InteractionEvent::Blur)
    })
}

pub(super) fn hit_test_focusable(
    store: &NodeStore,
    backend: &ShellSkiaBackend,
    x: f32,
    y: f32,
) -> Option<NodeId> {
    let mut cur = backend
        .hit_test_interaction(store, x, y)
        .or_else(|| backend.hit_test(store, x, y))?;
    for _ in 0..256 {
        if is_focusable(store, cur) {
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

impl<A: DesktopApp> DesktopSession<A> {
    pub(super) fn reset_multi_click(&mut self) {
        self.last_click_time = None;
        self.last_click_node = None;
        self.last_click_count = 0;
    }

    pub(super) fn finalize_input_turn(
        &mut self,
        events: &EventBatchBuf,
        visual_changed: bool,
    ) -> Result<bool, DesktopHostError> {
        if events.is_empty() && !visual_changed {
            return Ok(false);
        }
        let handled = if events.is_empty() {
            false
        } else {
            self.app.dispatch_events(events.as_validated())
        };
        if handled {
            self.render_and_rasterize()?;
        } else if visual_changed {
            self.rasterize_current();
        }
        Ok(handled || visual_changed)
    }

    /// Programmatically moves focus to `next_focus` (or blurs when `None`) with pointer modality.
    pub fn focus_node(&mut self, next_focus: Option<NodeId>) -> Result<bool, DesktopHostError> {
        self.focus_node_with_modality(next_focus, FocusModality::Pointer)
    }

    /// Moves focus to `next_focus` under `modality` (`Pointer` vs `Keyboard` for `:focus-visible`),
    /// emitting balanced `CompositionEnd` + `Blur`/`Focus` events and re-rendering/rasterizing.
    pub fn focus_node_with_modality(
        &mut self,
        next_focus: Option<NodeId>,
        modality: FocusModality,
    ) -> Result<bool, DesktopHostError> {
        let prev_focus = self.replica.backend().focused_node();
        let prev_modality = self.replica.backend().interaction_tracker().focus_modality();
        let had_preedit = self.replica.backend().focus_state().ime_preedit.is_some();
        let next_valid = next_focus.filter(|&id| is_focusable(self.replica.store(), id));
        {
            let (store, backend) = self.replica.store_and_backend_mut();
            backend.set_focused_node_with_modality(store, next_valid, modality);
        }
        let resolved_next = self.replica.backend().focused_node();
        let modality_changed = resolved_next.is_some() && prev_modality != modality;
        if resolved_next == prev_focus && !modality_changed {
            return Ok(false);
        }
        if resolved_next == prev_focus {
            self.rasterize_current();
            return Ok(true);
        }
        self.dragging_text_node = None;
        self.reset_multi_click();
        let ts = TimestampUs(now_us());
        let mut events = EventBatchBuf::with_capacity(3, 0);
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
        self.dispatch_and_redraw(&events)?;
        Ok(true)
    }

    pub(super) fn update_text_click_selection(
        &mut self,
        x: f32,
        y: f32,
        next_focus: Option<NodeId>,
    ) -> bool {
        let Some(text_id) = self
            .replica
            .backend()
            .focused_node()
            .filter(|&id| {
                self.replica
                    .backend()
                    .is_text_input(self.replica.store(), id)
            })
            .filter(|&id| next_focus == Some(id))
        else {
            self.dragging_text_node = None;
            self.reset_multi_click();
            return false;
        };

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
            .cursor_index_at_x(self.replica.store(), text_id, x)
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
                    .text_input_value(self.replica.store(), text_id)
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
                    .text_input_value(self.replica.store(), text_id)
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
            self.mark_a11y_dirty(text_id);
            true
        } else {
            false
        }
    }

    /// Scrolls the `ScrollView` under logical `(x, y)` by `(delta_x, delta_y)` (positive `delta_y`
    /// scrolls down), clamps to content bounds, dispatches `ScrollEvent::Scroll` into the app,
    /// and re-renders / re-rasterizes when either the app handled the event or the scroll offset
    /// changed.
    pub fn handle_scroll_logical(
        &mut self,
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let Some(scroll_node_id) = self
            .replica
            .backend()
            .hit_test_scroll_container(self.replica.store(), x, y)
        else {
            return Ok(false);
        };
        let prev = self.replica.backend().scroll_offset(scroll_node_id);
        let (store, backend) = self.replica.store_and_backend_mut();
        let (new_x, new_y) = backend.apply_scroll_delta(store, scroll_node_id, delta_x, delta_y);
        let offset_changed = (prev.0 - new_x).abs() > 1e-4 || (prev.1 - new_y).abs() > 1e-4;
        let (content_width, content_height) = backend
            .scroll_content_size(store, scroll_node_id)
            .unwrap_or((self.viewport.width, self.viewport.height));
        if offset_changed {
            self.mark_a11y_dirty(scroll_node_id);
        }

        let mut batch = EventBatchBuf::with_capacity(1, 0);
        batch.push_scroll(
            self.surface_id,
            scroll_node_id,
            TimestampUs(now_us()),
            new_x,
            new_y,
            content_width,
            content_height,
        );
        let handled = self.app.dispatch_events(batch.as_validated());
        if handled {
            self.render_and_rasterize()?;
        } else if offset_changed {
            self.rasterize_current();
        }
        Ok(handled || offset_changed)
    }
}
