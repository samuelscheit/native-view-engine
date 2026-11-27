//! Adapters bridging in-process [`NveApp`] and C ABI `nve_get_interface` modules to the
//! framework-agnostic [`RuntimeApp`] contract defined in `nve-engine`.

use nve_abi::{
    FrameId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent,
    NveEventBatch, NveEventBatchRef, NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi,
    NvePropsRef, NveSizeConstraints, NveSurfaceApi, SurfaceId, NVE_IFACE_COMPONENT,
    NVE_IFACE_DEV, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
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

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Option<bool> {
        Some(self.app.set_props(props))
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        self.app.drain_emitted_events()
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        Some(self.app.measure(constraints))
    }

    fn snapshot_state(&mut self) -> &[u8] {
        self.app.snapshot_state()
    }

    fn restore_state(&mut self, state: &[u8]) -> bool {
        self.app.restore_state(state);
        true
    }
}

/// Copies the optional static vtable `get_iface(id)` returns, or `None` when the module returns
/// null OR leaves any slot null. Every `NveXxxApi` is a struct of non-nullable Rust fn pointers,
/// so copying a vtable with a null slot would be undefined behaviour (and, through the
/// `Option<T>` niche, silently read as "interface absent"). A C module must fill every slot of an
/// interface it returns (returning 0 / empty for unsupported operations).
///
/// # Safety
/// `get_iface` follows the NVE modular C ABI: a non-null result points to a static `T`, and `T`
/// consists solely of function pointers.
unsafe fn optional_iface<T: Copy>(get_iface: NveGetInterfaceFn, id: u32) -> Option<T> {
    const PTR: usize = core::mem::size_of::<*const ()>();
    // SAFETY: Caller guarantees the `nve_get_interface` contract for `id`.
    let ptr = unsafe { get_iface(id) };
    if ptr.is_null() || !core::mem::size_of::<T>().is_multiple_of(PTR) {
        return None;
    }
    let slots = ptr.cast::<*const ()>();
    for i in 0..core::mem::size_of::<T>() / PTR {
        // SAFETY: `ptr` points to a readable `T` made of `size_of::<T>() / PTR` pointer slots.
        if unsafe { slots.add(i).read() }.is_null() {
            return None;
        }
    }
    // SAFETY: Non-null `ptr` points to a valid static `T` whose every fn-pointer slot is non-null.
    Some(unsafe { *ptr.cast::<T>() })
}

/// Adapts a foreign or statically linked C ABI `nve_get_interface` module to [`RuntimeApp`].
pub struct CAbiAppAdapter {
    handle: NveAppHandle,
    surface_api: NveSurfaceApi,
    layout_api: Option<NveLayoutApi>,
    component_api: Option<NveComponentApi>,
    dev_api: Option<NveDevApi>,
    patch_scratch: Vec<u8>,
}

impl CAbiAppAdapter {
    /// Instantiates an app handle by querying `NVE_IFACE_SURFACE` (required) plus the optional
    /// `NVE_IFACE_LAYOUT`, `NVE_IFACE_COMPONENT`, and `NVE_IFACE_DEV` from `get_iface`.
    ///
    /// # Safety
    /// `get_iface` must conform to the NVE modular C ABI (`nve_get_interface`) and the backing
    /// binary/module must remain loaded for the entire lifetime of the returned adapter.
    pub unsafe fn from_get_interface(
        get_iface: NveGetInterfaceFn,
        surface_id: SurfaceId,
    ) -> Option<Self> {
        // SAFETY: Caller guarantees `get_iface` follows the NVE modular C ABI contract.
        let surface_api = unsafe { optional_iface::<NveSurfaceApi>(get_iface, NVE_IFACE_SURFACE) }?;
        // SAFETY: Same contract for the optional interfaces.
        let (layout_api, component_api, dev_api) = unsafe {
            (
                optional_iface(get_iface, NVE_IFACE_LAYOUT),
                optional_iface(get_iface, NVE_IFACE_COMPONENT),
                optional_iface(get_iface, NVE_IFACE_DEV),
            )
        };
        // SAFETY: `surface_api.create` returns a freshly allocated opaque app handle or null.
        let handle = unsafe { (surface_api.create)(surface_id) };
        if handle.is_null() {
            return None;
        }
        Some(Self {
            handle,
            surface_api,
            layout_api,
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

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> bool {
        let Some(dev) = self.dev_api else {
            return false;
        };
        self.patch_scratch = patch.to_bytes();
        let slice = NveBufferSlice::from_slice(&self.patch_scratch);
        // SAFETY: `self.handle` is live and `slice` points into `self.patch_scratch`.
        unsafe { (dev.apply_hot_patch)(self.handle, slice) != 0 }
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Option<bool> {
        let component = self.component_api?;
        let entries = props.as_slice();
        // SAFETY: `self.handle` is live; `entries` stays borrowed for the duration of the call.
        Some(unsafe { (component.set_props)(self.handle, entries.as_ptr(), entries.len() as u32) } != 0)
    }

    fn drain_emitted_events(&mut self) -> &[NveDirectEvent] {
        let Some(component) = self.component_api else {
            return &[];
        };
        // SAFETY: `self.handle` is live; the batch stays valid until the next call on the handle,
        // which `&mut self` on the returned borrow forbids.
        let batch = unsafe { (component.drain_emitted_events)(self.handle) };
        // SAFETY: A structurally valid batch points at `len` readable `NveDirectEvent`s.
        unsafe { NveEventBatchRef::try_from_raw(batch) }.map_or(&[], |b| b.as_slice())
    }

    fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        let layout = self.layout_api?;
        // SAFETY: `self.handle` is live.
        Some(unsafe { (layout.measure)(self.handle, constraints) })
    }

    fn snapshot_state(&mut self) -> &[u8] {
        let Some(dev) = self.dev_api else {
            return &[];
        };
        // SAFETY: `self.handle` is live; the slice stays valid until the next call on the handle.
        let slice = unsafe { (dev.snapshot_state)(self.handle) };
        // SAFETY: A structurally valid slice is readable for `len` bytes.
        unsafe { slice.try_as_slice() }.unwrap_or(&[])
    }

    fn restore_state(&mut self, state: &[u8]) -> bool {
        let Some(dev) = self.dev_api else {
            return false;
        };
        // SAFETY: `self.handle` is live and `state` is borrowed for the duration of the call.
        unsafe { (dev.restore_state)(self.handle, NveBufferSlice::from_slice(state)) };
        true
    }
}
