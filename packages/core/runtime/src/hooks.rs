//! Embedding-host hooks on [`SurfaceRuntime`]: native text measurement, surface sizing
//! (`measure_surface`), intrinsic-size invalidation, `RuntimeHostRouter` ingress, and optional
//! app capabilities (props, outbound events, state snapshot/restore, hot patches).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use nve_abi::{
    HostRequestId, HostServiceId, NveBufferSlice, NveEventBatch, NveHostCallbacks, NveHostStatus,
    NvePropsRef, NveSizeConstraints, NveSurfaceMeasure, ValidatedEvents, NVE_MEASURE_FLAG_DRY_RUN,
    NVE_MEASURE_FLAG_IDEAL,
};
use nve_engine::{CapabilityUnsupported, HotPatchMsg};
use nve_host_services::HostServiceProvider;
use nve_layout::{HeuristicTextMeasurer, TextMeasurer};

use crate::app::RuntimeApp;
use crate::builder::RuntimeError;
use crate::host_router::RuntimeHostRouter;
use crate::state::RuntimeInner;
use crate::surface::SurfaceRuntime;

/// Host callback notified outside runtime locks whenever layout-affecting scene mutations
/// invalidate cached intrinsic surface dimensions.
pub type IntrinsicSizeCallback = Arc<dyn Fn() + Send + Sync>;

/// Synchronized intrinsic-size invalidation slot with lock-free idle fast path and synchronous
/// drain on `clear()` / replacement / `destroy`.
pub(crate) struct IntrinsicSizeNotifier {
    installed: AtomicBool,
    sink: RwLock<Option<IntrinsicSizeCallback>>,
}

impl IntrinsicSizeNotifier {
    pub(crate) fn new() -> Self {
        let sink = RwLock::new(None);
        drop(sink.read());
        Self {
            installed: AtomicBool::new(false),
            sink,
        }
    }

    pub(crate) fn set(&self, cb: Option<IntrinsicSizeCallback>) {
        let mut guard = self.sink.write().unwrap_or_else(|e| e.into_inner());
        self.installed.store(cb.is_some(), Ordering::Release);
        *guard = cb;
    }

    pub(crate) fn clear(&self) {
        self.set(None);
    }

    #[inline]
    pub(crate) fn notify(&self) {
        if !self.installed.load(Ordering::Acquire) {
            return;
        }
        let guard = self.sink.read().unwrap_or_else(|e| e.into_inner());
        if let Some(cb) = guard.as_ref() {
            cb();
        }
    }
}

const VALID_MEASURE_FLAGS: u32 = NVE_MEASURE_FLAG_DRY_RUN | NVE_MEASURE_FLAG_IDEAL;

/// Runs `f` on the attached app under the write lock, mapping a missing app or capability to
/// [`RuntimeError::Unsupported`].
pub(crate) fn with_app<T>(
    inner: &mut RuntimeInner,
    capability: &'static str,
    f: impl FnOnce(&mut dyn RuntimeApp) -> Result<T, CapabilityUnsupported>,
) -> Result<T, RuntimeError> {
    let app = inner
        .app
        .as_deref_mut()
        .ok_or(RuntimeError::Unsupported("no app attached"))?;
    f(app).map_err(|CapabilityUnsupported| RuntimeError::Unsupported(capability))
}

impl SurfaceRuntime {
    /// Installs a native text measurer (`None` restores [`HeuristicTextMeasurer`]), invalidates
    /// cached measurements, and wakes the host so the next `tick` relayouts and republishes.
    pub fn set_text_measurer(
        &mut self,
        measurer: Option<Arc<dyn TextMeasurer>>,
    ) -> Result<(), RuntimeError> {
        self.set_text_measurer_shared(measurer)
    }

    pub(crate) fn set_text_measurer_shared(
        &self,
        measurer: Option<Arc<dyn TextMeasurer>>,
    ) -> Result<(), RuntimeError> {
        let measurer = measurer.unwrap_or_else(|| Arc::new(HeuristicTextMeasurer));
        self.with_write(|inner| {
            inner.state.scene.set_text_measurer(measurer);
            inner.state.needs_publish = true;
            Ok(())
        })?;
        self.intrinsic_size.notify();
        self.wake.trigger();
        Ok(())
    }

    /// Measures the surface under `constraints` and `NVE_MEASURE_FLAG_*` bits without mutating
    /// committed viewport, layout rects, or published scene state.
    ///
    /// Delegates to `NVE_IFACE_LAYOUT = 2` when the attached app provides one, otherwise runs
    /// [`nve_layout::StoreLayoutEngine::measure_root`] against the active `NodeStore`.
    pub fn measure_surface(
        &mut self,
        constraints: NveSizeConstraints,
        flags: u32,
    ) -> Result<NveSurfaceMeasure, RuntimeError> {
        self.measure_surface_shared(constraints, flags)
    }

    pub(crate) fn measure_surface_shared(
        &self,
        constraints: NveSizeConstraints,
        flags: u32,
    ) -> Result<NveSurfaceMeasure, RuntimeError> {
        if flags & !VALID_MEASURE_FLAGS != 0 {
            return Err(RuntimeError::InvalidArg("unknown NVE_MEASURE_FLAG bits"));
        }
        if !constraints.min_w.is_finite()
            || constraints.min_w < 0.0
            || !constraints.min_h.is_finite()
            || constraints.min_h < 0.0
            || constraints.max_w.is_nan()
            || constraints.max_w < constraints.min_w
            || constraints.max_h.is_nan()
            || constraints.max_h < constraints.min_h
        {
            return Err(RuntimeError::InvalidArg("invalid NveSizeConstraints"));
        }
        let ideal = (flags & NVE_MEASURE_FLAG_IDEAL) != 0;
        self.with_write(|inner| {
            if let Some(app) = inner.app.as_mut()
                && let Some(measured) = app.measure_surface(constraints, flags)
            {
                return Ok(measured);
            }
            let (store, _, state) = inner.split_store_and_state();
            Ok(state.scene.measure_root(store, constraints, ideal))
        })
    }

    /// Registers a callback invoked (outside internal locks) whenever layout-affecting changes
    /// invalidate the surface's intrinsic size, or clears it with `None`.
    pub fn set_intrinsic_size_invalidated_callback(
        &self,
        cb: Option<IntrinsicSizeCallback>,
    ) -> Result<(), RuntimeError> {
        self.set_intrinsic_size_invalidated_callback_shared(cb)
    }

    pub(crate) fn set_intrinsic_size_invalidated_callback_shared(
        &self,
        cb: Option<IntrinsicSizeCallback>,
    ) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        self.intrinsic_size.set(cb);
        Ok(())
    }

    /// Borrows this surface's runtime-owned [`RuntimeHostRouter`].
    #[inline]
    pub fn host_router(&self) -> &RuntimeHostRouter {
        &self.host_router
    }

    /// Installs or clears the host's C-ABI [`NveHostCallbacks`] (`NVE_IFACE_HOST = 5`).
    pub fn set_host_callbacks(&mut self, callbacks: Option<NveHostCallbacks>) -> Result<(), RuntimeError> {
        self.set_host_callbacks_shared(callbacks)
    }

    pub(crate) fn set_host_callbacks_shared(
        &self,
        callbacks: Option<NveHostCallbacks>,
    ) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        self.host_router.set_callbacks(callbacks);
        Ok(())
    }

    /// Installs or clears an in-process Rust [`HostServiceProvider`] backing `NVE_IFACE_HOST = 5`.
    pub fn set_host_provider(
        &mut self,
        provider: Option<Arc<dyn HostServiceProvider>>,
    ) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.check_owner_thread()?;
        self.host_router.set_provider(provider);
        Ok(())
    }

    /// Completes an asynchronous `Pending` host request from ANY thread and triggers `WakeBridge`.
    /// Returns `Ok(true)` when resolved, or `Ok(false)` when stale/unknown (safely dropped).
    pub fn complete_host_request(
        &self,
        request_id: HostRequestId,
        status: NveHostStatus,
        payload: &[u8],
    ) -> Result<bool, RuntimeError> {
        self.check_not_poisoned()?;
        Ok(self.host_router.complete_request(request_id, status, payload))
    }

    /// Enqueues an unsolicited host push event from ANY thread and triggers `WakeBridge`.
    pub fn push_host_event(
        &self,
        service_id: HostServiceId,
        event_id: u32,
        payload: &[u8],
    ) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.host_router.push_event(service_id, event_id, payload);
        Ok(())
    }

    /// Enqueues an inbound host channel message from ANY thread and triggers `WakeBridge`.
    pub fn publish_host_channel(
        &self,
        channel_id: u32,
        flags: u32,
        payload: &[u8],
    ) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        self.host_router.publish_channel(channel_id, flags, payload);
        Ok(())
    }

    /// Forwards one Tier-1 hot patch to the app. `Ok(true)` when the app needs a re-render.
    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, RuntimeError> {
        self.apply_hot_patch_shared(patch)
    }

    pub(crate) fn apply_hot_patch_shared(&self, patch: &HotPatchMsg) -> Result<bool, RuntimeError> {
        self.render_if_changed(|inner| {
            with_app(inner, "app does not support hot patches (NVE_IFACE_DEV)", |app| {
                app.apply_hot_patch(patch)
            })
        })
    }

    /// Forwards embedding-host props to the app. `Ok(true)` when the app needs a re-render.
    pub fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, RuntimeError> {
        self.set_props_shared(props)
    }

    pub(crate) fn set_props_shared(&self, props: NvePropsRef<'_>) -> Result<bool, RuntimeError> {
        self.render_if_changed(|inner| {
            with_app(inner, "app does not support props (NVE_IFACE_COMPONENT)", |app| {
                app.set_props(props)
            })
        })
    }

    /// Drains app-emitted outbound events (borrowed until the next call on `self`).
    pub fn drain_emitted_events(&mut self) -> Result<ValidatedEvents<'_>, RuntimeError> {
        let batch = self.drain_emitted_events_shared()?;
        // SAFETY: The batch views app memory that stays untouched while `&mut self` is held.
        Ok(unsafe { ValidatedEvents::from_raw(batch) })
    }

    /// FFI form: the batch stays valid until the next owner-thread call into the runtime.
    pub(crate) fn drain_emitted_events_shared(&self) -> Result<NveEventBatch, RuntimeError> {
        self.with_write(|inner| {
            with_app(
                inner,
                "app does not emit events (NVE_IFACE_COMPONENT)",
                |app| app.drain_emitted_events().map(|events| events.as_raw_batch()),
            )
        })
    }

    /// Serializes app state (borrowed until the next call on `self`).
    pub fn snapshot_state(&mut self) -> Result<&[u8], RuntimeError> {
        let slice = self.snapshot_state_shared()?;
        // SAFETY: The slice borrows app memory that stays untouched while `&mut self` is held.
        Ok(unsafe { slice.try_as_slice() }.unwrap_or(&[]))
    }

    /// FFI form: the slice stays valid until the next owner-thread call into the runtime.
    pub(crate) fn snapshot_state_shared(&self) -> Result<NveBufferSlice, RuntimeError> {
        self.with_write(|inner| {
            with_app(inner, "app does not support state snapshots (NVE_IFACE_DEV)", |app| {
                app.snapshot_state().map(NveBufferSlice::from_slice)
            })
        })
    }

    /// Restores a [`Self::snapshot_state`] payload and wakes the host to re-render.
    pub fn restore_state(&mut self, state: &[u8]) -> Result<(), RuntimeError> {
        self.restore_state_shared(state)
    }

    pub(crate) fn restore_state_shared(&self, state: &[u8]) -> Result<(), RuntimeError> {
        self.render_if_changed(|inner| {
            with_app(inner, "app does not support state restore (NVE_IFACE_DEV)", |app| {
                app.restore_state(state)
            })?;
            Ok(true)
        })?;
        Ok(())
    }
}
