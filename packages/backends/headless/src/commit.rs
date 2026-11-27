//! [`StoreBackend`] commit, diff, and lifecycle implementation for [`HeadlessBackend`].

use std::time::Instant;

use nve_abi::table::effect;
use nve_abi::{NativeEvent, NodeId, NveSceneTable, SurfaceId};
use nve_components::tables::TextContent;
use nve_engine::{BackendApplyStats, BackendError, LayoutRect, StoreBackend, Viewport};
use nve_layout::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore};

use crate::{HeadlessBackend, HeadlessMutation, MAX_TREE_DEPTH};

impl<L: StoreLayoutEngine> StoreBackend for HeadlessBackend<L> {
    fn name(&self) -> &'static str {
        "headless"
    }

    fn set_viewport(&mut self, surface_id: SurfaceId, viewport: Viewport) {
        self.surface_id = surface_id;
        if self.scene.set_viewport(viewport) {
            self.scene_table = NveSceneTable::new();
        }
    }

    fn commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
    ) -> Result<BackendApplyStats, BackendError> {
        let t0 = Instant::now();
        if self.track_mutations {
            self.last_mutations.clear();
        }

        for &id in &changes.removed {
            let idx = id.index();
            if idx < self.prev_rects.len() {
                self.prev_rects[idx] = (0, LayoutRect::default());
            }
            if self.track_mutations {
                self.last_mutations.push(HeadlessMutation::Remove(id.raw()));
            }
        }
        for &id in &changes.created {
            let idx = id.index();
            if idx < self.prev_rects.len() {
                self.prev_rects[idx] = (0, LayoutRect::default());
            }
            if self.track_mutations {
                self.last_mutations
                    .push(HeadlessMutation::Create(id.raw(), store.primary_slot(id)));
            }
        }
        if self.track_mutations {
            for (&id, &fx) in changes.touched.iter().zip(changes.effects.iter()) {
                if fx & effect::MEASURE != 0
                    && let Some(t) = store.try_row::<TextContent>(id)
                {
                    self.last_mutations.push(HeadlessMutation::UpdateText(
                        id.raw(),
                        t.text().to_owned(),
                    ));
                }
                if fx & (effect::PAINT | effect::COMPOSITE) != 0 {
                    self.last_mutations
                        .push(HeadlessMutation::UpdateStyle(id.raw()));
                }
            }
        }

        let t_layout = Instant::now();
        let laid_out = self.scene.commit(store, changes);
        let layout_us = t_layout.elapsed().as_micros() as u64;

        let t_diff = Instant::now();
        if self.track_mutations && laid_out {
            let cap = store.capacity();
            if self.prev_rects.len() < cap {
                self.prev_rects.resize(cap, (0, LayoutRect::default()));
            }
            self.rect_gen = self.rect_gen.wrapping_add(1).max(1);
            let cur_gen = self.rect_gen;
            self.walk_stack.clear();
            if store.is_live(StoreNodeId::ROOT) {
                self.walk_stack.push((StoreNodeId::ROOT, 0));
            }
            while let Some((id, depth)) = self.walk_stack.pop() {
                if depth > MAX_TREE_DEPTH || !store.is_live(id) {
                    continue;
                }
                if let Some(rect) = self.scene.layout_of(id) {
                    let idx = id.index();
                    let (prev_gen, prev_rect) = self.prev_rects[idx];
                    if prev_gen == 0 || prev_rect != rect {
                        self.last_mutations
                            .push(HeadlessMutation::UpdateLayout(id.raw(), rect));
                    }
                    self.prev_rects[idx] = (cur_gen, rect);
                }
                for &cid in store.children(id).iter().rev() {
                    self.walk_stack.push((cid, depth + 1));
                }
            }
        }
        let diff_us = t_diff.elapsed().as_micros() as u64;
        let _ = t0;

        let op_count = (changes.created.len()
            + changes.removed.len()
            + changes.touched.len()
            + changes.children.len()) as u32;
        Ok(BackendApplyStats {
            op_count,
            mutation_count: self.last_mutations.len() as u32,
            decode_us: 0,
            layout_us,
            diff_us,
            mount_us: 0,
        })
    }

    fn drain_events(&mut self) -> Vec<NativeEvent> {
        std::mem::take(&mut self.pending_events)
    }

    fn layout_of(&self, node_id: NodeId) -> Option<LayoutRect> {
        self.scene.layout_of(StoreNodeId(node_id))
    }

    fn reset(&mut self) {
        self.scene.reset();
        self.prev_rects.clear();
        self.rect_gen = 0;
        self.last_mutations.clear();
        self.pending_events.clear();
        self.scene_table.clear();
        self.published_tables.clear();
        self.published_dirty_buf.clear();
    }
}
