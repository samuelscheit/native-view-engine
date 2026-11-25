//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`) and [`CursorKind`] resolution wired into [`crate::SkiaBackend`].

use nve_abi::{NodeId, StyleProps};
use nve_components::interaction::{
    CursorKind, InteractionState, InteractionTracker, StateStyleRule,
};
use nve_engine::LayoutEngine;

use crate::SkiaBackend;

impl<L: LayoutEngine> SkiaBackend<L> {
    /// Returns a read-only reference to the underlying [`InteractionTracker`].
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        self.layout_tree.interaction_tracker()
    }

    /// Returns the active [`InteractionState`] flags for `node_id`.
    #[inline]
    pub fn interaction_state(&self, node_id: NodeId) -> InteractionState {
        self.layout_tree.interaction_state(node_id)
    }

    /// Directly attaches or clears a zero-serialization base [`CursorKind`] on `id`.
    pub fn set_node_cursor(&mut self, id: NodeId, cursor: Option<CursorKind>) {
        let _ = self.layout_tree.set_node_cursor(id, cursor);
    }

    /// Directly replaces the zero-serialization [`StateStyleRule`] list on `id`, rebuilding the
    /// retained `DisplayList` immediately when `id` currently has an active interaction state.
    pub fn set_node_state_rules(&mut self, id: NodeId, rules: Vec<StateStyleRule>) {
        if self.layout_tree.set_node_state_rules(id, rules)
            && !self.layout_tree.interaction_state(id).is_empty()
        {
            self.sync_motion_targets(&[id]);
            self.rebuild_display_list();
        }
    }

    /// Bulk-synchronizes typed [`CursorKind`] values and [`StateStyleRule`] slices from any
    /// producer (e.g. `DioxusSurface`), rebuilding the `DisplayList` when any currently active
    /// node's rules changed.
    pub fn sync_interaction_from<'a>(
        &mut self,
        cursors: impl IntoIterator<Item = (NodeId, CursorKind)>,
        state_rules: impl IntoIterator<Item = (NodeId, &'a [StateStyleRule])>,
    ) {
        let dirty = self.layout_tree.sync_interaction_from(cursors, state_rules);
        if dirty
            .iter()
            .any(|&id| !self.layout_tree.interaction_state(id).is_empty())
        {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
    }

    /// Marks `id` disabled or enabled, clearing text focus if needed and rebuilding the display
    /// list when state changed.
    pub fn set_node_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        let dirty = self.layout_tree.set_node_disabled(id, disabled);
        if disabled && self.focus.focused_node == Some(id) {
            self.focus.focused_node = None;
            self.focus.ime_preedit = None;
            self.focus.ime_preedit_cursor = None;
            self.focus.uncontrolled_value = None;
            self.focus.caret_byte_index = 0;
            self.focus.selection_anchor = 0;
        }
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Moves the pointer to logical coordinates `(x, y)` using scroll-, clip-, and z-order-aware
    /// hit-testing, rebuilding the display list iff any node's interaction state changed.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        let dirty = self.layout_tree.pointer_move_node(target);
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Clears hover and cancels any active press when the pointer exits the surface.
    pub fn pointer_leave(&mut self) -> Vec<NodeId> {
        let dirty = self.layout_tree.pointer_leave();
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Presses the pointer at logical coordinates `(x, y)`, updating `ACTIVE` state unless disabled.
    pub fn pointer_down(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        let dirty = self.layout_tree.pointer_down_node(target);
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Releases the pointer at logical coordinates `(x, y)`, clearing `ACTIVE` and updating `HOVER`.
    pub fn pointer_up(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        let dirty = self.layout_tree.pointer_up_node(target);
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Handles a touch tap at `(x, y)`, clearing hover/active to prevent sticky hover on hybrid
    /// devices.
    pub fn touch_tap(&mut self, x: f32, y: f32) -> Vec<NodeId> {
        let target = self.hit_test_interaction(x, y);
        let dirty = self.layout_tree.touch_tap_node(target);
        if !dirty.is_empty() {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Moves keyboard focus (`FOCUS | FOCUS_VISIBLE`) to `node_id` (or blurs when `None`),
    /// synchronizing `TextFocusState` and rebuilding the display list when changed.
    pub fn focus_keyboard(&mut self, node_id: Option<NodeId>) -> Vec<NodeId> {
        let dirty = self.layout_tree.focus_keyboard(node_id);
        let resolved = self.layout_tree.interaction_tracker().focused();
        let text_changed = self.sync_text_focus_without_rebuild(resolved);
        if !dirty.is_empty() || text_changed {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Moves pointer focus (`FOCUS` without `FOCUS_VISIBLE`) to `node_id` (or blurs when `None`),
    /// synchronizing `TextFocusState` and rebuilding the display list when changed.
    pub fn focus_pointer(&mut self, node_id: Option<NodeId>) -> Vec<NodeId> {
        let dirty = self.layout_tree.focus_pointer(node_id);
        let resolved = self.layout_tree.interaction_tracker().focused();
        let text_changed = self.sync_text_focus_without_rebuild(resolved);
        if !dirty.is_empty() || text_changed {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Blurs the currently focused node.
    pub fn blur(&mut self) -> Vec<NodeId> {
        let dirty = self.layout_tree.blur();
        let text_changed = self.sync_text_focus_without_rebuild(None);
        if !dirty.is_empty() || text_changed {
            self.sync_motion_targets(&dirty);
            self.rebuild_display_list();
        }
        dirty
    }

    /// Resolves the effective [`StyleProps`] for `node_id` after applying active state patches.
    #[inline]
    pub fn effective_style(&self, node_id: NodeId) -> Option<StyleProps> {
        self.layout_tree.effective_style(node_id)
    }

    /// Returns the active window [`CursorKind`] derived from the currently pressed or hovered node.
    #[inline]
    pub fn active_cursor(&self) -> CursorKind {
        self.layout_tree.active_cursor()
    }
}
