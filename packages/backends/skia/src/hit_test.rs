//! Z-ordered, clip-aware, scroll-shifted, sticky-clamped, animated `Transform2D` hit testing for
//! [`crate::SkiaBackend`] delegating directly to [`nve_layout::LaidOutScene`].

use nve_abi::NodeId;
use nve_layout::{LaidOutScene, StoreLayoutEngine};
use nve_store::NodeStore;

use crate::SkiaBackend;

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.scene().hit_test_where_with_transform(
            store,
            x,
            y,
            |id, base| self.motion.resolve_animated_transform(id.raw(), base),
            |id| LaidOutScene::<L>::is_event_target(store, id),
        )
    }

    pub fn hit_test_interaction(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.scene().hit_test_where_with_transform(
            store,
            x,
            y,
            |id, base| self.motion.resolve_animated_transform(id.raw(), base),
            |id| self.layout_tree.is_interaction_target(store, id),
        )
    }

    pub fn hit_test_scroll_container(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.scene().hit_test_where_with_transform(
            store,
            x,
            y,
            |id, base| self.motion.resolve_animated_transform(id.raw(), base),
            |id| LaidOutScene::<L>::is_scroll_container(store, id),
        )
    }
}
