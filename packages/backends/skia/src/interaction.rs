//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`) and live [`CursorKind`] resolution for [`crate::SkiaBackend`] over `&NodeStore`.

use nve_abi::NodeId;
use nve_components::interaction::{CursorKind, FocusModality, InteractionState, InteractionTracker};
use nve_engine::StoreLayoutEngine;
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::SkiaBackend;

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        self.layout_tree.interaction_tracker()
    }

    #[inline]
    pub fn interaction_state(&self, node_id: NodeId) -> InteractionState {
        self.layout_tree.interaction_state(node_id)
    }

    /// Resolves the active cursor for the currently hovered/active node, or `CursorKind::Default`.
    pub fn current_cursor(&self, store: &NodeStore) -> CursorKind {
        self.layout_tree
            .interaction_tracker()
            .active_cursor_node()
            .map(|id| self.layout_tree.effective_cursor(store, StoreNodeId(id)))
            .unwrap_or(CursorKind::Default)
    }

    pub fn effective_cursor(&self, store: &NodeStore, node_id: StoreNodeId) -> CursorKind {
        self.layout_tree.effective_cursor(store, node_id)
    }

    pub fn pointer_move(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        let hit = self.hit_test_interaction(store, x, y);
        let dirty = self.layout_tree.interaction_tracker_mut_move(hit);
        if !dirty.is_empty() {
            self.sync_motion_targets(store, &dirty);
            self.rebuild_display_list(store);
            true
        } else {
            false
        }
    }

    pub fn pointer_leave(&mut self, store: &NodeStore) -> bool {
        let dirty = self.layout_tree.dispatch_pointer_leave();
        if !dirty.is_empty() {
            self.sync_motion_targets(store, &dirty);
            self.rebuild_display_list(store);
            true
        } else {
            false
        }
    }

    pub fn pointer_down(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        let hit = self.hit_test_interaction(store, x, y);
        let dirty = self.layout_tree.interaction_tracker_mut_down(hit);
        if !dirty.is_empty() {
            self.sync_motion_targets(store, &dirty);
            self.rebuild_display_list(store);
            true
        } else {
            false
        }
    }

    pub fn pointer_up(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        let hit = self.hit_test_interaction(store, x, y);
        let dirty = self.layout_tree.interaction_tracker_mut_up(hit);
        if !dirty.is_empty() {
            self.sync_motion_targets(store, &dirty);
            self.rebuild_display_list(store);
            true
        } else {
            false
        }
    }

    pub fn set_interaction_focus(
        &mut self,
        store: &NodeStore,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> bool {
        let dirty = self.layout_tree.dispatch_focus(node_id, modality);
        if !dirty.is_empty() {
            self.sync_motion_targets(store, &dirty);
            self.rebuild_display_list(store);
            true
        } else {
            false
        }
    }
}
