//! `SkiaBackend` integration helpers bridging `NodeStore`/`HeadlessBackend`/`SkiaFrameHooks` into
//! [`super::CompositorScene`].

use std::collections::HashSet;

use nve_abi::{EventBatchBuf, NodeId};
use nve_layout::StoreLayoutEngine;
use nve_store::{ChangeSet, NodeStore, TableStorage};

use super::emit::SceneSyncEnv;
use super::{CompositorScene, SceneDirtyKind};
use crate::slab_lru::FastBuildHasher;
use crate::table_system::SkiaTableRegistry;
use crate::{DisplayList, SkiaBackend};

macro_rules! sync_env {
    ($self:expr, $store:expr) => {
        SceneSyncEnv {
            store: $store,
            layout_tree: &$self.layout_tree,
            focus: &$self.focus,
            frame_hooks: &$self.frame_hooks,
            table_registry: &$self.table_registry,
            plugin_epochs: &$self.plugin_epochs,
            base_plugin_epoch: $self.base_plugin_epoch,
            text_engine: $self.text_engine.as_ref(),
        }
    };
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    /// Returns the retained local-space [`CompositorScene`].
    #[inline]
    pub fn scene(&self) -> &CompositorScene {
        &self.scene
    }

    /// Disjoint borrows of `(scene, table_registry, table_events)` for GPU/Skia/software rendering.
    #[inline]
    pub fn scene_paint_parts_mut(
        &mut self,
    ) -> (&CompositorScene, &SkiaTableRegistry, &mut EventBatchBuf) {
        (&self.scene, &self.table_registry, &mut self.table_events)
    }

    /// Materializes the retained [`CompositorScene`] into [`DisplayList`] on demand (for
    /// diagnostics, prerender inspection, and tests).
    pub fn materialize_display_list(&mut self) -> &DisplayList {
        self.scene
            .materialize_display_list(self.viewport.scale_factor, &mut self.display_list);
        &self.display_list
    }

    pub(crate) fn sync_scene_for_commit(
        &mut self,
        store: &NodeStore,
        changes: &ChangeSet,
        viewport_dirty: bool,
        plugin_dirty_nodes: &[NodeId],
        plugin_base_epoch_changed: bool,
    ) -> SceneDirtyKind {
        let env = sync_env!(self, store);
        self.scene.sync_commit(
            &env,
            changes,
            self.viewport,
            viewport_dirty,
            plugin_dirty_nodes,
            plugin_base_epoch_changed,
        )
    }

    pub(crate) fn update_scene_scroll(
        &mut self,
        store: &NodeStore,
        scroll_id: NodeId,
        clamped_x: f32,
        clamped_y: f32,
    ) {
        if !self.scene.is_initialized() {
            return;
        }
        let env = sync_env!(self, store);
        let _ = self.scene.update_scroll(&env, scroll_id, clamped_x, clamped_y);
    }

    pub(crate) fn update_scene_composite_nodes(&mut self, store: &NodeStore, nodes: &[NodeId]) {
        if !self.scene.is_initialized() {
            return;
        }
        let env = sync_env!(self, store);
        self.scene.update_composite_nodes(&env, nodes);
    }

    pub(crate) fn update_scene_paint_nodes(&mut self, store: &NodeStore, nodes: &[NodeId]) {
        if !self.scene.is_initialized() {
            return;
        }
        let env = sync_env!(self, store);
        self.scene.update_dirty_paint_nodes(&env, nodes);
    }

    pub(crate) fn sync_full_scene(&mut self, store: &NodeStore) {
        let empty = ChangeSet::default();
        self.sync_scene_for_commit(store, &empty, true, &[], true);
    }

    pub(crate) fn bump_plugin_nodes_referencing_changed_resources(
        &mut self,
        store: &NodeStore,
        slot: nve_store::TableSlot,
        changes: &ChangeSet,
        changed_set: Option<&HashSet<nve_store::ResourceId, FastBuildHasher>>,
        dirty_out: &mut Vec<NodeId>,
    ) {
        let Some(TableStorage {
            rows,
            owners,
            row_count,
            ..
        }) = store.table_storage(slot)
        else {
            return;
        };
        if (changes.resources_defined.is_empty() && changes.resources_released.is_empty())
            || row_count == 0
        {
            return;
        }
        let is_changed = |rid: nve_store::ResourceId| -> bool {
            if let Some(set) = changed_set {
                set.contains(&rid)
            } else {
                changes.resources_defined.contains(&rid)
                    || changes.resources_released.contains(&rid)
            }
        };

        let Some(desc) = store.table_desc(slot) else {
            let words_per_row = rows.len() / row_count;
            if words_per_row == 0 {
                return;
            }
            for r in 0..row_count {
                let owner = owners.get(r).copied().unwrap_or(NodeId::NULL);
                if owner.is_null() {
                    continue;
                }
                let row_base = r * words_per_row;
                let row_slice = &rows[row_base..row_base + words_per_row];
                let references_changed = row_slice.iter().any(|&word| {
                    let rid = nve_store::ResourceId(word);
                    !rid.is_none() && is_changed(rid)
                });
                if references_changed {
                    self.bump_plugin_epoch(owner);
                    dirty_out.push(owner);
                }
            }
            return;
        };
        let res_mask = desc.resource_words;
        let words_per_row = desc.words();
        if res_mask == 0 || words_per_row == 0 {
            return;
        }
        for r in 0..row_count {
            let owner = owners.get(r).copied().unwrap_or(NodeId::NULL);
            if owner.is_null() {
                continue;
            }
            let row_base = r * words_per_row;
            let mut m = res_mask;
            while m != 0 {
                let w = m.trailing_zeros() as usize;
                m &= m - 1;
                if let Some(&word) = rows.get(row_base + w) {
                    let rid = nve_store::ResourceId(word);
                    if !rid.is_none() && is_changed(rid) {
                        self.bump_plugin_epoch(owner);
                        dirty_out.push(owner);
                        break;
                    }
                }
            }
        }
    }
}
