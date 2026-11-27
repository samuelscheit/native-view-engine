//! Internal [`SurfaceState`] and [`RuntimeInner`] owning `LaidOutScene`, reusable `ChangeSet`
//! scratch buffers, `WireDecoder`, `FrameTickHook`s, and zero-copy [`NveRuntimeSceneView`]
//! publication.

use nve_abi::table::NveTableView;
use nve_abi::{
    FrameId, NveLayoutRect, NveNodeRow, NveRuntimeMetrics, NveRuntimeSceneView,
};
use nve_layout::{DynLayoutEngine, LaidOutScene};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore, RealizationOrder};

use crate::app::{AppFrame, FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeError, Timer};
use crate::pair::{PairInboundEvent, PairSession};
use crate::resources::ResourcePublisher;

mod publish;

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

/// Upper bound on paired inbound wire events drained in a single `tick()` to prevent UI starvation.
const MAX_PAIR_FRAMES_PER_TICK: usize = 64;

/// Disjoint non-store surface state so `app` / `replica_store` and `state` can be borrowed
/// simultaneously with zero `unsafe` pointer gymnastics or `Box` moves.
pub(crate) struct SurfaceState {
    pub(crate) frame_id: FrameId,
    /// Cached engine name so `layout_engine_name()` never touches `&StoreLayoutEngine` cross-thread.
    pub(crate) layout_engine_name: &'static str,
    pub(crate) realization_order: RealizationOrder,
    pub(crate) wire_decoder: WireDecoder,
    pub(crate) has_replica_state: bool,
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
    pub(crate) resources: ResourcePublisher,
    pub(crate) published_view: NveRuntimeSceneView,
    pub(crate) metrics: NveRuntimeMetrics,
    pub(crate) needs_render: bool,
    pub(crate) needs_publish: bool,
}

pub(crate) struct RuntimeInner {
    pub(crate) app: Option<Box<dyn RuntimeApp>>,
    pub(crate) pair: Option<PairSession>,
    pub(crate) replica_store: NodeStore,
    pub(crate) state: SurfaceState,
}

impl RuntimeInner {
    /// Resets `replica_store`, `wire_decoder`, `scene`, `rects`, and `published_view` when
    /// attaching, replacing, or detaching an app/pair or recovering from wire corruption.
    pub(crate) fn reset_for_app_change(&mut self) {
        Self::reset_replica_and_state(&mut self.replica_store, &mut self.state);
    }

    /// Lays out and republishes every live node on a new frame (layout engine or measurer swap).
    pub(crate) fn relayout_all(&mut self) {
        let (store, state) = self.split_store_and_state();
        state.invalidate_published_view();
        state.frame_id = state.frame_id.wrapping_add(1).max(1);
        let mut full = ChangeSet::default();
        full.created.extend(store.live_nodes());
        let laid_out = state.scene.commit(store, &full);
        state.scene.reconcile_scroll_offsets(store);
        state.publish_scene(store, &full, laid_out);
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

    fn apply_wire_into(
        replica_store: &mut NodeStore,
        state: &mut SurfaceState,
        bytes: &[u8],
        is_keyframe: bool,
        count_wire_bytes: bool,
        has_changes: &mut bool,
    ) -> Result<(), RuntimeError> {
        let (frame_num, _ops) = WireDecoder::validate_header(bytes)
            .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
        if (is_keyframe || frame_num == 1) && state.has_replica_state {
            Self::reset_replica_and_state(replica_store, state);
            state.merged_changes.copy_from(&state.empty_changes);
            *has_changes = false;
        } else {
            state.invalidate_published_view();
        }
        let apply_res = {
            let mut tx = replica_store.tx();
            let res = state.wire_decoder.apply(&mut tx, bytes);
            if res.is_ok() {
                tx.commit_into(&mut state.wire_changes);
            }
            res
        };
        if let Err(err) = apply_res {
            // Discard mid-frame partial mutations immediately so `replica_store` and
            // `LaidOutScene` stay synchronized across `submit_nve2`, `AppFrame::Wire`, and pair.
            Self::reset_replica_and_state(replica_store, state);
            state.merged_changes.copy_from(&state.empty_changes);
            *has_changes = false;
            state.needs_publish = true;
            return Err(RuntimeError::WireDecode(err.to_string()));
        }
        state.has_replica_state = true;
        if count_wire_bytes {
            state.metrics.wire_bytes_rx = state
                .metrics
                .wire_bytes_rx
                .saturating_add(bytes.len() as u64);
        }
        Self::record_changes(&mut state.merged_changes, has_changes, &state.wire_changes);
        state.needs_publish = true;
        Ok(())
    }

    fn reset_replica_and_state(replica_store: &mut NodeStore, state: &mut SurfaceState) {
        state.invalidate_published_view();
        state.metrics.live_nodes = 0;
        state.rects.clear();
        state.scene.reset();
        state.wire_decoder = WireDecoder::default();
        state.has_replica_state = false;
        *replica_store = NodeStore::replica();
        replica_store.set_realization_order(state.realization_order.clone());
    }

    fn apply_wire_bytes(
        &mut self,
        bytes: &[u8],
        is_keyframe: bool,
        count_wire_bytes: bool,
        has_changes: &mut bool,
    ) -> Result<(), RuntimeError> {
        Self::apply_wire_into(
            &mut self.replica_store,
            &mut self.state,
            bytes,
            is_keyframe,
            count_wire_bytes,
            has_changes,
        )
    }

    fn commit_and_publish(&mut self, has_changes: bool) {
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
                        Self::apply_wire_into(
                            &mut self.replica_store,
                            &mut self.state,
                            bytes,
                            false,
                            true,
                            &mut has_changes,
                        )?;
                    }
                }
            }
            if app.poll_async_work() {
                self.state.needs_render = true;
            }
        }

        let pair_follow_up = if self.pair.is_some() {
            self.drain_pair_frames(&mut has_changes)
        } else {
            false
        };

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
            self.commit_and_publish(has_changes);
            true
        } else {
            false
        };

        Ok((changed, needs_follow_up_wake))
    }

    fn reset_pair_replica(&mut self, has_changes: &mut bool) {
        self.reset_for_app_change();
        self.state
            .merged_changes
            .copy_from(&self.state.empty_changes);
        *has_changes = false;
        self.state.needs_publish = true;
    }

    /// Drains bounded paired wire events, delegating resync rules to `WireResyncState` across
    /// reconnects, queue overflows, and mid-frame/multi-frame decode failures without aborting `tick()`.
    fn drain_pair_frames(&mut self, has_changes: &mut bool) -> bool {
        let mut drained = 0usize;
        while drained < MAX_PAIR_FRAMES_PER_TICK {
            let Some(ev) = self.pair.as_ref().and_then(PairSession::try_recv_event) else {
                break;
            };
            drained += 1;
            match ev {
                PairInboundEvent::SessionStarted {
                    session_id,
                    reconnected,
                } => {
                    if let Some(ref mut pair) = self.pair {
                        pair.on_session_started(
                            session_id,
                            reconnected,
                            self.state.has_replica_state,
                        );
                    }
                }
                PairInboundEvent::Overflowed { session_id } => {
                    self.reset_pair_replica(has_changes);
                    if let Some(ref mut pair) = self.pair {
                        pair.on_overflowed(session_id);
                    }
                }
                PairInboundEvent::Keyframe {
                    session_id,
                    payload,
                } => {
                    if payload.is_empty() {
                        continue;
                    }
                    match self.apply_wire_bytes(&payload, true, false, has_changes) {
                        Ok(()) => {
                            if let Some(ref mut pair) = self.pair {
                                pair.mark_keyframe_applied();
                            }
                        }
                        Err(_) => {
                            self.reset_pair_replica(has_changes);
                            if let Some(ref mut pair) = self.pair {
                                pair.on_decode_error(session_id);
                            }
                        }
                    }
                }
                PairInboundEvent::Delta {
                    session_id,
                    payload,
                } => {
                    if payload.is_empty() {
                        continue;
                    }
                    let is_frame1 = WireDecoder::validate_header(&payload)
                        .is_ok_and(|(frame_num, _)| frame_num == 1);
                    let admit = self
                        .pair
                        .as_ref()
                        .is_none_or(|p| p.should_apply(false, is_frame1));
                    if !admit {
                        continue;
                    }
                    match self.apply_wire_bytes(&payload, false, false, has_changes) {
                        Ok(()) => {
                            if is_frame1 && let Some(ref mut pair) = self.pair {
                                pair.mark_keyframe_applied();
                            }
                        }
                        Err(_) => {
                            self.reset_pair_replica(has_changes);
                            if let Some(ref mut pair) = self.pair {
                                pair.on_decode_error(session_id);
                            }
                        }
                    }
                }
            }
        }
        if let Some(ref mut pair) = self.pair {
            pair.sync_metrics(&mut self.state.metrics);
            pair.has_pending_frame()
        } else {
            false
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
        let mut has_changes = false;
        self.apply_wire_bytes(bytes, false, true, &mut has_changes)?;
        self.commit_and_publish(has_changes);
        Ok(())
    }
}
