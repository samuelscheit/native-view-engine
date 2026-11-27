//! Core [`SurfaceRuntime`] orchestrating `NodeStore`, `LaidOutScene`, `FrameTickHook`s, and
//! zero-copy [`NveRuntimeSceneView`] publication.

use std::cell::UnsafeCell;
use std::sync::Arc;

use nve_abi::table::NveTableView;
use nve_abi::{
    FrameId, NodeId as AbiNodeId, NveEventBatchRef, NveLayoutRect, NveNodeRow, NveRuntimeMetrics,
    NveRuntimeSceneView, SurfaceId, NVE_RUNTIME_SCHEMA_HASH,
};
use nve_engine::{HotPatchMsg, LayoutRect, Viewport};
use nve_layout::{DynLayoutEngine, LaidOutScene};
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, NodeId, NodeStore};

use crate::app::{AppFrame, FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeBuilder, RuntimeError, Timer};
use crate::lock::SceneLock;
use crate::wake::{WakeBridge, WakeCallback};

// Compile-time proof that `nve_store::NodeRow` and `nve_abi::NveNodeRow` are bit-for-bit identical
// in stride, alignment compatibility, and every field offset (little-endian `[u32; 4]` == `u128`).
const _: () = {
    use core::mem::{align_of, offset_of, size_of};
    assert!(size_of::<nve_store::NodeRow>() == size_of::<NveNodeRow>());
    assert!(align_of::<nve_store::NodeRow>() >= align_of::<NveNodeRow>());
    assert!(offset_of!(nve_store::NodeRow, tables) == offset_of!(NveNodeRow, table_set));
    assert!(offset_of!(nve_store::NodeRow, parent) == offset_of!(NveNodeRow, parent));
    assert!(offset_of!(nve_store::NodeRow, children) == offset_of!(NveNodeRow, child_off));
    assert!(offset_of!(nve_store::NodeRow, generation) == offset_of!(NveNodeRow, generation));
    assert!(offset_of!(nve_store::NodeRow, primary_slot) == offset_of!(NveNodeRow, primary_slot));
    assert!(offset_of!(nve_store::NodeRow, effects) == offset_of!(NveNodeRow, effects));
    assert!(nve_store::LIVE == nve_abi::NVE_NODE_ROW_FLAG_LIVE);
};

/// Disjoint non-store surface state so `app` / `replica_store` and `state` can be borrowed
/// simultaneously with zero `unsafe` pointer gymnastics or `Box` moves.
pub(crate) struct SurfaceState {
    pub(crate) surface_id: SurfaceId,
    pub(crate) frame_id: FrameId,
    pub(crate) wire_decoder: WireDecoder,
    pub(crate) wire_changes: ChangeSet,
    pub(crate) empty_changes: ChangeSet,
    pub(crate) scene: LaidOutScene<DynLayoutEngine>,
    pub(crate) tick_hooks: Vec<Box<dyn FrameTickHook>>,
    pub(crate) rects: Vec<NveLayoutRect>,
    pub(crate) published_tables: Vec<NveTableView>,
    pub(crate) published_dirty_buf: Vec<u32>,
    pub(crate) dirty_indices: Vec<u32>,
    pub(crate) removed_ids: Vec<u32>,
    pub(crate) published_view: NveRuntimeSceneView,
    pub(crate) metrics: NveRuntimeMetrics,
    pub(crate) needs_render: bool,
    pub(crate) needs_publish: bool,
}

impl SurfaceState {
    /// Synchronizes dense `self.rects`, dirty/removed scratch, and `NveTableView`s directly from
    /// `store` and `self.scene` with zero heap allocations once capacities warm.
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

        self.dirty_indices.clear();
        for &id in &changes.created {
            self.dirty_indices.push(id.index() as u32);
        }
        for &id in &changes.touched {
            self.dirty_indices.push(id.index() as u32);
        }
        for &id in &changes.children {
            self.dirty_indices.push(id.index() as u32);
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
                // `NodeRow` and `NveNodeRow` are locked bit-for-bit identical at compile time.
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

    fn apply_wire_and_publish(
        &mut self,
        replica: &mut NodeStore,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        {
            let mut tx = replica.tx();
            self.wire_decoder
                .apply(&mut tx, bytes)
                .map_err(|e| RuntimeError::WireDecode(e.to_string()))?;
            tx.commit_into(&mut self.wire_changes);
        }
        let t_lay = Timer::start();
        let laid_out = self.scene.commit(replica, &self.wire_changes);
        self.scene.reconcile_scroll_offsets(replica);
        self.metrics.layout_us = t_lay.elapsed_us();
        let changes = std::mem::take(&mut self.wire_changes);
        self.publish_scene(replica, &changes, laid_out);
        self.wire_changes = changes;
        Ok(())
    }
}

pub(crate) struct RuntimeInner {
    pub(crate) app: Option<Box<dyn RuntimeApp>>,
    pub(crate) replica_store: NodeStore,
    pub(crate) state: SurfaceState,
}

impl RuntimeInner {
    #[inline]
    pub(crate) fn active_store(&self) -> &NodeStore {
        if let Some(ref app) = self.app
            && let Some(store) = app.direct_store()
        {
            return store;
        }
        &self.replica_store
    }

    pub(crate) fn run_hooks(&mut self, dt_s: f32) -> bool {
        if self.state.tick_hooks.is_empty() {
            return false;
        }
        let mut any = false;
        if let Some(ref mut app) = self.app
            && let Some(store) = app.direct_store_mut()
        {
            for hook in &mut self.state.tick_hooks {
                any |= hook.tick(store, dt_s);
            }
            return any;
        }
        for hook in &mut self.state.tick_hooks {
            any |= hook.tick(&mut self.replica_store, dt_s);
        }
        any
    }

    pub(crate) fn tick(&mut self, dt_s: f32) -> Result<bool, RuntimeError> {
        let mut changed = false;

        if let Some(ref mut app) = self.app
            && app.poll_async_work()
        {
            self.state.needs_render = true;
        }

        if self.run_hooks(dt_s) {
            self.state.needs_render = true;
            self.state.needs_publish = true;
        }

        if self.state.needs_render
            && let Some(ref mut app) = self.app
        {
            self.state.needs_render = false;
            let next_frame = self.state.frame_id.wrapping_add(1).max(1);
            let t_rec = Timer::start();
            match app.render_frame(next_frame) {
                AppFrame::Clean => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                }
                AppFrame::Direct { store, changes } => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                    self.state.frame_id = next_frame;
                    let t_lay = Timer::start();
                    let laid_out = self.state.scene.commit(store, changes);
                    self.state.scene.reconcile_scroll_offsets(store);
                    self.state.metrics.layout_us = t_lay.elapsed_us();
                    self.state.publish_scene(store, changes, laid_out);
                    changed = true;
                }
                AppFrame::Wire(bytes) => {
                    self.state.metrics.reconcile_us = t_rec.elapsed_us();
                    if !bytes.is_empty() {
                        self.state.frame_id = next_frame;
                        self.state
                            .apply_wire_and_publish(&mut self.replica_store, bytes)?;
                        changed = true;
                    }
                }
            }
        }

        if self.state.needs_publish {
            let store = match self.app.as_ref().and_then(|a| a.direct_store()) {
                Some(s) => s,
                None => &self.replica_store,
            };
            let t_lay = Timer::start();
            let laid_out = self.state.scene.recompute_layout(store);
            self.state.scene.reconcile_scroll_offsets(store);
            self.state.metrics.layout_us = t_lay.elapsed_us();
            let empty = std::mem::take(&mut self.state.empty_changes);
            self.state.publish_scene(store, &empty, laid_out);
            self.state.empty_changes = empty;
            changed = true;
        }

        Ok(changed)
    }

    pub(crate) fn submit_nve2(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.state.frame_id = self.state.frame_id.wrapping_add(1).max(1);
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

/// RAII read guard returned by [`SurfaceRuntime::acquire_scene`].
///
/// Guarantees that the zero-copy [`NveRuntimeSceneView`] pointers stay valid and immutable until
/// this guard is dropped.
pub struct SceneReadGuard<'a> {
    runtime: &'a SurfaceRuntime,
    view: NveRuntimeSceneView,
}

impl<'a> SceneReadGuard<'a> {
    #[inline]
    pub fn view(&self) -> &NveRuntimeSceneView {
        &self.view
    }
}

impl Drop for SceneReadGuard<'_> {
    fn drop(&mut self) {
        let _ = self.runtime.lock.release_read();
    }
}

/// Framework-agnostic per-surface host runtime.
pub struct SurfaceRuntime {
    pub(crate) lock: SceneLock,
    pub(crate) wake: Arc<WakeBridge>,
    pub(crate) inner: UnsafeCell<RuntimeInner>,
}

impl Drop for SurfaceRuntime {
    fn drop(&mut self) {
        // Clear any installed C/host wake callback before dropping so background wakers can never
        // invoke a freed host context pointer after `destroy`.
        self.wake.clear_sink();
    }
}

impl SurfaceRuntime {
    pub fn new(surface_id: SurfaceId, viewport: Viewport) -> Self {
        RuntimeBuilder::new()
            .surface_id(surface_id)
            .viewport(viewport)
            .build()
    }

    fn with_write<T>(
        &mut self,
        f: impl FnOnce(&mut RuntimeInner) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let _guard = self.lock.write_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `_guard` grants exclusive access and releases even on unwind.
        f(unsafe { &mut *self.inner.get() })
    }

    pub(crate) fn with_write_shared<T>(
        &self,
        f: impl FnOnce(&mut RuntimeInner) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let _guard = self.lock.write_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `_guard` grants exclusive access across all threads/FFI callers even on unwind.
        f(unsafe { &mut *self.inner.get() })
    }

    #[inline]
    pub fn wake_bridge(&self) -> &Arc<WakeBridge> {
        &self.wake
    }

    pub fn set_wake_callback(&self, cb: WakeCallback) {
        self.wake.set_sink(cb);
    }

    pub fn clear_wake_callback(&self) {
        self.wake.clear_sink();
    }

    pub fn attach_app(&mut self, mut app: Box<dyn RuntimeApp>) -> Result<(), RuntimeError> {
        let wake_ref = Arc::clone(&self.wake);
        app.install_wake(Arc::new(move || wake_ref.trigger()));
        self.with_write(|inner| {
            inner.app = Some(app);
            inner.state.needs_render = true;
            inner.state.needs_publish = true;
            Ok(())
        })
    }

    pub fn add_tick_hook(&mut self, hook: Box<dyn FrameTickHook>) -> Result<(), RuntimeError> {
        self.with_write(|inner| {
            inner.state.tick_hooks.push(hook);
            Ok(())
        })
    }

    pub fn set_viewport(&mut self, viewport: Viewport) -> Result<bool, RuntimeError> {
        self.with_write(|inner| {
            let changed = inner.state.scene.set_viewport(viewport);
            if changed {
                inner.state.needs_publish = true;
            }
            Ok(changed)
        })
    }

    pub fn swap_layout_engine(&mut self, engine: DynLayoutEngine) -> Result<(), RuntimeError> {
        self.with_write(|inner| {
            inner.state.scene.replace_layout_engine(engine);
            let store = match inner.app.as_ref().and_then(|a| a.direct_store()) {
                Some(s) => s,
                None => &inner.replica_store,
            };
            let mut full = ChangeSet::default();
            full.created.extend(store.live_nodes());
            let laid_out = inner.state.scene.commit(store, &full);
            inner.state.publish_scene(store, &full, laid_out);
            Ok(())
        })
    }

    pub fn layout_engine_name(&self) -> Result<&'static str, RuntimeError> {
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Reader lock held for `_guard`'s lifetime.
        Ok(unsafe { &*self.inner.get() }.state.scene.layout_engine_name())
    }

    pub fn submit_nve2(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.with_write(|inner| inner.submit_nve2(bytes))
    }

    pub fn tick(&mut self, dt_s: f32) -> Result<bool, RuntimeError> {
        let _ = self.wake.take_pending();
        self.with_write(|inner| inner.tick(dt_s))
    }

    pub fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> Result<bool, RuntimeError> {
        self.with_write(|inner| {
            let changed = inner
                .app
                .as_mut()
                .is_some_and(|app| app.dispatch_events(events));
            if changed {
                inner.state.needs_render = true;
            }
            Ok(changed)
        })
    }

    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, RuntimeError> {
        self.with_write(|inner| {
            let changed = inner
                .app
                .as_mut()
                .is_some_and(|app| app.apply_hot_patch(patch));
            if changed {
                inner.state.needs_render = true;
            }
            Ok(changed)
        })
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Result<Option<AbiNodeId>, RuntimeError> {
        let _guard = self.lock.hit_read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `_guard` guarantees no writer and no other `hit_test` accesses
        // `LaidOutScene::hit_scratch` concurrently (safe alongside `scene_acquire` readers).
        let inner = unsafe { &*self.inner.get() };
        Ok(inner.state.scene.hit_test(inner.active_store(), x, y))
    }

    pub fn hit_test_interaction(&self, x: f32, y: f32) -> Result<Option<AbiNodeId>, RuntimeError> {
        let _guard = self.lock.hit_read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `_guard` serializes `hit_scratch` against concurrent callers and releases on unwind.
        let inner = unsafe { &*self.inner.get() };
        Ok(inner
            .state
            .scene
            .hit_test_interaction(inner.active_store(), x, y))
    }

    pub fn scroll_by(
        &mut self,
        node_id: AbiNodeId,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), RuntimeError> {
        self.with_write(|inner| {
            let store = match inner.app.as_ref().and_then(|a| a.direct_store()) {
                Some(s) => s,
                None => &inner.replica_store,
            };
            let (x, y, _changed) = inner
                .state
                .scene
                .apply_scroll_delta(store, NodeId(node_id), dx, dy);
            Ok((x, y))
        })
    }

    /// Acquires a zero-copy [`NveRuntimeSceneView`] guarded by the surface's reader lock.
    pub fn acquire_scene(&self) -> Result<SceneReadGuard<'_>, RuntimeError> {
        if !self.lock.try_acquire_read() {
            return Err(RuntimeError::SceneBusy);
        }
        // SAFETY: Reader lock held until `SceneReadGuard` drops.
        let view = unsafe { &*self.inner.get() }.state.published_view;
        Ok(SceneReadGuard {
            runtime: self,
            view,
        })
    }

    pub fn metrics(&self) -> Result<NveRuntimeMetrics, RuntimeError> {
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Reader lock held for `_guard`'s lifetime.
        Ok(unsafe { &*self.inner.get() }.state.metrics)
    }
}
