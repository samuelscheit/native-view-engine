//! Interactive pseudo-state tracking (`:hover`, `:focus`, `:focus-visible`, `:active`,
//! `:disabled`), pointer capture, and live [`CursorKind`] resolution for [`crate::SkiaBackend`]
//! over `&NodeStore`.

use nve_abi::NodeId;
use nve_engine::InteractionTracker;
use nve_layout::StoreLayoutEngine;
use nve_store::NodeStore;
use nve_tables::interaction::{CursorKind, FocusModality, InteractionState};

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

    /// Resolves the active cursor (captured ancestor chain -> hovered chain -> active chain ->
    /// `CursorKind::Default`).
    pub fn current_cursor(&self, store: &NodeStore) -> CursorKind {
        self.layout_tree.current_cursor(store)
    }

    pub fn effective_cursor(&self, store: &NodeStore, node_id: NodeId) -> CursorKind {
        self.layout_tree.effective_cursor(store, node_id)
    }

    #[inline]
    pub fn captured_node(&self) -> Option<NodeId> {
        self.layout_tree.interaction_tracker().captured_node()
    }

    #[inline]
    pub fn set_pointer_capture(&mut self, pointer_id: u32, node: NodeId) {
        self.layout_tree
            .interaction_tracker_mut()
            .set_capture(pointer_id, node);
    }

    #[inline]
    pub fn release_pointer_capture(&mut self, pointer_id: u32) -> Option<NodeId> {
        self.layout_tree
            .interaction_tracker_mut()
            .release_capture(pointer_id)
    }

    fn apply_last_interaction_dirty(&mut self, store: &NodeStore) -> bool {
        let dirty = self.layout_tree.interaction_tracker().last_dirty();
        if dirty.is_empty() {
            return false;
        }
        if dirty.len() <= 32 {
            let mut buf = [NodeId::ROOT; 32];
            let n = dirty.len();
            buf[..n].copy_from_slice(dirty);
            self.sync_frame_hooks(store, &buf[..n]);
            self.update_scene_paint_nodes(store, &buf[..n]);
        } else {
            let owned = dirty.to_vec();
            self.sync_frame_hooks(store, &owned);
            self.update_scene_paint_nodes(store, &owned);
        }
        true
    }

    pub fn pointer_move(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        let hit = self.hit_test_interaction(store, x, y);
        let _ = self.layout_tree.scene_mut().pointer_move_in_store(store, hit);
        self.apply_last_interaction_dirty(store)
    }

    pub fn pointer_leave(&mut self, store: &NodeStore) -> bool {
        let _ = self.layout_tree.scene_mut().pointer_leave_slice();
        self.apply_last_interaction_dirty(store)
    }

    pub fn pointer_cancel(&mut self, store: &NodeStore) -> bool {
        let _ = self.layout_tree.scene_mut().pointer_cancel_slice();
        self.apply_last_interaction_dirty(store)
    }

    pub fn pointer_down(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        let hit = self.hit_test_interaction(store, x, y);
        let _ = self.layout_tree.scene_mut().pointer_down_in_store(store, hit);
        self.apply_last_interaction_dirty(store)
    }

    /// Releases any active press and pointer capture, returning `(clicked_node, visual_changed)`
    /// where `clicked_node` is the deepest common non-disabled ancestor of press and release.
    pub fn pointer_up_clicked(
        &mut self,
        store: &NodeStore,
        x: f32,
        y: f32,
    ) -> (Option<NodeId>, bool) {
        let hit = self.hit_test_interaction(store, x, y);
        let clicked = self.layout_tree.scene_mut().pointer_up_in_store(store, hit).0;
        let dirty = self.apply_last_interaction_dirty(store);
        (clicked, dirty)
    }

    pub fn pointer_up(&mut self, store: &NodeStore, x: f32, y: f32) -> bool {
        self.pointer_up_clicked(store, x, y).1
    }

    pub fn set_interaction_focus(
        &mut self,
        store: &NodeStore,
        node_id: Option<NodeId>,
        modality: FocusModality,
    ) -> bool {
        let _ = self
            .layout_tree
            .scene_mut()
            .set_focus_slice(node_id, modality);
        self.apply_last_interaction_dirty(store)
    }
}
