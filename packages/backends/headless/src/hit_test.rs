//! Hit-test geometry forwarding to [`nve_layout::LaidOutScene`].

use nve_abi::NodeId;
use nve_components::transform::Transform2D;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeId as StoreNodeId, NodeStore};

use crate::HeadlessBackend;

impl<L: StoreLayoutEngine> HeadlessBackend<L> {
    #[inline]
    pub fn compute_sticky_delta(&self, store: &NodeStore, id: StoreNodeId) -> (f32, f32) {
        self.scene.compute_sticky_delta(store, id)
    }

    #[inline]
    pub fn effective_transform(&self, store: &NodeStore, id: StoreNodeId) -> Option<Transform2D> {
        self.scene.effective_transform(store, id)
    }

    #[inline]
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.scene.hit_test(store, x, y)
    }
}

#[cfg(test)]
#[path = "hit_test_tests.rs"]
mod tests;
