//! Embedding-host hooks on [`SurfaceRuntime`]: native text measurement plus the optional app
//! capabilities (props, outbound events, state snapshot/restore, hot patches).

use std::sync::Arc;

use nve_abi::{NveBufferSlice, NveDirectEvent, NveEventBatch, NvePropsRef};
use nve_engine::{CapabilityUnsupported, HotPatchMsg};
use nve_layout::{HeuristicTextMeasurer, TextMeasurer};

use crate::app::RuntimeApp;
use crate::builder::RuntimeError;
use crate::state::RuntimeInner;
use crate::surface::SurfaceRuntime;

/// Runs `f` on the attached app under the write lock, mapping a missing app or capability to
/// [`RuntimeError::Unsupported`].
fn with_app<T>(
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
        self.wake.trigger();
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
    pub fn drain_emitted_events(&mut self) -> Result<&[NveDirectEvent], RuntimeError> {
        let batch = self.drain_emitted_events_shared()?;
        // SAFETY: The batch borrows app memory that stays untouched while `&mut self` is held.
        Ok(unsafe { batch.try_as_slice() }.unwrap_or(&[]))
    }

    /// FFI form: the batch stays valid until the next owner-thread call into the runtime.
    pub(crate) fn drain_emitted_events_shared(&self) -> Result<NveEventBatch, RuntimeError> {
        self.with_write(|inner| {
            with_app(
                inner,
                "app does not emit events (NVE_IFACE_COMPONENT)",
                |app| app.drain_emitted_events().map(NveEventBatch::from_slice),
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
