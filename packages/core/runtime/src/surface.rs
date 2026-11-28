//! Core [`SurfaceRuntime`] facade: owner-thread affinity, panic/unwind invalidation,
//! reader/writer [`SceneLock`] guards, and safe Rust / shared FFI operations.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nve_abi::{Key64, NveRuntimeMetrics, NveSceneView, SurfaceId, ValidatedEvents};
use nve_tables::tables::{Overflow, Scroll, Visual};
use nve_engine::Viewport;
use nve_layout::DynLayoutEngine;
use nve_store::{ChangeSet, NodeId, ResourceId};

use crate::app::{FrameTickHook, RuntimeApp};
use crate::builder::{RuntimeBuilder, RuntimeError};
use crate::lock::SceneLock;
use crate::pair::{PairSession, WIRE_PAIRING_SUPPORTED};
use crate::state::RuntimeInner;
use crate::wake::{WakeBridge, WakeCallback};

/// RAII read guard returned by [`SurfaceRuntime::acquire_scene`].
///
/// Guarantees that the zero-copy [`NveSceneView`] pointers stay valid and immutable until
/// this guard is dropped.
pub struct SceneReadGuard<'a> {
    runtime: &'a SurfaceRuntime,
    view: NveSceneView,
}

impl<'a> SceneReadGuard<'a> {
    #[inline]
    pub fn view(&self) -> &NveSceneView {
        &self.view
    }

    /// Kind and canonical payload bytes of published resource `id` (`None` when stale).
    pub fn resource_payload(&self, id: ResourceId) -> Option<(Key64, &[u8])> {
        // SAFETY: `self` holds a scene reader lock for the returned borrow's lifetime.
        unsafe { self.runtime.published_resource(id) }
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
/// - **Owner-thread only**: mutating, layout, and app/pairing operations (`attach_app`,
///   `detach_app`, `pair_connect`, `pair_disconnect`, `add_tick_hook`, `set_viewport`,
///   `swap_layout_engine`, `set_text_measurer`, `set_props`, `drain_emitted_events`,
///   `snapshot_state`, `restore_state`, `submit_nve2`, `tick`, `dispatch_events`, `apply_hot_patch`,
///   `hit_test`, `hit_test_interaction`, `scroll_by`, and FFI `destroy`) MUST execute on the
///   thread that created the runtime. Cross-thread calls fail with [`RuntimeError::InvalidArg`].
/// - **Cross-thread safe**: `acquire_scene` / `scene_acquire` (+ `resource_payload` while it is
///   held), `scene_release`, `metrics`,
///   `layout_engine_name`, `pair_is_connected`, `set_wake_callback`, `clear_wake_callback`, and
///   [`WakeBridge::trigger`].
pub struct SurfaceRuntime {
    pub(crate) surface_id: SurfaceId,
    pub(crate) lock: SceneLock,
    pub(crate) wake: Arc<WakeBridge>,
    pub(crate) pair_connected: Arc<AtomicBool>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) owner_thread: std::thread::ThreadId,
    pub(crate) poisoned: AtomicBool,
    pub(crate) inner: UnsafeCell<RuntimeInner>,
}

impl Drop for SurfaceRuntime {
    fn drop(&mut self) {
        // Synchronously drain and clear any installed host wake callback before dropping so
        // background wakers and any embedded pairing worker can never invoke a freed host context.
        self.wake.clear_sink();
        self.pair_connected.store(false, Ordering::Release);
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
        if !txn.inner.published_view_is_current() {
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
        let (had_pending, old_pair, old_app) = self.with_write(|inner| {
            let old_pair = inner.pair.take();
            if old_pair.is_some() {
                inner.state.metrics.pair_connected = 0;
            }
            inner.reset_for_app_change();
            let pending = app.install_wake_bridge(Arc::clone(&self.wake));
            let old_app = inner.app.replace(app);
            inner.state.needs_render = true;
            inner.state.needs_publish = true;
            Ok((pending, old_pair, old_app))
        })?;
        // The replaced app (and the module it owns) is destroyed outside the surface lock: its
        // `destroy` may block or re-enter (wake sinks), and its code unloads with it.
        drop(old_app);
        // Shut down any replaced pairing worker OUTSIDE the surface write lock to avoid deadlocks.
        if let Some(mut pair) = old_pair {
            pair.shutdown();
            self.pair_connected.store(false, Ordering::Release);
            let _ = self.wake.take_pending();
        }
        if had_pending {
            self.wake.trigger();
        }
        Ok(())
    }

    /// Connects this surface as a thin client to a live `nve pair` / `nve-wire` server,
    /// replacing any local app or previous pairing.
    ///
    /// A local app is swapped for the pairing in one write (replica reset, app dropped, session
    /// installed); a failed connect leaves it running. Fails with [`RuntimeError::InvalidArg`]
    /// when the URI/token/cert pin is invalid, or [`RuntimeError::Unsupported`] when built
    /// without `feature = "wire"` / on wasm32.
    pub fn pair_connect(&mut self, uri: &str, token: &str) -> Result<(), RuntimeError> {
        self.pair_connect_shared(uri, token)
    }

    pub(crate) fn pair_connect_shared(&self, uri: &str, token: &str) -> Result<(), RuntimeError> {
        if !WIRE_PAIRING_SUPPORTED {
            self.check_not_poisoned()?;
            self.check_owner_thread()?;
            return Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ));
        }
        // Validate URI/token/cert pin up front BEFORE mutating or disconnecting any live session.
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        let mut validated = Some(PairSession::validate_and_parse(uri, token)?);
        let (old_pair, old_app) = self.with_write(|inner| {
            if let Some(existing) = inner.pair.take() {
                inner.reset_for_app_change();
                inner.state.needs_render = false;
                inner.state.needs_publish = false;
                inner.state.metrics.pair_connected = 0;
                return Ok((Some(existing), None));
            }
            // Fast path: no prior pair running (a local app at most). Spawn BEFORE touching the
            // app so a failed spawn leaves it running; then swap it out in this same write.
            let Some(parsed) = validated.take() else {
                return Err(RuntimeError::InvalidArg("invalid pairing URI"));
            };
            self.pair_connected.store(false, Ordering::Release);
            let _ = self.wake.take_pending();
            let session = PairSession::spawn_parsed(
                parsed,
                Arc::clone(&self.wake),
                Arc::clone(&self.pair_connected),
            )?;
            let old_app = inner.app.take();
            inner.reset_for_app_change();
            inner.state.needs_render = false;
            inner.state.needs_publish = false;
            inner.state.metrics.pair_connected = 0;
            inner.pair = Some(session);
            Ok((None, old_app))
        })?;
        // The replaced app (and any module it owns) is destroyed outside the surface lock.
        drop(old_app);
        if let Some(mut old) = old_pair {
            // Join the old worker completely OUTSIDE the write lock before reusing `pair_connected`
            // and clearing the wake latch so it can never clobber the replacement session.
            old.shutdown();
            self.pair_connected.store(false, Ordering::Release);
            let _ = self.wake.take_pending();
            let Some(parsed) = validated.take() else {
                return Err(RuntimeError::InvalidArg("invalid pairing URI"));
            };
            // The owner thread is the only mutator, so no app can have been attached meanwhile.
            self.with_write(|inner| {
                let session = PairSession::spawn_parsed(
                    parsed,
                    Arc::clone(&self.wake),
                    Arc::clone(&self.pair_connected),
                )?;
                inner.pair = Some(session);
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Disconnects any active embedded `nve-wire` pairing session and invalidates its scene view.
    ///
    /// Idempotent when already disconnected; returns [`RuntimeError::Unsupported`] when built
    /// without `feature = "wire"` or on `wasm32`.
    pub fn pair_disconnect(&mut self) -> Result<(), RuntimeError> {
        self.pair_disconnect_shared()
    }

    pub(crate) fn pair_disconnect_shared(&self) -> Result<(), RuntimeError> {
        if !WIRE_PAIRING_SUPPORTED {
            self.check_not_poisoned()?;
            self.check_owner_thread()?;
            return Err(RuntimeError::Unsupported(
                "nve-runtime built without wire pairing support",
            ));
        }
        let old_pair = self.with_write(|inner| {
            let old_pair = inner.pair.take();
            inner.state.metrics.pair_connected = 0;
            if old_pair.is_some() {
                inner.reset_for_app_change();
                inner.state.needs_render = false;
                inner.state.needs_publish = false;
            }
            Ok(old_pair)
        })?;
        if let Some(mut pair) = old_pair {
            pair.shutdown();
            self.pair_connected.store(false, Ordering::Release);
            // Clear any stale pending wake left by the stopped session so a future `pair_connect`
            // does not have its initial frame wake suppressed by the coalesced wake latch.
            let _ = self.wake.take_pending();
        }
        Ok(())
    }

    /// Returns whether an embedded pairing session is currently connected (cross-thread safe).
    #[inline]
    pub fn pair_is_connected(&self) -> bool {
        if self.poisoned.load(Ordering::Acquire) {
            return false;
        }
        self.pair_connected.load(Ordering::Acquire)
    }

    pub fn detach_app(&mut self) -> Result<(), RuntimeError> {
        self.detach_app_shared()
    }

    pub(crate) fn detach_app_shared(&self) -> Result<(), RuntimeError> {
        let old_app = self.with_write(|inner| {
            // Only reset when a local app was actually attached so `attach_app_interface(h, NULL)`
            // while paired never wipes the live paired replica.
            let old_app = inner.app.take();
            if old_app.is_some() {
                inner.reset_for_app_change();
                inner.state.needs_render = false;
                inner.state.needs_publish = false;
            }
            Ok(old_app)
        })?;
        drop(old_app);
        Ok(())
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
            let (store, resources, state) = inner.split_store_and_state();
            state.invalidate_published_view();
            state.adopt_store(store);
            state.scene.replace_layout_engine(engine);
            state.layout_engine_name = state.scene.layout_engine_name();
            state.frame_id = state.frame_id.next();
            let mut full = ChangeSet::default();
            full.created.extend(store.live_nodes());
            let laid_out = state.scene.commit(store, &full);
            state.scene.reconcile_scroll_offsets(store);
            state.publish_scene(store, resources, &full, laid_out);
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

    /// Runs an owner-thread app mutation `f`; when it reports a change, schedules a re-render
    /// and wakes the host so the next `tick` renders it (no mutation outside `tick` stalls).
    pub(crate) fn render_if_changed(
        &self,
        f: impl FnOnce(&mut RuntimeInner) -> Result<bool, RuntimeError>,
    ) -> Result<bool, RuntimeError> {
        let changed = self.with_write(|inner| {
            let changed = f(inner)?;
            inner.state.needs_render |= changed;
            Ok(changed)
        })?;
        if changed {
            self.wake.trigger();
        }
        Ok(changed)
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

    pub fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> Result<bool, RuntimeError> {
        self.dispatch_events_shared(events)
    }

    pub(crate) fn dispatch_events_shared(
        &self,
        events: ValidatedEvents<'_>,
    ) -> Result<bool, RuntimeError> {
        self.render_if_changed(|inner| {
            if let Some(app) = inner.app.as_mut() {
                return Ok(app.dispatch_events(events));
            }
            if let Some(pair) = inner.pair.as_ref() {
                let _ = pair.send_events(events);
                // Local scene has not changed yet; the remote server will push a frame update.
                return Ok(false);
            }
            Ok(false)
        })
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Result<Option<NodeId>, RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        let _guard = self.lock.hit_read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: Owner-thread affinity protects `!Send`/`!Sync` `RuntimeApp::direct_store` and
        // `StoreLayoutEngine`, and `_guard` serializes `hit_scratch` against concurrent writers.
        let inner = unsafe { &*self.inner.get() };
        Ok(inner.state.scene.hit_test(inner.active_store(), x, y))
    }

    pub fn hit_test_interaction(&self, x: f32, y: f32) -> Result<Option<NodeId>, RuntimeError> {
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
        node_id: NodeId,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), RuntimeError> {
        self.scroll_by_shared(node_id, dx, dy)
    }

    pub(crate) fn scroll_by_shared(
        &self,
        node_id: NodeId,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), RuntimeError> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(RuntimeError::InvalidArg("scroll deltas must be finite"));
        }
        self.with_write(|inner| {
            let (store, _, state) = inner.split_store_and_state();
            let id = node_id;
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
            let (x, y, changed) = state.scene.apply_scroll_delta(store, id, dx, dy);
            // The published `scroll_offsets` refresh on the next tick.
            state.needs_publish |= changed;
            Ok((x, y, changed))
        })
        .map(|(x, y, changed)| {
            if changed {
                self.wake.trigger();
            }
            (x, y)
        })
    }

    /// Acquires a zero-copy [`NveSceneView`] guarded by the surface's reader lock.
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

    /// Kind and canonical payload of `id` in the store `published_view` borrows.
    ///
    /// # Safety
    /// The caller holds a scene (or scene-bound internal) reader lock for the borrow's lifetime.
    pub(crate) unsafe fn published_resource(&self, id: ResourceId) -> Option<(Key64, &[u8])> {
        // SAFETY: The caller's reader lock excludes writers, so `inner` and the published
        // `ResourceStore` (set and cleared only under the write lock) are immutable.
        let resources = unsafe { &*self.inner.get() }.published_resources()?;
        Some((resources.kind(id)?, resources.payload_bytes(id)?))
    }

    pub fn metrics(&self) -> Result<NveRuntimeMetrics, RuntimeError> {
        self.check_not_poisoned()?;
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: `read_guard()` prevents concurrent writes to `inner` (so `inner.pair` and
        // `last_rx`/`last_tx` stay immutable) while `overlay_live_metrics` reads `&self` atomics.
        let inner = unsafe { &*self.inner.get() };
        let mut m = inner.state.metrics;
        if let Some(pair) = inner.pair.as_ref() {
            pair.overlay_live_metrics(&mut m);
        } else {
            m.pair_connected = u32::from(self.pair_is_connected());
        }
        Ok(m)
    }
}
