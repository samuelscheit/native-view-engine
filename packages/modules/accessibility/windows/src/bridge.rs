//! Incremental `NveSceneView` -> replica `NodeStore` mirror feeding `AccessKitTreeBridge`.
//!
//! Keeps `nve-accessibility` as the single source of truth for ARIA roles, labels, states,
//! `AriaRelations`, `TextInput` `TextRun`s, scroll bounds, transforms, and epoch-checked actions.

use accesskit::{ActionRequest, NodeId as AccessKitNodeId, TreeUpdate};
use nve_abi::{
    NveLayoutRect, NveNodeRow, NveSceneView, NVE_NODE_ROW_FLAG_LIVE, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_accessibility::{AccessKitTreeBridge, ResolvedA11yAction, StoreEpoch};
use nve_store::{ChangeSet, NodeId, NodeIndex, NodeStore};
use nve_tables::tables::core_realization_order;

use crate::geo::SceneGeo;
use crate::table_sync::{
    raw_slice, sync_all_tables, sync_aria_relations, sync_children, BoundA11yTables,
};

/// Borrows raw FlatResource bytes from the active `NveSceneView` owner while a sync call runs.
pub trait ResourceReader {
    fn read(&self, resource_id: u32, expected_kind: u64) -> Option<&[u8]>;
}

/// Empty [`ResourceReader`] for scenes or tests without external resources.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopResources;

impl ResourceReader for NoopResources {
    #[inline]
    fn read(&self, _resource_id: u32, _expected_kind: u64) -> Option<&[u8]> {
        None
    }
}

/// Platform-independent incremental mirror from flat C-ABI [`NveSceneView`] snapshots to
/// [`AccessKitTreeBridge`].
///
/// Preconditions on [`NveSceneView`] (guaranteed by `nve-runtime`):
/// - `scroll_offsets` is sorted ascending by `node_id`.
/// - `dirty_resources` is sorted ascending by resource id.
pub struct WinA11yBridge {
    store: NodeStore,
    changes: ChangeSet,
    bridge: AccessKitTreeBridge,
    geo: SceneGeo,
    window_title: String,
    last_scene_epoch: u32,
    last_frame_id: u64,
    has_synced_scene: bool,
    patch_words: Vec<u32>,
    child_scratch: Vec<NodeId>,
    extra_dirty: Vec<NodeId>,
    res_words: Vec<u32>,
    /// Live nodes currently referencing a non-zero `Accessibility::RELATIONS` scene resource id.
    aria_rel_owners: Vec<(NodeId, u32)>,
    /// Reusable DFS buffers for propagating updated absolute rects across moved/shifted subtrees.
    shifted_indices: Vec<u32>,
    walk_stack: Vec<NodeId>,
    visit_marks: Vec<u32>,
    visit_gen: u32,
}

impl Default for WinA11yBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl WinA11yBridge {
    /// Creates an inactive mirror (zero heap allocations on `sync_scene` until activated).
    pub fn new() -> Self {
        let mut store = NodeStore::replica();
        store.set_realization_order(core_realization_order());
        Self {
            store,
            changes: ChangeSet::default(),
            bridge: AccessKitTreeBridge::new(),
            geo: SceneGeo::default(),
            window_title: String::new(),
            last_scene_epoch: 0,
            last_frame_id: 0,
            has_synced_scene: false,
            patch_words: Vec::new(),
            child_scratch: Vec::new(),
            extra_dirty: Vec::new(),
            res_words: Vec::new(),
            aria_rel_owners: Vec::new(),
            shifted_indices: Vec::new(),
            walk_stack: Vec::new(),
            visit_marks: Vec::new(),
            visit_gen: 0,
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.bridge.is_active()
    }

    #[inline]
    pub fn has_synced_scene(&self) -> bool {
        self.has_synced_scene
    }

    #[inline]
    pub fn epoch(&self) -> StoreEpoch {
        self.bridge.epoch()
    }

    #[inline]
    pub fn activate(&mut self) {
        self.bridge.activate();
    }

    #[inline]
    pub fn decode_store_node_id(&self, id: AccessKitNodeId) -> Option<NodeId> {
        self.bridge.decode_store_node_id(id)
    }

    #[inline]
    pub fn encode_store_node_id(&self, id: NodeId) -> AccessKitNodeId {
        self.bridge.encode_store_node_id(id)
    }

    #[inline]
    pub fn translate_action(&self, req: &ActionRequest) -> Option<ResolvedA11yAction> {
        self.bridge.translate_action(req)
    }

    /// Builds a complete initial [`TreeUpdate`] from the currently mirrored scene.
    pub fn build_full_tree_update(&mut self) -> TreeUpdate {
        self.bridge
            .build_full_tree_update(&self.store, &self.geo, &self.window_title)
    }

    /// Mirrors `scene` into the internal replica `NodeStore` and returns any incremental (or
    /// initial) [`TreeUpdate`].
    ///
    /// Performs **zero heap allocations** when `!self.is_active()` or when `scene_epoch`,
    /// `frame_id`, `focused_node_id`, `scale_factor`, and `window_title` are unchanged.
    ///
    /// # Safety
    ///
    /// All non-null pointers and lengths in `scene` must satisfy the `NveSceneView` C ABI
    /// contract for the duration of this call.
    pub unsafe fn sync_scene(
        &mut self,
        scene: &NveSceneView,
        resources: &dyn ResourceReader,
        focused_node_id: u32,
        scale_factor: f32,
        window_title: &str,
    ) -> Option<TreeUpdate> {
        if !self.bridge.is_active() {
            return None;
        }
        // SAFETY: delegated to caller's `NveSceneView` contract.
        let mirrored = unsafe {
            self.mirror_scene_inner(
                scene,
                resources,
                focused_node_id,
                scale_factor,
                window_title,
            )
        };
        match mirrored {
            MirrorResult::Unchanged => None,
            MirrorResult::MetadataOnly => {
                self.bridge
                    .sync_dirty_nodes(&self.store, &self.geo, &self.window_title, &[])
            }
            MirrorResult::Full => Some(self.bridge.build_full_tree_update(
                &self.store,
                &self.geo,
                &self.window_title,
            )),
            MirrorResult::Incremental => self.bridge.sync_changes_with_dirty(
                &self.store,
                &self.changes,
                &self.geo,
                &self.window_title,
                &self.extra_dirty,
                false,
            ),
        }
    }

    /// Mirrors `scene` into the replica `NodeStore` without yet projecting a [`TreeUpdate`] (used
    /// during cold `WM_GETOBJECT` activation right before `request_initial_tree` runs).
    ///
    /// # Safety
    ///
    /// Same `NveSceneView` safety contract as [`Self::sync_scene`].
    pub unsafe fn mirror_only(
        &mut self,
        scene: &NveSceneView,
        resources: &dyn ResourceReader,
        focused_node_id: u32,
        scale_factor: f32,
        window_title: &str,
    ) -> bool {
        if !self.bridge.is_active() {
            return false;
        }
        // SAFETY: delegated to caller's `NveSceneView` contract.
        !matches!(
            unsafe {
                self.mirror_scene_inner(
                    scene,
                    resources,
                    focused_node_id,
                    scale_factor,
                    window_title,
                )
            },
            MirrorResult::Unchanged
        )
    }

    unsafe fn mirror_scene_inner(
        &mut self,
        scene: &NveSceneView,
        resources: &dyn ResourceReader,
        focused_node_id: u32,
        scale_factor: f32,
        window_title: &str,
    ) -> MirrorResult {
        if scene.schema_hash != NVE_RUNTIME_SCHEMA_HASH {
            return MirrorResult::Unchanged;
        }
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let focused = {
            let raw = NodeId::from_raw_or_null(focused_node_id);
            (!raw.is_null()).then_some(raw)
        };
        let meta_changed = focused != self.geo.focused_node
            || (scale - self.geo.scale_factor).abs() > 1e-6
            || window_title != self.window_title.as_str();

        if self.has_synced_scene
            && scene.scene_epoch == self.last_scene_epoch
            && scene.frame_id == self.last_frame_id
        {
            if !meta_changed {
                return MirrorResult::Unchanged;
            }
            self.update_meta(focused, scale, window_title);
            return MirrorResult::MetadataOnly;
        }

        // SAFETY: caller guarantees `scene` slices are valid for this call.
        let (rows, rects, text_slab, children_slab, tables_slice, scrolls, dirty, removed, moved, dirty_res) =
            unsafe {
                (
                    raw_slice(scene.rows, scene.node_capacity),
                    raw_slice(scene.rects, scene.node_capacity),
                    raw_slice(scene.text_slab, scene.text_slab_len),
                    raw_slice(scene.children_slab, scene.children_slab_len),
                    raw_slice(scene.tables, scene.table_count),
                    raw_slice(scene.scroll_offsets, scene.scroll_count),
                    raw_slice(scene.dirty_indices, scene.dirty_count),
                    raw_slice(scene.removed_ids, scene.removed_count),
                    raw_slice(scene.moved_indices, scene.moved_count),
                    raw_slice(scene.dirty_resources, scene.dirty_resource_count),
                )
            };
        // SAFETY: `tables_slice` belongs to the validated `scene`.
        let bound = unsafe { BoundA11yTables::bind(tables_slice) };
        let epoch_changed = self.has_synced_scene && scene.scene_epoch != self.last_scene_epoch;
        // Delta slices (`dirty_indices`/`moved_indices`/`removed_ids`) describe only the single
        // step from `frame_id - 1` -> `frame_id`; any skipped/non-consecutive frame requires a
        // full resync so unmirrored edits are never missed.
        let full_rebuild = !self.has_synced_scene
            || epoch_changed
            || scene.frame_id != self.last_frame_id.wrapping_add(1);

        self.update_meta(focused, scale, window_title);
        self.extra_dirty.clear();

        if full_rebuild {
            if epoch_changed {
                self.bridge.advance_epoch();
            }
            self.store = NodeStore::replica();
            self.store.set_realization_order(core_realization_order());
            self.geo.sync_rects_full(rects);
            self.geo.scroll_offsets.clear();
            self.geo.scroll_offsets.extend_from_slice(scrolls);
            self.mirror_full_store(rows, text_slab, children_slab, &bound, resources);
            self.geo.rebuild_scroll_content(&self.store);
            self.last_scene_epoch = scene.scene_epoch;
            self.last_frame_id = scene.frame_id;
            self.has_synced_scene = true;
            return MirrorResult::Full;
        }

        self.geo.resize_rects_to(rects);
        self.shifted_indices.clear();
        for &raw_idx in dirty {
            if self.geo.sync_one_rect(raw_idx as usize, rects) {
                self.shifted_indices.push(raw_idx);
            }
        }
        self.geo.sync_scroll_offsets(scrolls, &mut self.extra_dirty);
        self.mirror_incremental_store(
            rows,
            text_slab,
            children_slab,
            &bound,
            dirty,
            removed,
            dirty_res,
            resources,
        );
        // `NveSceneView::moved_indices` (and dirtied containers whose origin shifted) omit
        // descendants whose parent-relative box did not change even though their absolute
        // `rects` shifted. Propagate both `geo.rects` and `extra_dirty` across those subtrees.
        if !moved.is_empty() || !self.shifted_indices.is_empty() {
            self.sync_moved_subtrees(rects, moved);
        }
        if !dirty.is_empty() || !moved.is_empty() || !removed.is_empty() {
            self.geo.rebuild_scroll_content(&self.store);
        }
        self.last_frame_id = scene.frame_id;
        MirrorResult::Incremental
    }

    fn update_meta(&mut self, focused: Option<NodeId>, scale: f32, window_title: &str) {
        self.geo.focused_node = focused;
        self.geo.scale_factor = scale;
        if self.window_title.as_str() != window_title {
            self.window_title.clear();
            self.window_title.push_str(window_title);
        }
    }

    fn bump_visit_gen(&mut self, needed: usize) -> u32 {
        if self.visit_marks.len() < needed {
            self.visit_marks.resize(needed, 0);
        }
        self.visit_gen = self.visit_gen.wrapping_add(1);
        if self.visit_gen == 0 {
            self.visit_marks.fill(0);
            self.visit_gen = 1;
        }
        self.visit_gen
    }

    fn sync_moved_subtrees(&mut self, rects: &[NveLayoutRect], moved: &[u32]) {
        let mark = self.bump_visit_gen(rects.len());
        self.walk_stack.clear();
        for &raw_idx in moved.iter().chain(self.shifted_indices.iter()) {
            if let Some(id) = self.store.id_at(raw_idx as usize) {
                self.walk_stack.push(id);
            }
        }
        while let Some(id) = self.walk_stack.pop() {
            let idx = id.index();
            if idx >= self.visit_marks.len() || self.visit_marks[idx] == mark {
                continue;
            }
            self.visit_marks[idx] = mark;
            let _ = self.geo.sync_one_rect(idx, rects);
            self.extra_dirty.push(id);
            for &child in self.store.children(id) {
                self.walk_stack.push(child);
            }
        }
    }

    fn set_aria_rel_owner(owners: &mut Vec<(NodeId, u32)>, id: NodeId, rid: u32) {
        if rid == 0 {
            Self::remove_aria_rel_owner(owners, id);
            return;
        }
        if let Some(slot) = owners.iter_mut().find(|(nid, _)| *nid == id) {
            slot.1 = rid;
        } else {
            owners.push((id, rid));
        }
    }

    fn remove_aria_rel_owner(owners: &mut Vec<(NodeId, u32)>, id: NodeId) {
        if owners.is_empty() {
            return;
        }
        if let Some(pos) = owners.iter().position(|&(nid, _)| nid == id) {
            owners.swap_remove(pos);
        }
    }

    fn mirror_full_store(
        &mut self,
        rows: &[NveNodeRow],
        text_slab: &[u8],
        children_slab: &[u32],
        bound: &BoundA11yTables<'_>,
        resources: &dyn ResourceReader,
    ) {
        self.aria_rel_owners.clear();
        let mut tx = self.store.tx();
        for (idx, row) in rows.iter().enumerate().skip(1) {
            if (row.flags & NVE_NODE_ROW_FLAG_LIVE) != 0 {
                let id = NodeId::from_index(NodeIndex(idx as u32), row.generation);
                let _ = tx.create_at(id);
            }
        }
        for (idx, row) in rows.iter().enumerate().skip(1) {
            if (row.flags & NVE_NODE_ROW_FLAG_LIVE) == 0 {
                continue;
            }
            let id = NodeId::from_index(NodeIndex(idx as u32), row.generation);
            sync_children(&mut tx, id, row, children_slab, &mut self.child_scratch);
            let rid = sync_all_tables(
                &mut tx,
                id,
                idx,
                row,
                text_slab,
                bound,
                resources,
                &mut self.patch_words,
                &mut self.res_words,
            );
            if rid != 0 {
                self.aria_rel_owners.push((id, rid));
            }
        }
        tx.commit_into(&mut self.changes);
    }

    #[allow(clippy::too_many_arguments)]
    fn mirror_incremental_store(
        &mut self,
        rows: &[NveNodeRow],
        text_slab: &[u8],
        children_slab: &[u32],
        bound: &BoundA11yTables<'_>,
        dirty: &[u32],
        removed: &[u32],
        dirty_res: &[u32],
        resources: &dyn ResourceReader,
    ) {
        let mut tx = self.store.tx();
        for &raw in removed {
            let nid = NodeId::from_raw_or_null(raw);
            if !nid.is_null() {
                Self::remove_aria_rel_owner(&mut self.aria_rel_owners, nid);
                if tx.store().is_live(nid) {
                    tx.remove(nid);
                }
            }
        }
        // Pass 1: realize every created/recycled dirty slot before wiring child lists.
        for &raw_idx in dirty {
            let idx = raw_idx as usize;
            let Some(row) = rows.get(idx) else {
                continue;
            };
            if idx == 0 {
                continue;
            }
            if (row.flags & NVE_NODE_ROW_FLAG_LIVE) != 0 {
                let new_id = NodeId::from_index(NodeIndex(idx as u32), row.generation);
                if let Some(old_id) = tx.store().id_at(idx)
                    && old_id != new_id
                {
                    Self::remove_aria_rel_owner(&mut self.aria_rel_owners, old_id);
                    tx.remove(old_id);
                }
                if !tx.store().is_live(new_id) {
                    let _ = tx.create_at(new_id);
                }
            } else if let Some(old_id) = tx.store().id_at(idx) {
                Self::remove_aria_rel_owner(&mut self.aria_rel_owners, old_id);
                tx.remove(old_id);
            }
        }
        // Pass 2: update children and table rows on every live dirty slot.
        for &raw_idx in dirty {
            let idx = raw_idx as usize;
            let Some(row) = rows.get(idx) else {
                continue;
            };
            if idx == 0 || (row.flags & NVE_NODE_ROW_FLAG_LIVE) == 0 {
                continue;
            }
            let id = NodeId::from_index(NodeIndex(idx as u32), row.generation);
            sync_children(&mut tx, id, row, children_slab, &mut self.child_scratch);
            let rid = sync_all_tables(
                &mut tx,
                id,
                idx,
                row,
                text_slab,
                bound,
                resources,
                &mut self.patch_words,
                &mut self.res_words,
            );
            Self::set_aria_rel_owner(&mut self.aria_rel_owners, id, rid);
        }
        // Pass 3: refresh in-place republished `AriaRelations` resources only on tracked owners
        // (avoids scanning every `Accessibility` row when unrelated resources are dirty).
        if !dirty_res.is_empty() && !self.aria_rel_owners.is_empty() {
            for &(owner, rid) in &self.aria_rel_owners {
                if dirty_res.binary_search(&rid).is_ok() && tx.store().is_live(owner) {
                    sync_aria_relations(&mut tx, owner, rid, resources, &mut self.res_words);
                }
            }
        }
        tx.commit_into(&mut self.changes);
    }
}

enum MirrorResult {
    Unchanged,
    MetadataOnly,
    Full,
    Incremental,
}
