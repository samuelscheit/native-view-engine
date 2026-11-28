//! Adapters bridging in-process [`NveApp`] and C ABI `nve_get_interface` modules to the
//! framework-agnostic [`RuntimeApp`] contract defined in `nve-engine`.

use nve_abi::{
    FrameId, InterfaceId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi,
    NveGetInterfaceFn, NvePropsRef, NveSurfaceApi, SurfaceId, ValidatedEvents,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_SURFACE,
};
#[cfg(not(target_arch = "wasm32"))]
use nve_engine::loader::StagedDylib;
use nve_engine::{HotPatchMsg, NveApp};

use crate::builder::RuntimeError;

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

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        self.app.dispatch_events(events)
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        Ok(self.app.apply_hot_patch(patch))
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        Ok(self.app.set_props(props))
    }

    fn drain_emitted_events(&mut self) -> Result<ValidatedEvents<'_>, CapabilityUnsupported> {
        Ok(self.app.drain_emitted_events())
    }

    fn supports_state(&self) -> bool {
        true
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
    /// The staged `cdylib` backing every pointer above; declared last so it unloads only after
    /// `Drop` destroyed the app handle.
    #[cfg(not(target_arch = "wasm32"))]
    module: Option<StagedDylib>,
}

/// Copies the interface table `T` that `get_iface(id)` returns, or `None` when the module returns
/// NULL OR leaves any slot NULL. Every `NveXxxApi` is a struct of non-nullable Rust fn pointers,
/// so copying a vtable with a NULL slot would be undefined behaviour (and, through the
/// `Option<T>` niche, silently read as "interface absent"). A C module must fill every slot of an
/// interface it returns (returning 0 / empty for unsupported operations).
///
/// # Safety
/// `get_iface` follows the NVE modular C ABI: a non-null result points to a static `T`, and `T`
/// consists solely of function pointers.
unsafe fn query_interface<T: Copy>(get_iface: NveGetInterfaceFn, id: InterfaceId) -> Option<T> {
    const PTR: usize = core::mem::size_of::<*const ()>();
    const { assert!(core::mem::size_of::<T>().is_multiple_of(PTR)) };
    // SAFETY: Caller guarantees the `nve_get_interface` contract for `id`.
    let ptr = unsafe { get_iface(id) };
    if ptr.is_null() {
        return None;
    }
    if !ptr.cast::<T>().is_aligned() {
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

impl CAbiAppAdapter {
    /// Instantiates an app handle by querying `NVE_IFACE_SURFACE` (required) plus the optional
    /// `NVE_IFACE_COMPONENT` and `NVE_IFACE_DEV` tables from `get_iface`.
    ///
    /// Fails with [`RuntimeError::Unsupported`] when the surface table is missing or partial, and
    /// [`RuntimeError::InvalidArg`] when `create` returns a NULL app handle.
    ///
    /// # Safety
    /// `get_iface` must conform to the NVE modular C ABI (`nve_get_interface`) and the backing
    /// binary/module must remain loaded for the entire lifetime of the returned adapter.
    pub unsafe fn from_get_interface(
        get_iface: NveGetInterfaceFn,
        surface_id: SurfaceId,
    ) -> Result<Self, RuntimeError> {
        // SAFETY: Caller guarantees `get_iface` follows the NVE modular C ABI.
        let surface_api =
            unsafe { query_interface::<NveSurfaceApi>(get_iface, NVE_IFACE_SURFACE) }.ok_or(
                RuntimeError::Unsupported(
                    "app exports no complete NVE_IFACE_SURFACE (missing table or NULL slot)",
                ),
            )?;
        // SAFETY: As above.
        let component_api =
            unsafe { query_interface::<NveComponentApi>(get_iface, NVE_IFACE_COMPONENT) };
        // SAFETY: As above.
        let dev_api = unsafe { query_interface::<NveDevApi>(get_iface, NVE_IFACE_DEV) };

        // SAFETY: `surface_api.create` returns a freshly allocated opaque app handle or null.
        let handle = unsafe { (surface_api.create)(surface_id) };
        if handle.is_null() {
            return Err(RuntimeError::InvalidArg("app surface create returned a NULL app handle"));
        }
        Ok(Self {
            handle,
            surface_api,
            component_api,
            dev_api,
            patch_scratch: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            module: None,
        })
    }

    /// Instantiates the app of a staged `cdylib` (see [`Self::from_get_interface`]); the adapter
    /// owns the library and unloads it (removing the staged copy) after destroying the app.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_staged_dylib(module: StagedDylib, surface_id: SurfaceId) -> Result<Self, RuntimeError> {
        // SAFETY: `StagedDylib` resolved `nve_get_interface` per the NVE modular C ABI, and the
        // adapter keeps `module` loaded for its whole lifetime.
        let mut adapter = unsafe { Self::from_get_interface(module.get_interface(), surface_id) }?;
        adapter.module = Some(module);
        Ok(adapter)
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

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        // SAFETY: `self.handle` is live and the raw batch borrows `events` for the synchronous call.
        unsafe { (self.surface_api.dispatch_events)(self.handle, events.as_raw_batch()) != 0 }
    }

    fn apply_hot_patch(&mut self, patch: &HotPatchMsg) -> Result<bool, CapabilityUnsupported> {
        let dev = self.dev_api.ok_or(CapabilityUnsupported)?;
        // A patch over the `NVP1` wire limits cannot cross the C ABI at all.
        self.patch_scratch = patch.to_bytes().map_err(|_| CapabilityUnsupported)?;
        let slice = NveBufferSlice::from_slice(&self.patch_scratch);
        // SAFETY: `self.handle` is live and `slice` points into `self.patch_scratch`.
        Ok(unsafe { (dev.apply_hot_patch)(self.handle, slice) != 0 })
    }

    fn set_props(&mut self, props: NvePropsRef<'_>) -> Result<bool, CapabilityUnsupported> {
        let component = self.component_api.ok_or(CapabilityUnsupported)?;
        let entries = props.entries();
        let count = u32::try_from(entries.len()).map_err(|_| CapabilityUnsupported)?;
        let ptr = if entries.is_empty() {
            core::ptr::null()
        } else {
            entries.as_ptr()
        };
        // SAFETY: `self.handle` is live and `entries` stays borrowed for the synchronous call.
        Ok(unsafe { (component.set_props)(self.handle, ptr, count) != 0 })
    }

    fn drain_emitted_events(&mut self) -> Result<ValidatedEvents<'_>, CapabilityUnsupported> {
        let component = self.component_api.ok_or(CapabilityUnsupported)?;
        // SAFETY: `self.handle` is live; the batch stays valid until the next call on it, which
        // the `&mut self` borrow of the returned view rules out.
        let batch = unsafe { (component.drain_emitted_events)(self.handle) };
        // SAFETY: The module's batch points to `event_count` events and `bytes_len` bytes; a
        // malformed batch (bad span, UTF-8 or pointers) validates to an empty view.
        Ok(unsafe { ValidatedEvents::from_raw(batch) })
    }

    fn supports_state(&self) -> bool {
        self.dev_api.is_some()
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
