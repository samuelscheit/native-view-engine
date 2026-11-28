//! Interactive pseudo-state tracking and cursor resolution forwarding to [`nve_layout::LaidOutScene`].

use nve_engine::InteractionTracker;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeId, NodeStore, RowPatchView};
use nve_tables::interaction::{CursorKind, FocusModality, InteractionState};
use nve_tables::snapshots::{LayoutSnapshot, VisualSnapshot};

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
    pub fn is_interaction_target(&self, store: &NodeStore, id: NodeId) -> bool {
        self.scene.is_interaction_target(store, id)
    }

    #[inline]
    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.scene.hit_test_interaction(store, x, y)
    }

    #[inline]
    pub fn visit_effective_state_patches<'a>(
        &self,
        store: &'a NodeStore,
        id: NodeId,
        f: impl FnMut(RowPatchView<'a>),
    ) {
        self.scene.visit_effective_state_patches(store, id, f);
    }

    #[inline]
    pub fn effective_visual(&self, store: &NodeStore, id: NodeId) -> VisualSnapshot {
        self.scene.effective_visual(store, id)
    }

    #[inline]
    pub fn effective_layout_and_visual(
        &self,
        store: &NodeStore,
        id: NodeId,
    ) -> (LayoutSnapshot, VisualSnapshot) {
        self.scene.effective_layout_and_visual(store, id)
    }

    #[inline]
    pub fn effective_cursor(&self, store: &NodeStore, id: NodeId) -> CursorKind {
        self.scene.effective_cursor(store, id)
    }

    #[inline]
    pub fn current_cursor(&self, store: &NodeStore) -> CursorKind {
        self.scene.current_cursor(store)
    }

    #[inline]
    pub fn interaction_tracker_mut(&mut self) -> &mut InteractionTracker {
        self.scene.interaction_tracker_mut()
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
