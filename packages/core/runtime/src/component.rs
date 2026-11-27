//! Embedded-component, dev-loop, text-measurement, and resource operations of [`SurfaceRuntime`]
//! forwarded to the attached [`RuntimeApp`](crate::RuntimeApp) (`NveComponentApi`,
//! `NveLayoutApi`, `NveDevApi` for C ABI apps) and to the active `NodeStore`.

use std::ffi::c_void;
use std::sync::Arc;

use nve_abi::{
    NveBufferSlice, NveEventBatch, NveIntrinsicSize, NvePropsRef, NveSizeConstraints, NveStrSlice,
    NveTextMeasureFn,
};
use nve_engine::HotPatchMsg;
use nve_layout::{HeuristicTextMeasurer, TextMeasurer};
use nve_store::ResourceId;

use crate::builder::RuntimeError;
use crate::surface::SurfaceRuntime;

const NO_APP: RuntimeError = RuntimeError::NotFound("no app attached");

/// Host [`NveTextMeasureFn`] adapted to the layout [`TextMeasurer`] contract.
struct HostTextMeasurer {
    cb: NveTextMeasureFn,
    ctx: *mut c_void,
}

// SAFETY: The ABI contract invokes `cb(ctx, ..)` only on the runtime owner thread during layout;
// the host owns `ctx` and keeps it valid until the measurer is replaced or the runtime destroyed.
unsafe impl Send for HostTextMeasurer {}
// SAFETY: See `Send`: the runtime never invokes the callback concurrently.
unsafe impl Sync for HostTextMeasurer {}

impl TextMeasurer for HostTextMeasurer {
    fn measure(&self, text: &str, font_size: f32, max_width: Option<f32>) -> (f32, f32) {
        self.measure_with_weight(text, font_size, 400, max_width)
    }

    fn measure_with_weight(
        &self,
        text: &str,
        font_size: f32,
        font_weight: u16,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let text = NveStrSlice::from_str(text);
        let max_width = max_width.unwrap_or(f32::INFINITY);
        // SAFETY: `text` borrows a live `&str` for the duration of the call; `ctx` per contract.
        let size = unsafe { (self.cb)(self.ctx, text, font_size, font_weight, max_width) };
        let clean = |v: f32| if v.is_finite() && v > 0.0 { v } else { 0.0 };
        (clean(size.width), clean(size.height))
    }
}

impl SurfaceRuntime {
    /// Pushes embedded-component props into the attached app (`NveComponentApi::set_props`).
    /// Returns `true` when the app requested a re-render (the next `tick` renders it).
    pub fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, RuntimeError> {
        self.set_props_shared(props)
    }

    pub(crate) fn set_props_shared(&self, props: NvePropsRef<'_>) -> Result<bool, RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            let changed = app
                .set_props(props)
                .ok_or(RuntimeError::Unsupported("app exposes no NveComponentApi"))?;
            inner.state.needs_render |= changed;
            Ok(changed)
        })
    }

    /// Outbound component events emitted since the last drain. The batch borrows app memory and
    /// stays valid until the next mutating call on this runtime.
    pub fn drain_emitted_events(&mut self) -> Result<NveEventBatch, RuntimeError> {
        self.drain_emitted_events_shared()
    }

    pub(crate) fn drain_emitted_events_shared(&self) -> Result<NveEventBatch, RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            Ok(NveEventBatch::from_slice(app.drain_emitted_events()))
        })
    }

    /// Intrinsic size of the attached app under `constraints` (`NveLayoutApi::measure`).
    pub fn measure(&mut self, constraints: NveSizeConstraints) -> Result<NveIntrinsicSize, RuntimeError> {
        self.measure_shared(constraints)
    }

    pub(crate) fn measure_shared(
        &self,
        constraints: NveSizeConstraints,
    ) -> Result<NveIntrinsicSize, RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            app.measure(constraints)
                .ok_or(RuntimeError::Unsupported("app exposes no NveLayoutApi"))
        })
    }

    /// Applies a Tier-1 hot-reload patch. Returns `true` if the surface dirtied.
    pub fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, RuntimeError> {
        self.apply_hot_patch_shared(patch)
    }

    pub(crate) fn apply_hot_patch_shared(&self, patch: &HotPatchMsg) -> Result<bool, RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            let changed = app.apply_hot_patch(patch);
            inner.state.needs_render |= changed;
            Ok(changed)
        })
    }

    /// Serialized app state for a Tier-2 hot swap / instant boot. The slice borrows app memory and
    /// stays valid until the next mutating call on this runtime.
    pub fn snapshot_state(&mut self) -> Result<NveBufferSlice, RuntimeError> {
        self.snapshot_state_shared()
    }

    pub(crate) fn snapshot_state_shared(&self) -> Result<NveBufferSlice, RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            Ok(NveBufferSlice::from_slice(app.snapshot_state()))
        })
    }

    /// Restores state captured by [`Self::snapshot_state`] and schedules a re-render.
    pub fn restore_state(&mut self, state: &[u8]) -> Result<(), RuntimeError> {
        self.restore_state_shared(state)
    }

    pub(crate) fn restore_state_shared(&self, state: &[u8]) -> Result<(), RuntimeError> {
        self.with_write(|inner| {
            let app = inner.app.as_mut().ok_or(NO_APP)?;
            if !app.restore_state(state) {
                return Err(RuntimeError::Unsupported("app exposes no NveDevApi"));
            }
            inner.state.needs_render = true;
            Ok(())
        })
    }

    /// Installs a host text measurer used by layout (`None` restores the built-in heuristic) and
    /// forces a relayout on the next `tick`.
    pub fn set_text_measurer(&mut self, measurer: Option<Arc<dyn TextMeasurer>>) -> Result<(), RuntimeError> {
        self.set_text_measurer_shared(measurer)
    }

    pub(crate) fn set_text_measurer_shared(
        &self,
        measurer: Option<Arc<dyn TextMeasurer>>,
    ) -> Result<(), RuntimeError> {
        self.with_write(|inner| {
            let measurer = measurer.unwrap_or_else(|| Arc::new(HeuristicTextMeasurer));
            inner.state.scene.set_text_measurer(measurer);
            inner.state.needs_publish = true;
            Ok(())
        })
    }

    /// Installs a C [`NveTextMeasureFn`] (`None` restores the built-in heuristic).
    ///
    /// # Safety
    /// `ctx` must stay valid for `cb` until the measurer is replaced or the runtime destroyed.
    pub(crate) unsafe fn set_host_text_measurer(
        &self,
        cb: Option<NveTextMeasureFn>,
        ctx: *mut c_void,
    ) -> Result<(), RuntimeError> {
        let measurer = cb.map(|cb| Arc::new(HostTextMeasurer { cb, ctx }) as Arc<dyn TextMeasurer>);
        self.set_text_measurer_shared(measurer)
    }

    /// Appends the wire encoding of resource `id` (e.g. `ImageSrc`, `BoxShadows`) referenced by a
    /// published table `Resource` word to `out`. Cross-thread safe (shared read lock).
    pub fn resource_bytes(&self, id: u32, out: &mut Vec<u8>) -> Result<(), RuntimeError> {
        self.check_not_poisoned()?;
        let _guard = self.lock.read_guard().ok_or(RuntimeError::SceneBusy)?;
        // SAFETY: The shared read lock above excludes writers for the guard's lifetime.
        let inner = unsafe { &*self.inner.get() };
        let resources = inner.active_store().resources();
        if !resources.is_live(ResourceId(id)) || !resources.encode(ResourceId(id), out) {
            return Err(RuntimeError::NotFound("resource is not live or has no codec"));
        }
        Ok(())
    }
}