//! Internal [`SurfaceState`] and [`RuntimeInner`] owning `LaidOutScene`, reusable `ChangeSet`
//! scratch buffers, `WireDecoder`, and `FrameTickHook`s (publication lives in `publish.rs`).

use nve_abi::table::{effect, NveTableView};
use nve_abi::{
    FrameId, NveHostEventBatchBuilder, NveLayoutRect, NveNodeRow, NveRuntimeMetrics, NveSceneView,
    NveScrollOffset,
};
use nve_engine::LayoutRect;
use nve_layout::{DynLayoutEngine, LaidOutScene};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeStore, RealizationOrder, ResourceStore};

use crate::app::{AppFrame, FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeError, Timer};
use crate::delta::SceneDelta;
use crate::host_router::{OwnerBindingHandle, RuntimeHostRouter};
use crate::pair::{PairInboundEvent, PairSession};

/// Returns `true` when `changes` can affect intrinsic or laid-out surface dimensions.
#[inline]
pub(crate) fn changes_affect_layout(changes: &ChangeSet) -> bool {
    !changes.created.is_empty()
        || !changes.removed.is_empty()
        || !changes.children.is_empty()
        || changes
            .effects
            .iter()
            .any(|&fx| fx & (effect::LAYOUT | effect::MEASURE) != 0)
}

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
    /// `NveSceneView::scene_epoch`: bumped by every scene reset.
    pub(crate) scene_epoch: u32,
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
    /// Published rects and dirty / moved / removed sets.
    pub(crate) delta: SceneDelta,
    pub(crate) published_tables: Vec<NveTableView>,
    pub(crate) published_dirty_buf: Vec<u32>,
    /// Visual scroll shifts published through `NveSceneView::scroll_offsets`.
    pub(crate) published_scroll: Vec<NveScrollOffset>,
    /// Sorted, deduplicated live resource ids (re)defined on the published frame.
    pub(crate) dirty_resources: Vec<u32>,
    pub(crate) published_view: NveSceneView,
    /// `ResourceStore` of the store `published_view` borrows; lets `resource_payload` serve any
    /// `scene_acquire` thread without touching `!Sync` apps.
    pub(crate) published_resources: PublishedResources,
    /// [`NodeStore::instance_id`] the scene, delta and `scene_epoch` were last committed from
    /// (`0` right after a reset): node and resource ids of another store are another namespace,
    /// so committing a different store resets them into a new epoch first.
    pub(crate) committed_store: u64,
    pub(crate) metrics: NveRuntimeMetrics,
    pub(crate) host_batch_builder: NveHostEventBatchBuilder,
    pub(crate) active_host_owner_tag: u64,
    pub(crate) intrinsic_size_dirty: bool,
    pub(crate) needs_render: bool,
    pub(crate) needs_publish: bool,
}

/// Which `ResourceStore` the published view borrows. The replica store lives inline in
/// [`RuntimeInner`] (it moves with `SurfaceRuntime`), so it is resolved at read time instead of
/// being cached as an address; a direct app's store lives behind its `Box<dyn RuntimeApp>`, whose
/// heap address survives runtime moves.
/// Each source carries its store's [`NodeStore::instance_id`], so an in-place store replacement
/// at the same address never compares equal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishedResources {
    /// Invalidated: no resource reads.
    None,
    /// `RuntimeInner::replica_store.resources()`.
    Replica(u64),
    /// The attached app's `direct_store().resources()`.
    App(u64, *const ResourceStore),
}

pub(crate) struct RuntimeInner {
    // `app` MUST be declared before `host_binding` so Rust's field drop order destroys the app
    // before the `OwnerBindingHandle` even if `SurfaceRuntime::drop` ever skips cleanup.
    pub(crate) app: Option<Box<dyn RuntimeApp>>,
    pub(crate) host_binding: Option<OwnerBindingHandle>,
    pub(crate) pair: Option<PairSession>,
    pub(crate) replica_store: NodeStore,
    pub(crate) state: SurfaceState,
}

impl RuntimeInner {
    #[inline]
    pub(crate) fn has_non_empty_scene(&self) -> bool {
        self.active_store().stats().live_nodes > 0 || self.state.metrics.live_nodes > 0
    }

    /// Resets `replica_store`, `wire_decoder`, `scene`, `rects`, and `published_view` when
    /// attaching, replacing, or detaching an app/pair or recovering from wire corruption.
    pub(crate) fn reset_for_app_change(&mut self) {
        Self::reset_replica_and_state(&mut self.replica_store, &mut self.state);
    }

    #[inline]
    pub(crate) fn active_store(&self) -> &NodeStore {
        self.app
            .as_ref()
            .and_then(|a| a.direct_store())
            .unwrap_or(&self.replica_store)
    }

    /// The active store, which `ResourceStore` publishing it borrows, and the disjoint state.
    #[inline]
    pub(crate) fn split_store_and_state(
        &mut self,
    ) -> (&NodeStore, PublishedResources, &mut SurfaceState) {
        match self.app.as_ref().and_then(|a| a.direct_store()) {
            Some(store) => (
                store,
                PublishedResources::App(store.instance_id(), store.resources()),
                &mut self.state,
            ),
            None => (
                &self.replica_store,
                PublishedResources::Replica(self.replica_store.instance_id()),
                &mut self.state,
            ),
        }
    }

    /// `true` while every pointer in `published_view` (and its resource source) still matches
    /// the active store.
    pub(crate) fn published_view_is_current(&mut self) -> bool {
        let (store, resources, state) = self.split_store_and_state();
        state.published_view_matches_store(store, resources)
    }

    /// Resources of the published view; `None` while invalidated.
    #[inline]
    pub(crate) fn published_resources(&self) -> Option<&ResourceStore> {
        match self.state.published_resources {
            PublishedResources::None => None,
            PublishedResources::Replica(_) => Some(self.replica_store.resources()),
            // SAFETY: Set by `publish_scene` from the attached app's boxed store; any app detach,
            // swap, or store move invalidates the view (`published_view_is_current`) first.
            PublishedResources::App(_, p) => Some(unsafe { &*p }),
        }
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
        let had_live_nodes =
            replica_store.stats().live_nodes > 0 || state.metrics.live_nodes > 0;
        if (is_keyframe || frame_num == 1) && state.has_replica_state {
            if had_live_nodes {
                state.intrinsic_size_dirty = true;
            }
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
            if had_live_nodes {
                state.intrinsic_size_dirty = true;
            }
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
        state.reset_scene_epoch();
        state.metrics.live_nodes = 0;
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
        let (store, resources, state) = self.split_store_and_state();
        state.invalidate_published_view();
        state.frame_id = state.frame_id.next();
        state.adopt_store(store);
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
            state.publish_scene(store, resources, &changes, laid_out);
            state.merged_changes = changes;
        } else {
            let empty = std::mem::take(&mut state.empty_changes);
            state.publish_scene(store, resources, &empty, laid_out);
            state.empty_changes = empty;
        }
    }

    /// Runs one host tick. Returns `(changed, needs_follow_up_wake, intrinsic_size_invalidated)`.
    pub(crate) fn tick(
        &mut self,
        dt_s: f32,
        host_router: &RuntimeHostRouter,
    ) -> Result<(bool, bool, bool), RuntimeError> {
        let mut has_changes = false;

        let host_follow_up = if let Some(ref mut app) = self.app {
            let (dirtied, more) = host_router.drain_into_app(
                self.state.active_host_owner_tag,
                &mut self.state.host_batch_builder,
                app.as_mut(),
            );
            if dirtied {
                self.state.needs_render = true;
            }
            more
        } else {
            false
        };

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
            let next_frame = self.state.frame_id.next();
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

        let mut needs_follow_up_wake =
            (self.state.needs_render && self.app.is_some()) || pair_follow_up || host_follow_up;

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
            // A hook that asked for a refresh or wrote this tick (a running animation) needs the
            // next tick too; wake-driven hosts would otherwise stall after one frame. Only mark
            // `needs_publish` when the hook actually mutated the store (zero-change hooks must not
            // spin full scene publications/repaints).
            needs_follow_up_wake |= hook_requested || !state.hook_changes.is_empty();
            if !state.hook_changes.is_empty() {
                Self::record_changes(
                    &mut state.merged_changes,
                    &mut has_changes,
                    &state.hook_changes,
                );
                state.needs_publish = true;
            }
        }

        let layout_invalidated = self.state.intrinsic_size_dirty
            || (has_changes && changes_affect_layout(&self.state.merged_changes));
        self.state.intrinsic_size_dirty = false;

        let changed = if has_changes || self.state.needs_publish {
            self.commit_and_publish(has_changes);
            true
        } else {
            false
        };

        Ok((changed, needs_follow_up_wake, layout_invalidated))
    }

    fn reset_pair_replica(&mut self, has_changes: &mut bool) {
        if self.replica_store.stats().live_nodes > 0 || self.state.metrics.live_nodes > 0 {
            self.state.intrinsic_size_dirty = true;
        }
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

    pub(crate) fn submit_nve2(&mut self, bytes: &[u8]) -> Result<bool, RuntimeError> {
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
            return Ok(false);
        }
        let mut has_changes = false;
        self.apply_wire_bytes(bytes, false, true, &mut has_changes)?;
        let layout_invalidated = self.state.intrinsic_size_dirty
            || (has_changes && changes_affect_layout(&self.state.merged_changes));
        self.state.intrinsic_size_dirty = false;
        self.commit_and_publish(has_changes);
        Ok(layout_invalidated)
    }

    /// Atomically decodes a pre-validated Frame-0 `NVE2` keyframe payload into a staging
    /// `NodeStore`, verifies `expected_nodes`, and ONLY swaps/publishes on success so any failure
    /// preserves the prior live scene intact.
    pub(crate) fn boot_validated_frame0(
        &mut self,
        payload: &[u8],
        expected_nodes: usize,
    ) -> Result<(), RuntimeError> {
        if self.app.is_some() {
            return Err(RuntimeError::InvalidArg(
                "cannot boot frame0 while an app is attached",
            ));
        }
        if self.pair.is_some() {
            return Err(RuntimeError::InvalidArg(
                "cannot boot frame0 while a pair session is attached",
            ));
        }
        let mut staging = NodeStore::replica();
        staging.set_realization_order(self.state.realization_order.clone());
        let mut decoder = WireDecoder::default();
        let mut changes = ChangeSet::default();
        let decode_res = {
            let mut tx = staging.tx();
            let res = decoder.apply(&mut tx, payload);
            if res.is_ok() {
                tx.commit_into(&mut changes);
            }
            res
        };
        if decode_res.is_err() {
            return Err(RuntimeError::NotFound("frame0.nve2 NVE2 decode failed"));
        }
        if staging.stats().live_nodes != expected_nodes {
            return Err(RuntimeError::NotFound("frame0.nve2 node_count mismatch"));
        }
        self.state.reset_scene_epoch();
        self.replica_store = staging;
        self.state.wire_decoder = decoder;
        self.state.has_replica_state = true;
        self.state.merged_changes.copy_from(&changes);
        self.state.needs_publish = true;
        self.commit_and_publish(true);
        Ok(())
    }
}

#[inline]
pub(crate) fn layout_rect_to_abi(r: LayoutRect) -> NveLayoutRect {
    NveLayoutRect {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}
