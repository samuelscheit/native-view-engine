//! Z-ordered, clip-aware, scroll-shifted, sticky-clamped, animated `Transform2D` hit testing for
//! [`crate::SkiaBackend`] delegating directly to [`nve_layout::LaidOutScene`].

use nve_abi::{EventMask, NodeId};
use nve_components::tables::{Interaction, Scroll};
use nve_layout::StoreLayoutEngine;
use nve_store::NodeStore;

use crate::SkiaBackend;

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    pub fn hit_test(&self, store: &NodeStore, x: f32, y: f32) -> Option<NodeId> {
        self.layout_tree.scene().hit_test_where_with_transform(
            store,
            x,
            y,
            |id, base| self.motion.resolve_animated_transform(id.raw(), base),
            |id| {
                store
                    .try_row::<Interaction>(id)
                    .is_some_and(|i| i.event_mask() != 0)
            },
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
            |id| {
                store.has::<Scroll>(id)
                    || store
                        .try_row::<Interaction>(id)
                        .is_some_and(|i| (u64::from(i.event_mask()) & EventMask::SCROLL) != 0)
            },
        )
    }
}
