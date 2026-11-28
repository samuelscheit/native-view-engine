//! Focus, grapheme-safe caret, BiDi visual navigation, multi-run range selection, and IME
//! composition/cursor-rect state for [`crate::SkiaBackend`] over `&NodeStore`.

use nve_abi::NodeId;
use nve_engine::LayoutRect;
use nve_layout::StoreLayoutEngine;
use nve_store::NodeStore;
use nve_tables::tables::{is_subtree_display_none, InputKind, TextInput, Typography};

use crate::text::DEFAULT_FONT_WEIGHT;
use crate::text_grapheme::clamp_grapheme_boundary;
use crate::text_input::{
    compute_ime_cursor_rect, compute_text_input_inner_rect, compute_text_input_scroll_x,
    cursor_byte_index_at_local_x, is_multiline_node, is_secure_node, raw_text_input_value,
    selection_rects_for_range as compute_selection_rects,
};
use crate::SkiaBackend;

/// Active focus, caret, range selection, and IME composition state tracked by [`SkiaBackend`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextFocusState {
    pub focused_node: Option<NodeId>,
    pub caret_byte_index: usize,
    pub selection_anchor: usize,
    pub ime_preedit: Option<String>,
    pub ime_preedit_cursor: Option<(usize, usize)>,
    pub(crate) uncontrolled_value: Option<String>,
}

impl TextFocusState {
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

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    #[inline]
    pub fn focused_node(&self) -> Option<NodeId> {
        self.focus.focused_node
    }

    #[inline]
    pub fn focus_state(&self) -> &TextFocusState {
        &self.focus
    }

    #[inline]
    pub fn is_text_input(&self, store: &NodeStore, node_id: NodeId) -> bool {
        store.is_live(node_id)
            && store
                .try_row::<TextInput>(node_id)
                .is_some_and(|t| t.input_kind() != InputKind::Hidden)
    }

    #[inline]
    pub fn is_secure_text_input(&self, store: &NodeStore, node_id: NodeId) -> bool {
        store.is_live(node_id) && is_secure_node(store, node_id)
    }

    #[inline]
    pub fn is_multiline_text_input(&self, store: &NodeStore, node_id: NodeId) -> bool {
        store.is_live(node_id) && is_multiline_node(store, node_id)
    }

    pub fn text_input_value(&self, store: &NodeStore, node_id: NodeId) -> Option<String> {
        if !self.is_text_input(store, node_id) {
            return None;
        }
        if self.focus.focused_node == Some(node_id)
            && let Some(ref v) = self.focus.uncontrolled_value
        {
            return Some(v.clone());
        }
        Some(raw_text_input_value(store, node_id).to_owned())
    }

    pub fn cursor_index_at_x(
        &self,
        store: &NodeStore,
        node_id: NodeId,
        logical_x: f32,
    ) -> Option<usize> {
        if !self.is_text_input(store, node_id) {
            return None;
        }
        let committed = self.text_input_value(store, node_id)?;
        let visual_rect = self.visual_rect_of(store, node_id)?;
        let font_size = store
            .try_row::<Typography>(node_id)
            .map(|t| t.font_size())
            .filter(|&s| s > 0.0)
            .unwrap_or(13.0);
        let text_rect = compute_text_input_inner_rect(store, node_id, visual_rect, font_size);
        let is_secure = is_secure_node(store, node_id);
        let scroll_x = if self.focus.focused_node == Some(node_id) {
            compute_text_input_scroll_x(
                self.text_engine.as_ref(),
                &committed,
                self.focus.caret_byte_index,
                font_size,
                DEFAULT_FONT_WEIGHT,
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
            DEFAULT_FONT_WEIGHT,
            is_secure,
            local_x,
        ))
    }

    pub fn selection_rects_for_range(
        &self,
        store: &NodeStore,
        node_id: NodeId,
        start: usize,
        end: usize,
    ) -> Vec<LayoutRect> {
        let Some(committed) = self.text_input_value(store, node_id) else {
            return Vec::new();
        };
        let Some(visual_rect) = self.visual_rect_of(store, node_id) else {
            return Vec::new();
        };
        let font_size = store
            .try_row::<Typography>(node_id)
            .map(|t| t.font_size())
            .filter(|&s| s > 0.0)
            .unwrap_or(13.0);
        let text_rect = compute_text_input_inner_rect(store, node_id, visual_rect, font_size);
        let is_secure = is_secure_node(store, node_id);
        let scroll_x = compute_text_input_scroll_x(
            self.text_engine.as_ref(),
            &committed,
            self.focus.caret_byte_index,
            font_size,
            DEFAULT_FONT_WEIGHT,
            is_secure,
            text_rect.width,
        );
        compute_selection_rects(
            self.text_engine.as_ref(),
            &committed,
            start,
            end,
            font_size,
            DEFAULT_FONT_WEIGHT,
            is_secure,
            text_rect,
            scroll_x,
        )
    }

    pub fn ime_cursor_rect(&self, store: &NodeStore) -> Option<LayoutRect> {
        let id = self.focus.focused_node?;
        let committed = self.text_input_value(store, id)?;
        let visual_rect = self.visual_rect_of(store, id)?;
        let font_size = store
            .try_row::<Typography>(id)
            .map(|t| t.font_size())
            .filter(|&s| s > 0.0)
            .unwrap_or(13.0);
        let text_rect = compute_text_input_inner_rect(store, id, visual_rect, font_size);
        Some(compute_ime_cursor_rect(
            self.text_engine.as_ref(),
            &committed,
            self.focus.caret_byte_index,
            self.focus.ime_preedit.as_deref(),
            font_size,
            DEFAULT_FONT_WEIGHT,
            is_secure_node(store, id),
            text_rect,
            self.viewport.scale_factor,
        ))
    }

    pub fn set_focused_node(&mut self, store: &NodeStore, node_id: Option<NodeId>) {
        let node_id = node_id
            .filter(|&id| store.is_live(id) && !is_subtree_display_none(store, id));
        let prev = self.focus.focused_node;
        if prev == node_id {
            return;
        }
        self.focus = TextFocusState {
            focused_node: node_id,
            caret_byte_index: node_id
                .and_then(|id| self.text_input_value(store, id))
                .map_or(0, |s| s.len()),
            selection_anchor: node_id
                .and_then(|id| self.text_input_value(store, id))
                .map_or(0, |s| s.len()),
            ime_preedit: None,
            ime_preedit_cursor: None,
            uncontrolled_value: None,
        };
        let mut dirty = [NodeId::ROOT; 2];
        let mut n = 0usize;
        if let Some(p) = prev {
            dirty[n] = p;
            n += 1;
        }
        if let Some(cur) = node_id {
            dirty[n] = cur;
            n += 1;
        }
        self.update_scene_paint_nodes(store, &dirty[..n]);
    }

    pub fn set_caret_and_selection(
        &mut self,
        store: &NodeStore,
        caret: usize,
        anchor: usize,
    ) {
        if let Some(id) = self.focus.focused_node
            && let Some(val) = self.text_input_value(store, id)
        {
            self.focus.caret_byte_index = clamp_grapheme_boundary(&val, caret);
            self.focus.selection_anchor = clamp_grapheme_boundary(&val, anchor);
            self.update_scene_paint_nodes(store, &[id]);
        }
    }

    pub fn set_ime_preedit(
        &mut self,
        store: &NodeStore,
        preedit: Option<String>,
        cursor: Option<(usize, usize)>,
    ) {
        self.focus.ime_preedit = preedit.filter(|s| !s.is_empty());
        self.focus.ime_preedit_cursor = cursor;
        if let Some(id) = self.focus.focused_node {
            self.update_scene_paint_nodes(store, &[id]);
        }
    }

    pub fn set_uncontrolled_text_input(
        &mut self,
        store: &NodeStore,
        value: Option<String>,
        caret_byte_index: usize,
    ) {
        let clamped = match value.as_deref() {
            Some(v) => clamp_grapheme_boundary(v, caret_byte_index),
            None => self.focus.focused_node.map_or(0, |id| {
                clamp_grapheme_boundary(raw_text_input_value(store, id), caret_byte_index)
            }),
        };
        self.focus.uncontrolled_value = value;
        self.focus.caret_byte_index = clamped;
        self.focus.selection_anchor = clamped;
        if let Some(id) = self.focus.focused_node {
            self.update_scene_paint_nodes(store, &[id]);
        }
    }

    pub(crate) fn reconcile_focus_state(&mut self, store: &NodeStore) {
        let Some(id) = self.focus.focused_node else {
            return;
        };
        if !store.is_live(id) || is_subtree_display_none(store, id) {
            self.focus = TextFocusState::default();
            return;
        }
        if self.is_text_input(store, id) {
            if !raw_text_input_value(store, id).is_empty() {
                self.focus.uncontrolled_value = None;
            }
            let current_val = self
                .focus
                .uncontrolled_value
                .as_deref()
                .unwrap_or_else(|| raw_text_input_value(store, id));
            self.focus.caret_byte_index =
                clamp_grapheme_boundary(current_val, self.focus.caret_byte_index);
            self.focus.selection_anchor =
                clamp_grapheme_boundary(current_val, self.focus.selection_anchor);
        } else {
            self.focus.caret_byte_index = 0;
            self.focus.selection_anchor = 0;
            self.focus.ime_preedit = None;
            self.focus.ime_preedit_cursor = None;
            self.focus.uncontrolled_value = None;
        }
    }
}
