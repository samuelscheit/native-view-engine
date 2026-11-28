//! Flat hit-test fixture shared by the `hit_test` zero-allocation guard and Criterion bench:
//! `ROOT` → `n` absolute 2×2 siblings side by side (sibling `i` at x = 2·i), each with a press
//! handler, with either one shared `z_index` or ascending distinct ones (`z = i`).
//!
//! [`ZSiblings::BOTTOM`] lands on sibling 0, the last one visited topmost-first in both
//! shapes, so every query rejects all other siblings before hitting.

use nve_abi::{EventSet, InteractionEvent, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_tables::tables::{Interaction, Layout, Position, Visual, core_realization_order};
use nve_engine::{StoreBackend, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore};

pub struct ZSiblings {
    pub store: NodeStore,
    pub backend: HeadlessBackend,
    pub bottom: NodeId,
}

impl ZSiblings {
    /// Inside sibling 0 only.
    pub const BOTTOM: (f32, f32) = (1.0, 1.0);

    pub fn new(n: usize, distinct_z: bool) -> Self {
        let mut store = NodeStore::new();
        store.set_realization_order(core_realization_order());
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let siblings: Vec<NodeId> = (0..n)
            .map(|i| {
                let id = tx.create();
                tx.edit::<Layout>(id)
                    .position(Position::Absolute)
                    .inset_left(Length::points(2.0 * i as f32))
                    .inset_top(Length::points(0.0))
                    .width(Length::points(2.0))
                    .height(Length::points(2.0));
                tx.edit::<Visual>(id)
                    .z_index(if distinct_z { i as i32 } else { 0 });
                tx.edit::<Interaction>(id)
                    .listeners(EventSet::one(InteractionEvent::Press));
                id
            })
            .collect();
        tx.set_children(NodeId::ROOT, &siblings);
        tx.commit_into(&mut changes);
        let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 1280.0,
                height: 800.0,
                scale_factor: 1.0,
            },
        );
        assert!(backend.commit(&store, &changes).is_ok());
        Self {
            store,
            backend,
            bottom: siblings[0],
        }
    }

    /// `hit_test` and `hit_test_interaction` at [`Self::BOTTOM`]; both must find sibling 0.
    pub fn hit_bottom(&self) -> bool {
        let (x, y) = Self::BOTTOM;
        let expected = Some(self.bottom);
        self.backend.hit_test(&self.store, x, y) == expected
            && self.backend.hit_test_interaction(&self.store, x, y) == expected
    }
}
