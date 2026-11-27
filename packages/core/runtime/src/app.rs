//! Adapters bridging in-process [`NveApp`] and C ABI `nve_get_interface` modules to the
//! framework-agnostic [`RuntimeApp`] contract defined in `nve-engine`.

use nve_abi::{
    FrameId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent,
    NveEventBatch, NveEventBatchRef, NveGetInterfaceFn, NvePropsRef, NveSurfaceApi, SurfaceId,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_SURFACE,
};
use nve_engine::{HotPatchMsg, NveApp};

pub use nve_engine::{AppFrame, CapabilityUnsupported, FrameTickHook, RuntimeApp};

/// Adapts any Rust [`NveApp`] implementation to [`RuntimeApp`].
pub struct NveAppAdapter<A: NveApp> {
    app: A,
}

impl<A: NveApp> NveAppAdapter<A> {
    pub fn new(app: A) -> Self {
        Self { app }
    }

    pub fn create(surface_id: SurfaceId) -> Self {
        Self::new(A::create(surface_id))
    }

    #[inline]
    pub fn inner(&self) -> &A {
        &self.app
    }

    #[inline]
    pub fn inner_mut(&mut self) -> &mut A {
        &mut self.app
    }
}

impl<A: NveApp> RuntimeApp for NveAppAdapter<A> {
    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
        let bytes = self.app.render(frame_id);
        if bytes.is_empty() {
            AppFrame::Clean
        } else {
            AppFrame::Wire(bytes)
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        self.app.dispatch_events(events)
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        Ok(self.app.apply_hot_patch(patch))
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        Ok(self.app.set_props(props))
    }

    fn drain_emitted_events(&mut self) -> Result<&[NveDirectEvent], CapabilityUnsupported> {
        Ok(self.app.drain_emitted_events())
    }

    fn snapshot_state(&mut self) -> Result<&[u8], CapabilityUnsupported> {
        Ok(self.app.snapshot_state())
    }

    fn restore_state(&mut self, state: &[u8]) -> Result<(), CapabilityUnsupported> {
        self.app.restore_state(state);
        Ok(())
    }
}

/// Adapts a foreign or statically linked C ABI `nve_get_interface` module to [`RuntimeApp`].
pub struct CAbiAppAdapter {
    handle: NveAppHandle,
    surface_api: NveSurfaceApi,
    component_api: Option<NveComponentApi>,
    dev_api: Option<NveDevApi>,
    patch_scratch: Vec<u8>,
}

/// Copies the optional interface table `T` that `get_iface(id)` returns (`None` for NULL).
///
/// # Safety
/// `get_iface` must follow the NVE modular C ABI: a non-null result for `id` points to a `T`.
unsafe fn query_interface<T: Copy>(get_iface: NveGetInterfaceFn, id: u32) -> Option<T> {
    // SAFETY: Caller guarantees `get_iface` follows the NVE modular C ABI contract.
    let ptr = unsafe { get_iface(id) }.cast::<T>();
    // SAFETY: A non-null pointer returned for `id` points to a valid static `T`.
    (!ptr.is_null()).then(|| unsafe { *ptr })
}

impl CAbiAppAdapter {
    /// Instantiates an app handle by querying `NVE_IFACE_SURFACE` (required) plus the optional
    /// `NVE_IFACE_COMPONENT` and `NVE_IFACE_DEV` tables from `get_iface`.
    ///
    /// # Safety
    /// `get_iface` must conform to the NVE modular C ABI (`nve_get_interface`) and the backing
    /// binary/module must remain loaded for the entire lifetime of the returned adapter.
    pub unsafe fn from_get_interface(
        get_iface: NveGetInterfaceFn,
        surface_id: SurfaceId,
    ) -> Option<Self> {
        // SAFETY: Caller guarantees `get_iface` follows the NVE modular C ABI.
        let surface_api =
            unsafe { query_interface::<NveSurfaceApi>(get_iface, NVE_IFACE_SURFACE) }?;
        // SAFETY: As above.
        let component_api =
            unsafe { query_interface::<NveComponentApi>(get_iface, NVE_IFACE_COMPONENT) };
        // SAFETY: As above.
        let dev_api = unsafe { query_interface::<NveDevApi>(get_iface, NVE_IFACE_DEV) };

        // SAFETY: `surface_api.create` returns a freshly allocated opaque app handle or null.
        let handle = unsafe { (surface_api.create)(surface_id) };
        if handle.is_null() {
            return None;
        }
        Some(Self {
            handle,
            surface_api,
            component_api,
            dev_api,
            patch_scratch: Vec::new(),
        })
    }
}

impl Drop for CAbiAppAdapter {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `self.handle` was allocated by `surface_api.create` and is destroyed once.
            unsafe { (self.surface_api.destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

impl RuntimeApp for CAbiAppAdapter {
    fn render_frame(&mut self, frame_id: FrameId) -> AppFrame<'_> {
        // SAFETY: `self.handle` is live; returned slice is valid until the next call on `self.handle`.
        let slice = unsafe { (self.surface_api.render)(self.handle, frame_id) };
        // SAFETY: `slice` is guaranteed readable for `slice.len` bytes until the next mutable call.
        let bytes = unsafe { slice.as_slice() };
        if bytes.is_empty() {
            AppFrame::Clean
        } else {
            AppFrame::Wire(bytes)
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        let raw_slice: &[NveDirectEvent] = events.as_slice();
        let batch = NveEventBatch::from_slice(raw_slice);
        // SAFETY: `self.handle` is live and `batch` borrows `raw_slice` for the duration of the call.
        unsafe { (self.surface_api.dispatch_events)(self.handle, batch) != 0 }
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        let dev = self.dev_api.ok_or(CapabilityUnsupported)?;
        self.patch_scratch = patch.to_bytes();
        let slice = NveBufferSlice::from_slice(&self.patch_scratch);
        // SAFETY: `self.handle` is live and `slice` points into `self.patch_scratch`.
        Ok(unsafe { (dev.apply_hot_patch)(self.handle, slice) != 0 })
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        let component = self.component_api.ok_or(CapabilityUnsupported)?;
        let entries = props.entries();
        let count = u32::try_from(entries.len()).map_err(|_| CapabilityUnsupported)?;
        // SAFETY: `self.handle` is live and `entries` stays borrowed for the synchronous call.
        Ok(unsafe { (component.set_props)(self.handle, entries.as_ptr(), count) != 0 })
    }

    fn drain_emitted_events(&mut self) -> Result<&[NveDirectEvent], CapabilityUnsupported> {
        let component = self.component_api.ok_or(CapabilityUnsupported)?;
        // SAFETY: `self.handle` is live; the batch stays valid until the next call on it, which
        // the `&mut self` borrow of the returned slice rules out.
        let batch = unsafe { (component.drain_emitted_events)(self.handle) };
        // SAFETY: A structurally valid batch points to `len` initialized `NveDirectEvent`s.
        Ok(unsafe { batch.try_as_slice() }.unwrap_or(&[]))
    }

    fn snapshot_state(&mut self) -> Result<&[u8], CapabilityUnsupported> {
        let dev = self.dev_api.ok_or(CapabilityUnsupported)?;
        // SAFETY: `self.handle` is live; the slice stays valid until the next call on it.
        let slice = unsafe { (dev.snapshot_state)(self.handle) };
        // SAFETY: A structurally valid slice is readable for `len` bytes.
        Ok(unsafe { slice.try_as_slice() }.unwrap_or(&[]))
    }

    fn restore_state(&mut self, state: &[u8]) -> Result<(), CapabilityUnsupported> {
        let dev = self.dev_api.ok_or(CapabilityUnsupported)?;
        // SAFETY: `self.handle` is live and `state` stays borrowed for the synchronous call.
        unsafe { (dev.restore_state)(self.handle, NveBufferSlice::from_slice(state)) };
        Ok(())
    }
}
