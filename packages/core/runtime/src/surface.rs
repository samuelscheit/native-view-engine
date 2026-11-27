//! Core [`SurfaceRuntime`] facade: owner-thread affinity, panic/unwind invalidation,
//! reader/writer [`SceneLock`] guards, and safe Rust / shared FFI operations.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nve_abi::{
    NodeId as AbiNodeId, NveEventBatchRef, NveRuntimeMetrics, NveRuntimeSceneView, SurfaceId,
};
use nve_components::tables::{Overflow, Scroll, Visual};
use nve_engine::{HotPatchMsg, Viewport};
use nve_layout::DynLayoutEngine;
use nve_store::{ChangeSet, NodeId};

use crate::app::{FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeBuilder, RuntimeError};
use crate::lock::SceneLock;
use crate::state::RuntimeInner;
use crate::wake::{WakeBridge, WakeCallback};

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
///
/// # Threading Contract
/// - **Owner-thread only**: mutating, layout, and app-touching operations (`attach_app`,
///   `detach_app`, `add_tick_hook`, `set_viewport`, `swap_layout_engine`, `submit_nve2`, `tick`,
///   `dispatch_events`, `apply_hot_patch`, `hit_test`, `hit_test_interaction`, `scroll_by`, and
///   FFI `destroy`) MUST execute on the thread that created the runtime. Cross-thread calls fail
///   with [`RuntimeError::InvalidArg`].
/// - **Cross-thread safe**: `acquire_scene` / `scene_acquire`, `scene_release`, `metrics`,
///   `layout_engine_name`, `set_wake_callback`, `clear_wake_callback`, and [`WakeBridge::trigger`].
pub struct SurfaceRuntime {
    pub(crate) surface_id: SurfaceId,
    pub(crate) lock: SceneLock,
    pub(crate) wake: Arc<WakeBridge>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) owner_thread: std::thread::ThreadId,
    pub(crate) poisoned: AtomicBool,
    pub(crate) inner: UnsafeCell<RuntimeInner>,
}

impl Drop for SurfaceRuntime {
    fn drop(&mut self) {
        // Synchronously drain and clear any installed host wake callback before dropping so
        // background wakers can never invoke a freed host context pointer after `destroy`.
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

    #[inline]
    pub(crate) fn check_owner_thread(&self) -> Result<(), RuntimeError> {
        #[cfg(not(target_arch = "wasm32"))]
        if std::thread::current().id() != self.owner_thread {
            return Err(RuntimeError::InvalidArg(
                "mutating runtime operation called from non-owner thread",
            ));
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn check_not_poisoned(&self) -> Result<(), RuntimeError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(RuntimeError::Poisoned);
        }
        Ok(())
    }

    /// Acquires the exclusive write lock, verifies owner-thread affinity and poison state, and
    /// structurally invalidates `published_view` on any panic, error, or store pointer drift.
    pub(crate) fn with_write<T>(
        &self,
        f: impl FnOnce(&mut RuntimeInner) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        let _guard = self.lock.write_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `_guard` grants exclusive access across all threads/FFI callers and releases on unwind.
        let inner = unsafe { &mut *self.inner.get() };

        struct WriteTxnGuard<'a> {
            inner: &'a mut RuntimeInner,
            poisoned: &'a AtomicBool,
            completed: bool,
        }
        impl Drop for WriteTxnGuard<'_> {
            fn drop(&mut self) {
                if !self.completed {
                    self.inner.state.invalidate_published_view();
                    self.poisoned.store(true, Ordering::Release);
                }
            }
        }

        let mut txn = WriteTxnGuard {
            inner,
            poisoned: &self.poisoned,
            completed: false,
        };
        let res = f(txn.inner);
        // Validate every borrowed slab AND every `NveTableView` column pointer/row_count against
        // the active store unconditionally. Non-mutating errors (e.g. `scroll_by(non_scroll)` or
        // `submit_nve2` while app attached) keep the live view intact; any pointer/row drift
        // invalidates `published_view` and schedules republishing so `acquire_scene` never UAFs.
        if !txn
            .inner
            .state
            .published_view_matches_store(txn.inner.active_store())
        {
            txn.inner.state.invalidate_published_view();
            txn.inner.state.needs_publish = true;
        }
        txn.completed = true;
        res
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

    pub fn attach_app(&mut self, app: Box<dyn RuntimeApp>) -> Result<(), RuntimeError> {
        self.attach_app_shared(app)
    }

    pub(crate) fn attach_app_shared(
        &self,
        mut app: Box<dyn RuntimeApp>,
    ) -> Result<(), RuntimeError> {
        let had_pending = self.with_write(|inner| {
            inner.reset_for_app_change();
            let pending = app.install_wake_bridge(Arc::clone(&self.wake));
            inner.app = Some(app);
            inner.state.needs_render = true;
            inner.state.needs_publish = true;
            Ok(pending)
        })?;
        if had_pending {
            self.wake.trigger();
        }
        Ok(())
    }

    pub fn detach_app(&mut self) -> Result<(), RuntimeError> {
        self.detach_app_shared()
    }

    pub(crate) fn detach_app_shared(&self) -> Result<(), RuntimeError> {
        self.with_write(|inner| {
            inner.reset_for_app_change();
            inner.app = None;
            inner.state.needs_render = false;
            inner.state.needs_publish = false;
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
        self.set_viewport_shared(viewport)
    }

    pub(crate) fn set_viewport_shared(&self, viewport: Viewport) -> Result<bool, RuntimeError> {
        if !viewport.width.is_finite()
            || viewport.width <= 0.0
            || !viewport.height.is_finite()
            || viewport.height <= 0.0
            || !viewport.scale_factor.is_finite()
            || viewport.scale_factor <= 0.0
        {
            return Err(RuntimeError::InvalidArg(
                "viewport dimensions and scale_factor must be finite and > 0",
            ));
        }
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
            let (store, state) = inner.split_store_and_state();
            state.invalidate_published_view();
            state.scene.replace_layout_engine(engine);
            state.layout_engine_name = state.scene.layout_engine_name();
            state.frame_id = state.frame_id.wrapping_add(1).max(1);
            let mut full = ChangeSet::default();
            full.created.extend(store.live_nodes());
            let laid_out = state.scene.commit(store, &full);
            state.scene.reconcile_scroll_offsets(store);
            state.publish_scene(store, &full, laid_out);
            Ok(())
        })
    }

    pub fn layout_engine_name(&self) -> Result<&'static str, RuntimeError> {
        self.check_not_poisoned()?;
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Reader lock held for `_guard`'s lifetime; reads cached `&'static str` only
        // without touching `!Sync` `StoreLayoutEngine`.
        Ok(unsafe { &*self.inner.get() }.state.layout_engine_name)
    }

    pub fn submit_nve2(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.submit_nve2_shared(bytes)
    }

    pub(crate) fn submit_nve2_shared(&self, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.with_write(|inner| inner.submit_nve2(bytes))
    }

    pub fn tick(&mut self, dt_s: f32) -> Result<bool, RuntimeError> {
        self.tick_shared(dt_s)
    }

    pub(crate) fn tick_shared(&self, dt_s: f32) -> Result<bool, RuntimeError> {
        let dt = if dt_s.is_finite() && dt_s >= 0.0 {
            dt_s
        } else {
            0.0
        };
        let (changed, follow_up) = self.with_write(|inner| {
            // Clear pending wake ONLY after acquiring the write lock so `SceneBusy` never drops wakes.
            let _ = self.wake.take_pending();
            inner.tick(dt)
        })?;
        if follow_up {
            self.wake.trigger();
        }
        Ok(changed)
    }

    pub fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> Result<bool, RuntimeError> {
        self.dispatch_events_shared(events)
    }

    pub(crate) fn dispatch_events_shared(
        &self,
        events: NveEventBatchRef<'_>,
    ) -> Result<bool, RuntimeError> {
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
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        let _guard = self.lock.hit_read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Owner-thread affinity protects `!Send`/`!Sync` `RuntimeApp::direct_store` and
        // `StoreLayoutEngine`, and `_guard` serializes `hit_scratch` against concurrent writers.
        let inner = unsafe { &*self.inner.get() };
        Ok(inner.state.scene.hit_test(inner.active_store(), x, y))
    }

    pub fn hit_test_interaction(&self, x: f32, y: f32) -> Result<Option<AbiNodeId>, RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        let _guard = self.lock.hit_read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Owner-thread affinity protects `!Send`/`!Sync` `RuntimeApp` & layout engine;
        // `_guard` serializes `hit_scratch` and releases on unwind.
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
        self.scroll_by_shared(node_id, dx, dy)
    }

    pub(crate) fn scroll_by_shared(
        &self,
        node_id: AbiNodeId,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), RuntimeError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(RuntimeError::InvalidArg("scroll deltas must be finite"));
        }
        self.with_write(|inner| {
            let (store, state) = inner.split_store_and_state();
            let id = NodeId(node_id);
            if !store.is_live(id) {
                return Err(RuntimeError::NotFound("scroll target node is not live"));
            }
            let is_scrollable = store.has::<Scroll>(id)
                || state.scene.scroll_offsets().contains(node_id)
                || store.try_row::<Visual>(id).is_some_and(|v| {
                    matches!(v.overflow_x(), Overflow::Scroll | Overflow::Auto)
                        || matches!(v.overflow_y(), Overflow::Scroll | Overflow::Auto)
                });
            if !is_scrollable {
                return Err(RuntimeError::NotFound(
                    "target node is not a scroll container",
                ));
            }
            let (x, y, _changed) = state.scene.apply_scroll_delta(store, id, dx, dy);
            Ok((x, y))
        })
    }

    /// Acquires a zero-copy [`NveRuntimeSceneView`] guarded by the surface's reader lock.
    pub fn acquire_scene(&self) -> Result<SceneReadGuard<'_>, RuntimeError> {
        self.check_not_poisoned()?;
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
        self.check_not_poisoned()?;
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Reader lock held for `_guard`'s lifetime.
        Ok(unsafe { &*self.inner.get() }.state.metrics)
    }
}
