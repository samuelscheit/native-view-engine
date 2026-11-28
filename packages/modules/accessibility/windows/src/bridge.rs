//! Incremental `NveSceneView` -> replica `NodeStore` mirror feeding `AccessKitTreeBridge`.
//!
//! Keeps `nve-accessibility` as the single source of truth for ARIA roles, labels, states,
//! `AriaRelations`, `TextInput` `TextRun`s, scroll bounds, transforms, and epoch-checked actions.

use accesskit::{ActionRequest, NodeId as AccessKitNodeId, TreeUpdate};
use nve_abi::{
    NveLayoutRect, NveNodeRow, NveSceneView, NveScrollOffset, NVE_NODE_ROW_FLAG_LIVE,
    NVE_RUNTIME_SCHEMA_HASH,
};
use nve_accessibility::{
    A11yGeometryProvider, AccessKitTreeBridge, ResolvedA11yAction, StoreEpoch,
};
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeIndex, NodeStore, NveTable};
use nve_tables::tables::{core_realization_order, Accessibility, Scroll};

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

struct SceneGeo {
    rects: Vec<LayoutRect>,
    scroll_offsets: Vec<NveScrollOffset>,
    scroll_content: Vec<(u32, f32, f32)>,
    focused_node: Option<NodeId>,
    scale_factor: f32,
}

impl Default for SceneGeo {
    fn default() -> Self {
        Self {
            rects: Vec::new(),
            scroll_offsets: Vec::new(),
            scroll_content: Vec::new(),
            focused_node: None,
            scale_factor: 1.0,
        }
    }
}

impl A11yGeometryProvider for SceneGeo {
    #[inline]
    fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    #[inline]
    fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
        self.rects.get(id.index()).copied()
    }

    #[inline]
    fn scroll_offset(&self, id: NodeId) -> (f32, f32) {
        self.scroll_offsets
            .binary_search_by_key(&id.raw(), |s| s.node_id)
            .map_or((0.0, 0.0), |i| {
                (self.scroll_offsets[i].x, self.scroll_offsets[i].y)
            })
    }

    #[inline]
    fn scroll_content_size(&self, id: NodeId) -> Option<(f32, f32)> {
        self.scroll_content
            .binary_search_by_key(&id.raw(), |&(nid, _, _)| nid)
            .ok()
            .map(|i| (self.scroll_content[i].1, self.scroll_content[i].2))
    }

    #[inline]
    fn focused_node(&self) -> Option<NodeId> {
        self.focused_node
    }
}

/// Platform-independent incremental mirror from flat C-ABI [`NveSceneView`] snapshots to
/// [`AccessKitTreeBridge`].
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
    scroll_dirty: Vec<NodeId>,
    res_words: Vec<u32>,
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
            scroll_dirty: Vec::new(),
            res_words: Vec::new(),
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
            MirrorResult::Incremental { force_full_diff } => self.bridge.sync_changes_with_dirty(
                &self.store,
                &self.changes,
                &self.geo,
                &self.window_title,
                &self.scroll_dirty,
                force_full_diff,
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
        let full_rebuild = !self.has_synced_scene || scene.scene_epoch != self.last_scene_epoch;

        self.update_meta(focused, scale, window_title);
        self.scroll_dirty.clear();

        if full_rebuild {
            if self.has_synced_scene {
                self.bridge.advance_epoch();
            }
            self.store = NodeStore::replica();
            self.store.set_realization_order(core_realization_order());
            self.sync_rects_full(rects);
            self.geo.scroll_offsets.clear();
            self.geo.scroll_offsets.extend_from_slice(scrolls);
            self.mirror_full_store(rows, text_slab, children_slab, &bound, resources);
            self.rebuild_scroll_content();
            self.last_scene_epoch = scene.scene_epoch;
            self.last_frame_id = scene.frame_id;
            self.has_synced_scene = true;
            return MirrorResult::Full;
        }

        self.sync_rects_incremental(rects, dirty, moved);
        self.sync_scroll_offsets(scrolls);
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
        self.rebuild_scroll_content();
        self.last_frame_id = scene.frame_id;
        MirrorResult::Incremental {
            force_full_diff: !moved.is_empty(),
        }
    }

    fn update_meta(&mut self, focused: Option<NodeId>, scale: f32, window_title: &str) {
        self.geo.focused_node = focused;
        self.geo.scale_factor = scale;
        if self.window_title.as_str() != window_title {
            self.window_title.clear();
            self.window_title.push_str(window_title);
        }
    }

    fn sync_rects_full(&mut self, rects: &[NveLayoutRect]) {
        self.geo.rects.clear();
        self.geo.rects.reserve(rects.len());
        for r in rects {
            self.geo.rects.push(LayoutRect {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            });
        }
    }

    fn sync_rects_incremental(
        &mut self,
        rects: &[NveLayoutRect],
        dirty: &[u32],
        moved: &[u32],
    ) {
        if self.geo.rects.len() < rects.len() {
            let old = self.geo.rects.len();
            self.geo.rects.resize(
                rects.len(),
                LayoutRect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                },
            );
            for (dst, src) in self.geo.rects[old..].iter_mut().zip(&rects[old..]) {
                *dst = LayoutRect {
                    x: src.x,
                    y: src.y,
                    width: src.width,
                    height: src.height,
                };
            }
        } else {
            self.geo.rects.truncate(rects.len());
        }
        for &raw_idx in dirty.iter().chain(moved.iter()) {
            let idx = raw_idx as usize;
            if let (Some(dst), Some(src)) = (self.geo.rects.get_mut(idx), rects.get(idx)) {
                *dst = LayoutRect {
                    x: src.x,
                    y: src.y,
                    width: src.width,
                    height: src.height,
                };
            }
        }
    }

    fn sync_scroll_offsets(&mut self, next: &[NveScrollOffset]) {
        let prev = &self.geo.scroll_offsets;
        let (mut i, mut j) = (0usize, 0usize);
        while i < prev.len() && j < next.len() {
            let a = prev[i];
            let b = next[j];
            if a.node_id == b.node_id {
                if (a.x - b.x).abs() > 1e-6 || (a.y - b.y).abs() > 1e-6 {
                    let nid = NodeId::from_raw_or_null(a.node_id);
                    if !nid.is_null() {
                        self.scroll_dirty.push(nid);
                    }
                }
                i += 1;
                j += 1;
            } else if a.node_id < b.node_id {
                let nid = NodeId::from_raw_or_null(a.node_id);
                if !nid.is_null() {
                    self.scroll_dirty.push(nid);
                }
                i += 1;
            } else {
                let nid = NodeId::from_raw_or_null(b.node_id);
                if !nid.is_null() {
                    self.scroll_dirty.push(nid);
                }
                j += 1;
            }
        }
        for a in &prev[i..] {
            let nid = NodeId::from_raw_or_null(a.node_id);
            if !nid.is_null() {
                self.scroll_dirty.push(nid);
            }
        }
        for b in &next[j..] {
            let nid = NodeId::from_raw_or_null(b.node_id);
            if !nid.is_null() {
                self.scroll_dirty.push(nid);
            }
        }
        self.geo.scroll_offsets.clear();
        self.geo.scroll_offsets.extend_from_slice(next);
    }

    fn mirror_full_store(
        &mut self,
        rows: &[NveNodeRow],
        text_slab: &[u8],
        children_slab: &[u32],
        bound: &BoundA11yTables<'_>,
        resources: &dyn ResourceReader,
    ) {
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
            sync_all_tables(
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
            if !nid.is_null() && tx.store().is_live(nid) {
                tx.remove(nid);
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
                    tx.remove(old_id);
                }
                if !tx.store().is_live(new_id) {
                    let _ = tx.create_at(new_id);
                }
            } else if let Some(old_id) = tx.store().id_at(idx) {
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
            sync_all_tables(
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
        }
        // Pass 3: refresh in-place republished `AriaRelations` resources on untouched nodes.
        if !dirty_res.is_empty()
            && let Some(a11y_tbl) = bound.accessibility
        {
            let rel_word = Accessibility::RELATIONS.word() as usize;
            for r in 0..a11y_tbl.row_count {
                let Some(&owner) = a11y_tbl.owners.get(r) else {
                    continue;
                };
                if !tx.store().is_live(owner) {
                    continue;
                }
                let start = r * a11y_tbl.words;
                let Some(words) = a11y_tbl.rows.get(start..start + a11y_tbl.words) else {
                    continue;
                };
                let rid = words.get(rel_word).copied().unwrap_or(0);
                if rid != 0 && dirty_res.binary_search(&rid).is_ok() {
                    sync_aria_relations(&mut tx, owner, rid, resources, &mut self.res_words);
                }
            }
        }
        tx.commit_into(&mut self.changes);
    }

    fn rebuild_scroll_content(&mut self) {
        self.geo.scroll_content.clear();
        let Some(storage) = self.store.table_storage(Scroll::slot()) else {
            return;
        };
        for &owner in storage.owners {
            if !self.store.is_live(owner) {
                continue;
            }
            let Some(parent_rect) = self.geo.rects.get(owner.index()).copied() else {
                continue;
            };
            let mut max_w = parent_rect.width.max(0.0);
            let mut max_h = parent_rect.height.max(0.0);
            for &child in self.store.children(owner) {
                if let Some(cr) = self.geo.rects.get(child.index()) {
                    max_w = max_w.max((cr.x + cr.width) - parent_rect.x);
                    max_h = max_h.max((cr.y + cr.height) - parent_rect.y);
                }
            }
            self.geo.scroll_content.push((owner.raw(), max_w, max_h));
        }
        self.geo.scroll_content.sort_unstable_by_key(|&(nid, _, _)| nid);
    }
}

enum MirrorResult {
    Unchanged,
    MetadataOnly,
    Full,
    Incremental { force_full_diff: bool },
}
