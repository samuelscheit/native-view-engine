//! Zero-copy [`NveRuntimeSceneView`] publication from the active `NodeStore`: invalidation,
//! pointer-drift validation, and per-frame publish of rows, rects, slabs, tables, and resources.

use nve_abi::{
    NveLayoutRect, NveNodeRow, NveRuntimeSceneView, NveScrollOffset, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_store::{ChangeSet, NodeId, NodeStore, TableSlot};

use crate::builder::Timer;
use crate::state::{layout_rect_to_abi, SurfaceState};

impl SurfaceState {
    /// Clears all borrowed pointers in `published_view` and `published_tables` so an early error,
    /// app detach/swap, or unwind can never expose dangling pointers to `scene_acquire`.
    pub(crate) fn invalidate_published_view(&mut self) {
        self.published_view = NveRuntimeSceneView {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: self.frame_id,
            ..NveRuntimeSceneView::default()
        };
        self.published_resources = core::ptr::null();
        self.published_tables.clear();
        self.published_dirty_buf.clear();
        self.dirty_indices.clear();
        self.removed_ids.clear();
        self.published_scroll.clear();
        self.metrics.dirty_nodes = 0;
    }

    /// Verifies that every borrowed slab pointer and every published `NveTableView` column pointer
    /// and row count in `published_view` still matches `store` identically.
    pub(crate) fn published_view_matches_store(&self, store: &NodeStore) -> bool {
        if !self.published_resources.is_null()
            && !core::ptr::eq(self.published_resources, store.resources())
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
            && if rows.is_empty() {
                v.rows.is_null()
            } else {
                v.rows == rows.as_ptr().cast::<NveNodeRow>()
            };
        let text_ok = v.text_slab_len as usize == text.len()
            && if text.is_empty() {
                v.text_slab.is_null()
            } else {
                v.text_slab == text.as_ptr()
            };
        let children_ok = v.children_slab_len as usize == children.len()
            && if children.is_empty() {
                v.children_slab.is_null()
            } else {
                v.children_slab == children.as_ptr().cast::<u32>()
            };
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
            let Some((t_rows, t_owners, t_row_count)) =
                store.table_storage(TableSlot(slot_u8))
            else {
                return false;
            };
            let rows_ptr_ok = if t_rows.is_empty() {
                tv.rows.is_null()
            } else {
                tv.rows == t_rows.as_ptr()
            };
            let owners_ptr_ok = if t_owners.is_empty() {
                tv.owners.is_null()
            } else {
                tv.owners == t_owners.as_ptr()
            };
            if tv.row_count as usize != t_row_count || !rows_ptr_ok || !owners_ptr_ok {
                return false;
            }
        }
        true
    }

    /// Synchronizes dense `self.rects`, deduplicated dirty/removed scratch, and `NveTableView`s
    /// directly from `store` and `self.scene` with zero heap allocations once capacities warm.
    pub(crate) fn publish_scene(&mut self, store: &NodeStore, changes: &ChangeSet, laid_out: bool) {
        let t_pub = Timer::start();
        let cap = store.capacity();
        if self.rects.len() < cap {
            self.rects.resize(cap, NveLayoutRect::default());
        } else if self.rects.len() > cap {
            self.rects.truncate(cap);
        }

        if self.dirty_stamp.len() < cap {
            self.dirty_stamp.resize(cap, 0);
        }
        self.dirty_epoch = self.dirty_epoch.wrapping_add(1);
        if self.dirty_epoch == 0 {
            self.dirty_stamp.fill(0);
            self.dirty_epoch = 1;
        }
        let epoch = self.dirty_epoch;
        self.dirty_indices.clear();

        // `dirty_indices` contract: every live node whose row, tables, children OR layout rect
        // changed. Relayout moves untouched nodes (siblings, ancestors), so retained-view hosts
        // must see them without diffing every rect themselves.
        if laid_out {
            for idx in 1..cap {
                let id = store.id_at(idx);
                let r = id
                    .and_then(|id| self.scene.layout_of(id))
                    .map_or(NveLayoutRect::default(), layout_rect_to_abi);
                if id.is_some() && self.rects[idx] != r && self.dirty_stamp[idx] != epoch {
                    self.dirty_stamp[idx] = epoch;
                    self.dirty_indices.push(idx as u32);
                }
                self.rects[idx] = r;
            }
        }
        for &id in changes
            .created
            .iter()
            .chain(&changes.touched)
            .chain(&changes.children)
        {
            let idx = id.index();
            if idx < cap && store.is_live(id) && self.dirty_stamp[idx] != epoch {
                self.dirty_stamp[idx] = epoch;
                self.dirty_indices.push(idx as u32);
            }
        }

        self.removed_ids.clear();
        for &id in &changes.removed {
            self.removed_ids.push(id.raw());
        }

        store.publish_table_views(
            changes,
            &mut self.published_tables,
            &mut self.published_dirty_buf,
        );

        self.published_scroll.clear();
        let scroll = self.scene.scroll_offsets();
        for (node_id, _) in scroll.iter() {
            let (x, y) = scroll.visual_shift(store, NodeId(node_id));
            self.published_scroll.push(NveScrollOffset { node_id, x, y });
        }
        self.published_scroll.sort_unstable_by_key(|s| s.node_id);

        let rows_slice = store.node_rows();
        let text_slab = store.text_slab();
        let children_slab = store.children_slab();

        self.published_view = NveRuntimeSceneView {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: self.frame_id,
            node_capacity: rows_slice.len() as u32,
            dirty_count: self.dirty_indices.len() as u32,
            removed_count: self.removed_ids.len() as u32,
            text_slab_len: text_slab.len() as u32,
            children_slab_len: children_slab.len() as u32,
            table_count: self.published_tables.len() as u32,
            scroll_count: self.published_scroll.len() as u32,
            _reserved: 0,
            rows: if rows_slice.is_empty() {
                core::ptr::null()
            } else {
                // `NodeRow` and `NveNodeRow` are locked bit-for-bit identical in `nve_store::store`.
                rows_slice.as_ptr().cast::<NveNodeRow>()
            },
            rects: if self.rects.is_empty() {
                core::ptr::null()
            } else {
                self.rects.as_ptr()
            },
            text_slab: if text_slab.is_empty() {
                core::ptr::null()
            } else {
                text_slab.as_ptr()
            },
            children_slab: if children_slab.is_empty() {
                core::ptr::null()
            } else {
                // `NodeId` is `#[repr(transparent)]` over `u32`.
                children_slab.as_ptr().cast::<u32>()
            },
            dirty_indices: if self.dirty_indices.is_empty() {
                core::ptr::null()
            } else {
                self.dirty_indices.as_ptr()
            },
            removed_ids: if self.removed_ids.is_empty() {
                core::ptr::null()
            } else {
                self.removed_ids.as_ptr()
            },
            tables: if self.published_tables.is_empty() {
                core::ptr::null()
            } else {
                self.published_tables.as_ptr()
            },
            scroll_offsets: if self.published_scroll.is_empty() {
                core::ptr::null()
            } else {
                self.published_scroll.as_ptr()
            },
        };

        self.published_resources = store.resources();
        self.metrics.frame_id = self.frame_id;
        self.metrics.publish_us = t_pub.elapsed_us();
        self.metrics.dirty_nodes = self.dirty_indices.len() as u32;
        if laid_out || !changes.created.is_empty() || !changes.removed.is_empty() {
            self.metrics.live_nodes = store.stats().live_nodes as u32;
        }
        self.needs_publish = false;
    }
}
