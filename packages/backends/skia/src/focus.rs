//! Focus, caret, range selection, and IME composition state for [`crate::SkiaBackend`].

use nve_abi::{ComponentKind, NodeId};
use nve_engine::LayoutEngine;

use crate::text_input::{
    clamp_char_boundary, compute_text_input_inner_rect, compute_text_input_scroll_x,
    cursor_byte_index_at_local_x, is_secure_node, raw_text_input_value,
};
use crate::SkiaBackend;

/// Active focus, caret, range selection, and IME composition state tracked by [`SkiaBackend`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFocusState {
    pub focused_node: Option<NodeId>,
    /// Caret (active end) UTF-8 byte offset within the focused `TextInput`'s committed value
    /// (always clamped to a valid UTF-8 char boundary).
    pub caret_byte_index: usize,
    /// Selection anchor UTF-8 byte offset (`selection_anchor != caret_byte_index` indicates an
    /// active range selection).
    pub selection_anchor: usize,
    /// Active IME preedit (composition) string inserted visually at `caret_byte_index`.
    pub ime_preedit: Option<String>,
    /// Optional byte range `(start, end)` of cursor/selection inside `ime_preedit`.
    pub ime_preedit_cursor: Option<(usize, usize)>,
    /// Local value override for uncontrolled `TextInput`s; cleared automatically whenever
    /// `submit_frame` supplies a `value` prop or focus moves.
    pub(crate) uncontrolled_value: Option<String>,
}

impl TextFocusState {
    /// Returns the ordered `(start, end)` UTF-8 byte range when a non-empty selection is active.
    #[inline]
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        if self.selection_anchor != self.caret_byte_index {
            Some((
                self.selection_anchor.min(self.caret_byte_index),
                self.selection_anchor.max(self.caret_byte_index),
            ))
        } else {
            None
        }
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus.focused_node
    }

    pub fn focus_state(&self) -> &TextFocusState {
        &self.focus
    }

    pub fn is_text_input(&self, node_id: NodeId) -> bool {
        self.layout_tree
            .node(node_id)
            .is_some_and(|n| n.kind == ComponentKind::TextInput)
    }

    pub fn is_secure_text_input(&self, node_id: NodeId) -> bool {
        self.layout_tree
            .node(node_id)
            .is_some_and(|n| n.kind == ComponentKind::TextInput && is_secure_node(n))
    }

    /// Hit-tests a logical viewport X coordinate against a `TextInput` node and returns the
    /// closest UTF-8 byte boundary inside its current committed value.
    pub fn cursor_index_at_x(&self, node_id: NodeId, logical_x: f32) -> Option<usize> {
        let node = self.layout_tree.node(node_id)?;
        if node.kind != ComponentKind::TextInput {
            return None;
        }
        let committed = self.text_input_value(node_id)?;
        let visual_rect = self.visual_rect_of(node_id)?;
        let font_size = if node.style.font_size > 0.0 {
            node.style.font_size
        } else {
            13.0
        };
        let text_rect = compute_text_input_inner_rect(node, visual_rect, font_size);
        let is_secure = is_secure_node(node);
        let scroll_x = if self.focus.focused_node == Some(node_id) {
            compute_text_input_scroll_x(
                self.text_engine.as_ref(),
                &committed,
                self.focus.caret_byte_index,
                font_size,
                is_secure,
                text_rect.width,
            )
        } else {
            0.0
        };
        let local_x = logical_x - text_rect.x + scroll_x;
        Some(cursor_byte_index_at_local_x(
            self.text_engine.as_ref(),
            &committed,
            font_size,
            is_secure,
            local_x,
        ))
    }

    pub fn text_input_value(&self, node_id: NodeId) -> Option<String> {
        let node = self.layout_tree.node(node_id)?;
        if node.kind != ComponentKind::TextInput {
            return None;
        }
        if self.focus.focused_node == Some(node_id)
            && let Some(override_val) = &self.focus.uncontrolled_value
        {
            return Some(override_val.clone());
        }
        Some(raw_text_input_value(node).to_owned())
    }

    pub(crate) fn sync_text_focus_without_rebuild(&mut self, resolved: Option<NodeId>) -> bool {
        if self.focus.focused_node == resolved {
            return false;
        }
        self.focus.focused_node = resolved;
        self.focus.ime_preedit = None;
        self.focus.ime_preedit_cursor = None;
        self.focus.uncontrolled_value = None;
        let end = resolved
            .and_then(|id| self.text_input_value(id))
            .map_or(0, |v| v.len());
        self.focus.caret_byte_index = end;
        self.focus.selection_anchor = end;
        true
    }

    pub fn set_focused_node(&mut self, node_id: Option<NodeId>) -> bool {
        let dirty = self.layout_tree.focus_pointer(node_id);
        let resolved = self.layout_tree.interaction_tracker().focused();
        let changed = self.sync_text_focus_without_rebuild(resolved);
        if changed || !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
            true
        } else {
            false
        }
    }

    pub fn caret_byte_index(&self) -> usize {
        self.focus.caret_byte_index
    }

    pub fn selection_anchor_byte_index(&self) -> usize {
        self.focus.selection_anchor
    }

    /// Returns the ordered `(start, end)` UTF-8 byte range when a non-empty selection is active.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.focus.selection_range()
    }

    /// Returns the currently selected substring of the focused `TextInput`, if any.
    pub fn selected_text(&self) -> Option<String> {
        let id = self.focus.focused_node?;
        let (s, e) = self.focus.selection_range()?;
        let val = self.text_input_value(id)?;
        let cs = clamp_char_boundary(&val, s);
        let ce = clamp_char_boundary(&val, e);
        (cs < ce).then(|| val[cs..ce].to_owned())
    }

    pub fn caret_char_index(&self) -> usize {
        let Some(id) = self.focus.focused_node else {
            return 0;
        };
        let Some(val) = self.text_input_value(id) else {
            return 0;
        };
        let clamped = clamp_char_boundary(&val, self.focus.caret_byte_index);
        val[..clamped].chars().count()
    }

    /// Moves the caret to `byte_index` (clamped to a valid char boundary), collapsing any active
    /// selection (`selection_anchor = caret_byte_index`).
    pub fn set_caret_byte_index(&mut self, byte_index: usize) {
        self.set_selection_range(byte_index, byte_index);
    }

    /// Sets the selection anchor and active caret byte indices (both clamped to valid UTF-8 char
    /// boundaries) and rebuilds the display list when changed.
    pub fn set_selection_range(&mut self, anchor: usize, caret: usize) {
        let (clamped_anchor, clamped_caret) = self
            .focus
            .focused_node
            .and_then(|id| self.text_input_value(id))
            .map_or((0, 0), |v| {
                (
                    clamp_char_boundary(&v, anchor),
                    clamp_char_boundary(&v, caret),
                )
            });
        let changed = self.focus.selection_anchor != clamped_anchor
            || self.focus.caret_byte_index != clamped_caret;
        self.focus.selection_anchor = clamped_anchor;
        self.focus.caret_byte_index = clamped_caret;
        if changed {
            self.rebuild_display_list();
        }
    }

    /// Selects the entire committed value of the focused `TextInput` (`0..val.len()`).
    pub fn select_all_text(&mut self) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let len = self.text_input_value(id).map_or(0, |v| v.len());
        self.set_selection_range(0, len);
    }

    /// Collapses any active selection to the current caret position.
    pub fn clear_selection(&mut self) {
        let caret = self.focus.caret_byte_index;
        self.set_selection_range(caret, caret);
    }

    pub fn ime_preedit(&self) -> Option<&str> {
        self.focus.ime_preedit.as_deref()
    }

    pub fn set_ime_preedit(
        &mut self,
        preedit: Option<String>,
        cursor: Option<(usize, usize)>,
    ) {
        let normalized = preedit.filter(|s| !s.is_empty());
        let normalized_cursor = normalized.as_deref().and_then(|text| {
            cursor.map(|(s, e)| {
                let cs = clamp_char_boundary(text, s);
                let ce = clamp_char_boundary(text, e.max(s));
                (cs, ce)
            })
        });
        if self.focus.ime_preedit != normalized
            || self.focus.ime_preedit_cursor != normalized_cursor
        {
            self.focus.ime_preedit = normalized;
            self.focus.ime_preedit_cursor = normalized_cursor;
            self.rebuild_display_list();
        }
    }

    pub fn set_focused_text_override(&mut self, value: Option<String>, caret_byte_index: usize) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let clamped = match &value {
            Some(v) => clamp_char_boundary(v, caret_byte_index),
            None => self.layout_tree.node(id).map_or(0, |n| {
                clamp_char_boundary(raw_text_input_value(n), caret_byte_index)
            }),
        };
        self.focus.uncontrolled_value = value;
        self.focus.caret_byte_index = clamped;
        self.focus.selection_anchor = clamped;
        self.rebuild_display_list();
    }

    pub(crate) fn reconcile_focus_state(&mut self) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        let Some(node) = self.layout_tree.node(id) else {
            self.focus = TextFocusState::default();
            return;
        };
        if node.kind == ComponentKind::TextInput {
            if node.raw_props.contains_key("value") {
                self.focus.uncontrolled_value = None;
            }
            let current_val = self
                .focus
                .uncontrolled_value
                .as_deref()
                .unwrap_or_else(|| raw_text_input_value(node));
            self.focus.caret_byte_index =
                clamp_char_boundary(current_val, self.focus.caret_byte_index);
            self.focus.selection_anchor =
                clamp_char_boundary(current_val, self.focus.selection_anchor);
        } else {
            self.focus.caret_byte_index = 0;
            self.focus.selection_anchor = 0;
            self.focus.ime_preedit = None;
            self.focus.ime_preedit_cursor = None;
            self.focus.uncontrolled_value = None;
        }
    }
}
