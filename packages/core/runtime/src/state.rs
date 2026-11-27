//! Internal [`SurfaceState`] and [`RuntimeInner`] owning `LaidOutScene`, reusable `ChangeSet`
//! scratch buffers, `WireDecoder`, `FrameTickHook`s, and zero-copy [`NveRuntimeSceneView`]
//! publication.

use nve_abi::table::NveTableView;
use nve_abi::{
    FrameId, NveLayoutRect, NveNodeRow, NveRuntimeMetrics, NveRuntimeSceneView,
    NVE_RUNTIME_SCHEMA_HASH,
};
use nve_engine::LayoutRect;
use nve_layout::{DynLayoutEngine, LaidOutScene};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore, RealizationOrder, TableSlot};

use crate::app::{AppFrame, FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeError, Timer};
use crate::pair::{PairInboundFrame, PairSession};

// Full field-by-field + little-endian proof (including `pub(crate)` `flags` and
// `ChildSpan::{off, len}`) is enforced at compile time in `nve_store::store`.
const _: () = {
    use core::mem::{align_of, size_of};
    assert!(cfg!(target_endian = "little"));
    assert!(size_of::<nve_store::NodeRow>() == size_of::<NveNodeRow>());
    assert!(align_of::<nve_store::NodeRow>() >= align_of::<NveNodeRow>());
};

/// Upper bound on synchronous `render_frame -> poll_async_work` pumps within a single `tick()`.
const MAX_RENDER_PUMPS: usize = 4;

/// Upper bound on paired inbound wire frames drained in a single `tick()` to prevent UI starvation.
const MAX_PAIR_FRAMES_PER_TICK: usize = 64;

/// Disjoint non-store surface state so `app` / `replica_store` and `state` can be borrowed
/// simultaneously with zero `unsafe` pointer gymnastics or `Box` moves.
pub(crate) struct SurfaceState {
    pub(crate) frame_id: FrameId,
    /// Cached engine name so `layout_engine_name()` never touches `&StoreLayoutEngine` cross-thread.
    pub(crate) layout_engine_name: &'static str,
    pub(crate) realization_order: RealizationOrder,
    pub(crate) wire_decoder: WireDecoder,
    pub(crate) wire_changes: ChangeSet,
    pub(crate) hook_changes: ChangeSet,
    pub(crate) merged_changes: ChangeSet,
    pub(crate) empty_changes: ChangeSet,
    pub(crate) scene: LaidOutScene<DynLayoutEngine>,
    pub(crate) tick_hooks: Vec<Box<dyn FrameTickHook>>,
    pub(crate) rects: Vec<NveLayoutRect>,
    pub(crate) published_tables: Vec<NveTableView>,
    pub(crate) published_dirty_buf: Vec<u32>,
    pub(crate) dirty_indices: Vec<u32>,
    pub(crate) dirty_stamp: Vec<u32>,
    pub(crate) dirty_epoch: u32,
    pub(crate) removed_ids: Vec<u32>,
    pub(crate) published_view: NveRuntimeSceneView,
    pub(crate) metrics: NveRuntimeMetrics,
    pub(crate) needs_render: bool,
    pub(crate) needs_publish: bool,
}

impl SurfaceState {
    /// Clears all borrowed pointers in `published_view` and `published_tables` so an early error,
    /// app detach/swap, or unwind can never expose dangling pointers to `scene_acquire`.
    pub(crate) fn invalidate_published_view(&mut self) {
        self.published_view = NveRuntimeSceneView {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: self.frame_id,
            ..NveRuntimeSceneView::default()
        };
        self.published_tables.clear();
        self.published_dirty_buf.clear();
        self.dirty_indices.clear();
        self.removed_ids.clear();
        self.metrics.dirty_nodes = 0;
    }

    /// Verifies that every borrowed slab pointer and every published `NveTableView` column pointer
    /// and row count in `published_view` still matches `store` identically.
    pub(crate) fn published_view_matches_store(&self, store: &NodeStore) -> bool {
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

        if laid_out {
            for idx in 1..cap {
                let r = store
                    .id_at(idx)
                    .and_then(|id| self.scene.layout_of(id))
                    .map_or(NveLayoutRect::default(), layout_rect_to_abi);
                self.rects[idx] = r;
            }
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
        };

        self.metrics.frame_id = self.frame_id;
        self.metrics.publish_us = t_pub.elapsed_us();
        self.metrics.dirty_nodes = self.dirty_indices.len() as u32;
        if laid_out || !changes.created.is_empty() || !changes.removed.is_empty() {
            self.metrics.live_nodes = store.stats().live_nodes as u32;
        }
        self.needs_publish = false;
    }

    pub(crate) fn apply_wire_and_publish(
        &mut self,
        replica: &mut NodeStore,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        // Validate the 24-byte NVE2 header BEFORE invalidating the live published view so
        // truncated/misaligned frames fail cleanly without blanking the screen.
        WireDecoder::validate_header(bytes)
            .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
        self.invalidate_published_view();
        {
            let mut tx = replica.tx();
            self.wire_decoder
                .apply(&mut tx, bytes)
                .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
            tx.commit_into(&mut self.wire_changes);
        }
        self.metrics.wire_bytes_rx = self
            .metrics
            .wire_bytes_rx
            .saturating_add(bytes.len() as u64);
        self.frame_id = self.frame_id.wrapping_add(1).max(1);
        let t_lay = Timer::start();
        let laid_out = self.scene.commit(replica, &self.wire_changes);
        self.scene
            .reconcile_scroll_offsets_if_needed(replica, &self.wire_changes, laid_out);
        self.metrics.layout_us = t_lay.elapsed_us();
        let changes = std::mem::take(&mut self.wire_changes);
        self.publish_scene(replica, &changes, laid_out);
        self.wire_changes = changes;
        Ok(())
    }
}

pub(crate) struct RuntimeInner {
    pub(crate) app: Option<Box<dyn RuntimeApp>>,
    pub(crate) pair: Option<PairSession>,
    pub(crate) replica_store: NodeStore,
    pub(crate) state: SurfaceState,
}

impl RuntimeInner {
    /// Resets `replica_store`, `wire_decoder`, `scene`, `rects`, and `published_view` when
    /// attaching, replacing, or detaching an app so stale nodes, rects, or wire table IDs never
    /// leak across sessions.
    pub(crate) fn reset_for_app_change(&mut self) {
        self.state.invalidate_published_view();
        self.state.rects.clear();
        self.state.scene.reset();
        self.state.wire_decoder = WireDecoder::default();
        self.replica_store = NodeStore::replica();
        self.replica_store
            .set_realization_order(self.state.realization_order.clone());
    }

    #[inline]
    pub(crate) fn active_store(&self) -> &NodeStore {
        self.app
            .as_ref()
            .and_then(|a| a.direct_store())
            .unwrap_or(&self.replica_store)
    }

    #[inline]
    pub(crate) fn split_store_and_state(&mut self) -> (&NodeStore, &mut SurfaceState) {
        let store = self
            .app
            .as_ref()
            .and_then(|a| a.direct_store())
            .unwrap_or(&self.replica_store);
        (store, &mut self.state)
    }

    #[inline]
    pub(crate) fn split_store_mut_and_state(&mut self) -> (&mut NodeStore, &mut SurfaceState) {
        let store = self
            .app
            .as_mut()
            .and_then(|a| a.direct_store_mut())
            .unwrap_or(&mut self.replica_store);
        (store, &mut self.state)
    }

    fn record_changes(dst: &mut ChangeSet, has_changes: &mut bool, src: &ChangeSet) {
        if !*has_changes {
            dst.copy_from(src);
            *has_changes = true;
        } else {
            dst.merge_from(src);
        }
    }

    /// Runs one host tick. Returns `(changed, needs_follow_up_wake)`.
    pub(crate) fn tick(&mut self, dt_s: f32) -> Result<(bool, bool), RuntimeError> {
        let mut has_changes = false;

        if let Some(ref mut app) = self.app
            && app.poll_async_work()
        {
            self.state.needs_render = true;
        }

        let mut pumps = 0usize;
        while self.state.needs_render && pumps < MAX_RENDER_PUMPS {
            let Some(ref mut app) = self.app else {
                self.state.needs_render = false;
                break;
            };
            self.state.needs_render = false;
            pumps += 1;
            let next_frame = self.state.frame_id.wrapping_add(1).max(1);
            let has_direct_store = app.direct_store().is_some();
            let t_rec = Timer::start();
            match app.render_frame(next_frame) {
                AppFrame::Clean => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                }
                AppFrame::Direct { changes } => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                    if !has_direct_store {
                        return Err(RuntimeError::InvalidArg(
                            "AppFrame::Direct returned by app without direct_store()",
                        ));
                    }
                    Self::record_changes(&mut self.state.merged_changes, &mut has_changes, changes);
                    self.state.needs_publish = true;
                }
                AppFrame::Wire(bytes) => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                    if !bytes.is_empty() {
                        WireDecoder::validate_header(bytes)
                            .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                        self.state.invalidate_published_view();
                        {
                            let mut tx = self.replica_store.tx();
                            self.state
                                .wire_decoder
                                .apply(&mut tx, bytes)
                                .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                            tx.commit_into(&mut self.state.wire_changes);
                        }
                        self.state.metrics.wire_bytes_rx = self
                            .state
                            .metrics
                            .wire_bytes_rx
                            .saturating_add(bytes.len() as u64);
                        Self::record_changes(
                            &mut self.state.merged_changes,
                            &mut has_changes,
                            &self.state.wire_changes,
                        );
                        self.state.needs_publish = true;
                    }
                }
            }
            if app.poll_async_work() {
                self.state.needs_render = true;
            }
        }

        let mut pair_follow_up = false;
        if self.pair.is_some() {
            pair_follow_up = self.drain_pair_frames(&mut has_changes)?;
        }

        let needs_follow_up_wake =
            (self.state.needs_render && self.app.is_some()) || pair_follow_up;

        if !self.state.tick_hooks.is_empty() {
            let (store, state) = self.split_store_mut_and_state();
            let mut hook_requested = false;
            {
                let mut tx = store.tx();
                for hook in &mut state.tick_hooks {
                    hook_requested |= hook.tick(&mut tx, dt_s);
                }
                tx.commit_into(&mut state.hook_changes);
            }
            if !state.hook_changes.is_empty() {
                Self::record_changes(
                    &mut state.merged_changes,
                    &mut has_changes,
                    &state.hook_changes,
                );
                state.needs_publish = true;
            } else if hook_requested {
                state.needs_publish = true;
            }
        }

        let changed = if has_changes || self.state.needs_publish {
            let (store, state) = self.split_store_and_state();
            state.invalidate_published_view();
            state.frame_id = state.frame_id.wrapping_add(1).max(1);
            let t_lay = Timer::start();
            let laid_out = if has_changes {
                let changes = std::mem::take(&mut state.merged_changes);
                let r = state.scene.commit(store, &changes);
                state
                    .scene
                    .reconcile_scroll_offsets_if_needed(store, &changes, r);
                state.merged_changes = changes;
                r
            } else {
                let r = state.scene.recompute_layout(store);
                let empty = std::mem::take(&mut state.empty_changes);
                state
                    .scene
                    .reconcile_scroll_offsets_if_needed(store, &empty, r);
                state.empty_changes = empty;
                r
            };
            state.metrics.layout_us = t_lay.elapsed_us();
            if has_changes {
                let changes = std::mem::take(&mut state.merged_changes);
                state.publish_scene(store, &changes, laid_out);
                state.merged_changes = changes;
            } else {
                let empty = std::mem::take(&mut state.empty_changes);
                state.publish_scene(store, &empty, laid_out);
                state.empty_changes = empty;
            }
            true
        } else {
            false
        };

        Ok((changed, needs_follow_up_wake))
    }

    fn drain_pair_frames(&mut self, has_changes: &mut bool) -> Result<bool, RuntimeError> {
        let mut drained = 0usize;
        while drained < MAX_PAIR_FRAMES_PER_TICK {
            let Some(frame) = self.pair.as_ref().and_then(PairSession::try_recv_scene_frame) else {
                break;
            };
            drained += 1;
            match frame {
                PairInboundFrame::Keyframe(bytes) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    WireDecoder::validate_header(&bytes)
                        .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                    self.reset_for_app_change();
                    self.state.merged_changes.copy_from(&self.state.empty_changes);
                    *has_changes = false;
                    {
                        let mut tx = self.replica_store.tx();
                        self.state
                            .wire_decoder
                            .apply(&mut tx, &bytes)
                            .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                        tx.commit_into(&mut self.state.wire_changes);
                    }
                    Self::record_changes(
                        &mut self.state.merged_changes,
                        has_changes,
                        &self.state.wire_changes,
                    );
                    self.state.needs_publish = true;
                }
                PairInboundFrame::Delta(bytes) => {
                    if bytes.is_empty() {
                        continue;
                    }
                    let (frame_num, _ops) = WireDecoder::validate_header(&bytes)
                        .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                    if frame_num == 1 && self.replica_store.is_live(nve_store::NodeId::ROOT) {
                        self.reset_for_app_change();
                        self.state.merged_changes.copy_from(&self.state.empty_changes);
                        *has_changes = false;
                    } else {
                        self.state.invalidate_published_view();
                    }
                    {
                        let mut tx = self.replica_store.tx();
                        self.state
                            .wire_decoder
                            .apply(&mut tx, &bytes)
                            .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
                        tx.commit_into(&mut self.state.wire_changes);
                    }
                    Self::record_changes(
                        &mut self.state.merged_changes,
                        has_changes,
                        &self.state.wire_changes,
                    );
                    self.state.needs_publish = true;
                }
            }
        }
        if let Some(ref mut pair) = self.pair {
            pair.sync_metrics(&mut self.state.metrics);
            Ok(pair.has_pending_frame())
        } else {
            Ok(false)
        }
    }

    pub(crate) fn submit_nve2(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        if self.app.is_some() {
            return Err(RuntimeError::InvalidArg(
                "cannot submit NVE2 wire frames while an app is attached",
            ));
        }
        if self.pair.is_some() {
            return Err(RuntimeError::InvalidArg(
                "cannot submit NVE2 wire frames while a pair session is attached",
            ));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        self.state
            .apply_wire_and_publish(&mut self.replica_store, bytes)?;
        self.state.needs_publish = false;
        Ok(())
    }
}

#[inline]
fn layout_rect_to_abi(r: LayoutRect) -> NveLayoutRect {
    NveLayoutRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
