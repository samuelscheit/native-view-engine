//! Adapters bridging in-process [`NveApp`] and C ABI `nve_get_interface` modules to the
//! framework-agnostic [`RuntimeApp`] contract defined in `nve-engine`.

use nve_abi::{
    FrameId, NveAppHandle, NveBufferSlice, NveDevApi, NveDirectEvent, NveEventBatch,
    NveEventBatchRef, NveGetInterfaceFn, NveSurfaceApi, SurfaceId, NVE_IFACE_DEV,
    NVE_IFACE_SURFACE,
};
use nve_engine::{HotPatchMsg, NveApp};

pub use nve_engine::{AppFrame, FrameTickHook, RuntimeApp};

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

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        self.app.apply_hot_patch(patch)
    }
}

/// Adapts a foreign or statically linked C ABI `nve_get_interface` module to [`RuntimeApp`].
pub struct CAbiAppAdapter {
    handle: NveAppHandle,
    surface_api: NveSurfaceApi,
    dev_api: Option<NveDevApi>,
    patch_scratch: Vec<u8>,
}

impl CAbiAppAdapter {
    /// Instantiates an app handle by querying `NVE_IFACE_SURFACE` (required) plus optional
    /// `NVE_IFACE_DEV` from `get_iface`.
    ///
    /// # Safety
    /// `get_iface` must conform to the NVE modular C ABI (`nve_get_interface`) and the backing
    /// binary/module must remain loaded for the entire lifetime of the returned adapter.
    pub unsafe fn from_get_interface(
        get_iface: NveGetInterfaceFn,
        surface_id: SurfaceId,
    ) -> Option<Self> {
        // SAFETY: Caller guarantees `get_iface` follows the NVE modular C ABI contract.
        let surface_ptr = unsafe { get_iface(NVE_IFACE_SURFACE) }.cast::<NveSurfaceApi>();
        if surface_ptr.is_null() {
            return None;
        }
        // SAFETY: Non-null pointer returned for `NVE_IFACE_SURFACE` points to a static `NveSurfaceApi`.
        let surface_api = unsafe { *surface_ptr };

        // SAFETY: Querying optional `NVE_IFACE_DEV`.
        let dev_ptr = unsafe { get_iface(NVE_IFACE_DEV) }.cast::<NveDevApi>();
        let dev_api = if dev_ptr.is_null() {
            None
        } else {
            // SAFETY: Non-null pointer points to a valid static `NveDevApi`.
            Some(unsafe { *dev_ptr })
        };

        // SAFETY: `surface_api.create` returns a freshly allocated opaque app handle or null.
        let handle = unsafe { (surface_api.create)(surface_id) };
        if handle.is_null() {
            return None;
        }
        Some(Self {
            handle,
            surface_api,
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

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        let Some(dev) = self.dev_api else {
            return false;
        };
        self.patch_scratch = patch.to_bytes();
        let slice = NveBufferSlice::from_slice(&self.patch_scratch);
        // SAFETY: `self.handle` is live and `slice` points into `self.patch_scratch`.
        unsafe { (dev.apply_hot_patch)(self.handle, slice) != 0 }
    }
}
