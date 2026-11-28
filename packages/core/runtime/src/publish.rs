//! Zero-copy [`NveSceneView`] publication from the active `NodeStore`: invalidation,
//! pointer-drift validation, and per-frame publish of rows, rects, slabs, tables, and resources.

use nve_abi::table::NVE_ROW_INDEX_PAGE_LEN;
use nve_abi::{slab_ptr, NveNodeRow, NveSceneView, NveScrollOffset, NVE_RUNTIME_SCHEMA_HASH};
use nve_layout::ScrollOffsets;
use nve_store::{ChangeSet, NodeId, NodeStore, TableSlot};

use crate::builder::Timer;
use crate::state::{layout_rect_to_abi, PublishedResources, SurfaceState};

impl SurfaceState {
    /// Clears all borrowed pointers in `published_view` and `published_tables` so an early error,
    /// app detach/swap, or unwind can never expose dangling pointers to `scene_acquire`.
    pub(crate) fn invalidate_published_view(&mut self) {
        self.published_view = NveSceneView {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: self.frame_id,
            scene_epoch: self.scene_epoch,
            ..NveSceneView::default()
        };
        self.published_resources = PublishedResources::None;
        self.published_tables.clear();
        self.published_dirty_buf.clear();
        self.delta.clear_sets();
        self.published_scroll.clear();
        self.dirty_resources.clear();
        self.metrics.dirty_nodes = 0;
    }

    /// Verifies that every borrowed slab pointer and every published `NveTableView` column pointer
    /// and row count in `published_view` still matches `store` (whose resources are `resources`).
    pub(crate) fn published_view_matches_store(
        &self,
        store: &NodeStore,
        resources: PublishedResources,
    ) -> bool {
        if self.published_resources != PublishedResources::None
            && self.published_resources != resources
        {
            return false;
        }
        let v = &self.published_view;
        if v.node_capacity == 0 && v.table_count == 0 {
            return true;
        }
        let rows = store.node_rows();
        let text = store.text_slab();
        let children = store.children_slab();
        let rows_ok = v.node_capacity as usize == rows.len()
            && v.rows == slab_ptr(rows).cast::<NveNodeRow>();
        let text_ok = v.text_slab_len as usize == text.len() && v.text_slab == slab_ptr(text);
        let children_ok = v.children_slab_len as usize == children.len()
            && v.children_slab == slab_ptr(children).cast::<u32>();
        if !(rows_ok && text_ok && children_ok) {
            return false;
        }
        if v.table_count as usize != self.published_tables.len() {
            return false;
        }
        if !self.published_tables.is_empty() && v.tables != self.published_tables.as_ptr() {
            return false;
        }
        for tv in &self.published_tables {
            let Ok(slot_u8) = u8::try_from(tv.slot) else {
                return false;
            };
            let Some(t) = store.table_storage(TableSlot(slot_u8)) else {
                return false;
            };
            // The paged row index grows (and may reallocate) as rows attach.
            if tv.row_count as usize != t.row_count
                || tv.rows != slab_ptr(t.rows)
                || tv.owners != slab_ptr(t.owners)
                || tv.row_index_dir_len as usize != t.row_index_dir.len()
                || tv.row_index_dir != slab_ptr(t.row_index_dir)
                || tv.row_index_page_count as usize * NVE_ROW_INDEX_PAGE_LEN as usize
                    != t.row_index_pages.len()
                || tv.row_index_pages != slab_ptr(t.row_index_pages)
            {
                return false;
            }
        }
        true
    }

    /// Synchronizes the published rects, the exact disjoint dirty / moved / removed sets
    /// ([`crate::delta::SceneDelta`]), and `NveTableView`s directly from `store` and `self.scene`
    /// with zero heap allocations once capacities warm.
    pub(crate) fn publish_scene(
        &mut self,
        store: &NodeStore,
        resources: PublishedResources,
        changes: &ChangeSet,
        laid_out: bool,
    ) {
        let t_pub = Timer::start();
        if self.epoch_resources != PublishedResources::None && self.epoch_resources != resources {
            self.scene_epoch = self.scene_epoch.wrapping_add(1);
        }
        self.epoch_resources = resources;
        let scene = &self.scene;
        self.delta.record(store, changes, laid_out, |id| {
            scene.layout_of(id).map(layout_rect_to_abi)
        });

        store.publish_table_views(
            changes,
            &mut self.published_tables,
            &mut self.published_dirty_buf,
        );

        self.published_scroll.clear();
        let scroll = self.scene.scroll_offsets();
        for (node_id, (sx, sy)) in scroll.iter() {
            let (x, y) = ScrollOffsets::visual_shift_with(store, NodeId(node_id), sx, sy);
            self.published_scroll.push(NveScrollOffset { node_id, x, y });
        }
        self.published_scroll.sort_unstable_by_key(|s| s.node_id);

        // Resource ids whose bytes changed this frame (created, recycled from the free list, or an
        // ephemeral rewrite in place): host caches keyed by id drop exactly these.
        self.dirty_resources.clear();
        let store_resources = store.resources();
        self.dirty_resources.extend(
            changes
                .resources_defined
                .iter()
                .filter(|&&id| store_resources.is_live(id))
                .map(|id| id.0),
        );
        self.dirty_resources.sort_unstable();
        self.dirty_resources.dedup();

        let rows_slice = store.node_rows();
        let text_slab = store.text_slab();
        let children_slab = store.children_slab();

        self.published_view = NveSceneView {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: self.frame_id,
            node_capacity: rows_slice.len() as u32,
            dirty_count: self.delta.dirty.len() as u32,
            removed_count: self.delta.removed.len() as u32,
            text_slab_len: text_slab.len() as u32,
            children_slab_len: children_slab.len() as u32,
            table_count: self.published_tables.len() as u32,
            scroll_count: self.published_scroll.len() as u32,
            scene_epoch: self.scene_epoch,
            dirty_resource_count: self.dirty_resources.len() as u32,
            moved_count: self.delta.moved.len() as u32,
            // `NodeRow` and `NveNodeRow` are locked bit-for-bit identical in `nve_store::store`.
            rows: slab_ptr(rows_slice).cast::<NveNodeRow>(),
            rects: slab_ptr(&self.delta.rects),
            text_slab: slab_ptr(text_slab),
            // `NodeId` is `#[repr(transparent)]` over `u32`.
            children_slab: slab_ptr(children_slab).cast::<u32>(),
            dirty_indices: slab_ptr(&self.delta.dirty),
            removed_ids: slab_ptr(&self.delta.removed),
            tables: slab_ptr(&self.published_tables),
            scroll_offsets: slab_ptr(&self.published_scroll),
            dirty_resources: slab_ptr(&self.dirty_resources),
            moved_indices: slab_ptr(&self.delta.moved),
        };

        self.published_resources = resources;
        self.metrics.frame_id = self.frame_id;
        self.metrics.publish_us = t_pub.elapsed_us();
        self.metrics.dirty_nodes = self.delta.dirty.len() as u32;
        if laid_out || !changes.created.is_empty() || !changes.removed.is_empty() {
            self.metrics.live_nodes = store.stats().live_nodes as u32;
        }
        self.needs_publish = false;
    }
}
