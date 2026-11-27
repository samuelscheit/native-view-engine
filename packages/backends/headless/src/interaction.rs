//! Interactive pseudo-state tracking and cursor resolution forwarding to [`nve_layout::LaidOutScene`].

use nve_abi::NodeId;
use nve_components::interaction::{CursorKind, FocusModality, InteractionState, InteractionTracker};
use nve_components::style_patch::{LayoutSnapshot, StylePatch, VisualSnapshot};
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::HeadlessBackend;

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    #[inline]
    pub fn interaction_tracker(&self) -> &InteractionTracker {
        self.scene.interaction_tracker()
    }

    #[inline]
    pub fn interaction_state(&self, node_id: NodeId) -> InteractionState {
        self.scene.interaction_state(node_id)
    }

    #[inline]
    pub fn set_node_disabled(&mut self, id: NodeId, disabled: bool) -> Vec<NodeId> {
        self.scene.set_node_disabled(id, disabled)
    }

    #[inline]
    pub fn is_interaction_target(&self, store: &NodeStore, id: StoreNodeId) -> bool {
        self.scene.is_interaction_target(store, id)
    }

    #[inline]
    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.scene.hit_test_interaction(store, x, y)
    }

    #[inline]
    pub fn effective_patch(&self, store: &NodeStore, id: StoreNodeId) -> StylePatch {
        self.scene.effective_patch(store, id)
    }

    #[inline]
    pub fn effective_visual(&self, store: &NodeStore, id: StoreNodeId) -> VisualSnapshot {
        self.scene.effective_visual(store, id)
    }

    #[inline]
    pub fn effective_layout_and_visual(
        &self,
        store: &NodeStore,
        id: StoreNodeId,
    ) -> (LayoutSnapshot, VisualSnapshot) {
        self.scene.effective_layout_and_visual(store, id)
    }

    #[inline]
    pub fn effective_cursor(&self, store: &NodeStore, id: StoreNodeId) -> CursorKind {
        self.scene.effective_cursor(store, id)
    }

    #[inline]
    pub fn interaction_tracker_mut_move(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.scene.interaction_move(target)
    }

    #[inline]
    pub fn interaction_tracker_mut_down(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.scene.interaction_down(target)
    }

    #[inline]
    pub fn interaction_tracker_mut_up(&mut self, target: Option<NodeId>) -> Vec<NodeId> {
        self.scene.interaction_up(target)
    }

    #[inline]
    pub fn dispatch_pointer_move(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        self.scene.dispatch_pointer_move(store, x, y)
    }

    #[inline]
    pub fn dispatch_pointer_leave(&mut self) -> Vec<NodeId> {
        self.scene.dispatch_pointer_leave()
    }

    #[inline]
    pub fn dispatch_pointer_down(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        self.scene.dispatch_pointer_down(store, x, y)
    }

    #[inline]
    pub fn dispatch_pointer_up(&mut self, store: &NodeStore, x: f32, y: f32) -> Vec<NodeId> {
        self.scene.dispatch_pointer_up(store, x, y)
    }

    #[inline]
    pub fn dispatch_pointer_cancel(&mut self) -> Vec<NodeId> {
        self.scene.dispatch_pointer_cancel()
    }

    #[inline]
    pub fn dispatch_focus(
        &mut self,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> Vec<NodeId> {
        self.scene.dispatch_focus(node_id, modality)
    }
}
