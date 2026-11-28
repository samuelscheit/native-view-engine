//! `Scroll` table content bounds, offset clamping, and virtualized scroll reconciliation for
//! [`crate::SkiaBackend`], backed directly by [`nve_layout::LaidOutScene`].

use nve_abi::NodeId;
use nve_backend_headless::HeadlessBackend;
use nve_layout::StoreLayoutEngine;
use nve_store::NodeStore;

use crate::SkiaBackend;

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    #[inline]
    pub fn scroll_offset(&self, node_id: NodeId) -> (f32, f32) {
        self.layout_tree.scene().scroll_offset(node_id)
    }

    pub fn scroll_content_size(&self, store: &NodeStore, node_id: NodeId) -> Option<(f32, f32)> {
        self.layout_tree.scene().scroll_content_size(store, node_id)
    }

    /// Sets the scroll offset of `node_id`, clamping against `[0..=content - viewport]`,
    /// rebuilds the retained [`crate::DisplayList`] if changed, and returns the clamped `(x, y)`.
    pub fn set_scroll_offset(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        x: f32,
        y: f32,
    ) -> (f32, f32) {
        let (clamped_x, clamped_y, changed) = self
            .layout_tree
            .scene_mut()
            .set_scroll_offset_clamped(store, node_id, x, y);
        if changed {
            self.rebuild_display_list(store);
        }
        (clamped_x, clamped_y)
    }

    pub fn apply_scroll_delta(
        &mut self,
        store: &NodeStore,
        node_id: NodeId,
        dx: f32,
        dy: f32,
    ) -> (f32, f32) {
        let (clamped_x, clamped_y, changed) = self
            .layout_tree
            .scene_mut()
            .apply_scroll_delta(store, node_id, dx, dy);
        if changed {
            self.rebuild_display_list(store);
        }
        (clamped_x, clamped_y)
    }

    #[inline]
    pub(crate) fn visual_scroll_shift(&self, store: &NodeStore, id: NodeId) -> (f32, f32) {
        self.layout_tree.scene().visual_scroll_shift(store, id)
    }

    #[inline]
    pub(crate) fn reconcile_scroll_offsets(&mut self, store: &NodeStore) {
        self.layout_tree.scene_mut().reconcile_scroll_offsets(store);
    }
}

/// Computes `(content_width, content_height)` for a `Scroll` node.
pub(crate) fn compute_scroll_content_size<L: StoreLayoutEngine>(
    store: &NodeStore,
    layout_tree: &HeadlessBackend<L>,
    node_id: NodeId,
) -> Option<(f32, f32)> {
    layout_tree.scene().scroll_content_size(store, node_id)
}
