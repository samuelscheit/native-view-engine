use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use nve_abi::{
    FrameId, NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent,
    NveEventBatch, NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi,
    NveMapEntry, NveSizeConstraints, NveSurfaceApi, SurfaceId,
};
#[cfg(not(target_arch = "wasm32"))]
use nve_engine::loader::{LoaderError, StagedDylib};

use super::manifest::{
    resolve_and_validate_tables, PluginCapabilities, PluginError, PluginId, PluginMetadata,
    PluginRequirement, PluginSourceKind,
};

/// Safe RAII wrapper owning one live `nve_get_interface`-compatible plugin instance.
pub struct NvePluginSlot {
    #[cfg(not(target_arch = "wasm32"))]
    staged: Option<StagedDylib>,
    metadata: PluginMetadata,
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    handle: NveAppHandle,
}

impl NvePluginSlot {
    /// Instantiates an in-process plugin from its `nve_get_interface` entry point.
    pub fn from_in_process(
        plugin_id: PluginId,
        name: String,
        surface_id: SurfaceId,
        generation: u64,
        get_interface: NveGetInterfaceFn,
        req: PluginRequirement,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, PluginError> {
        let tables = resolve_and_validate_tables(get_interface, req)?;
        nve_engine::clear_last_ffi_panic();
        let handle = catch_unwind(AssertUnwindSafe(|| unsafe {
            (tables.surface.create)(surface_id)
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::create".into()))?;

        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            if !handle.is_null() {
                let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                    (tables.surface.destroy)(handle);
                }));
                nve_engine::clear_last_ffi_panic();
            }
            return Err(PluginError::PluginPanicked(msg));
        }

        if handle.is_null() {
            return Err(PluginError::NullHandleReturned);
        }

        if let (Some(dev_api), Some(state)) = (tables.dev, prior_state)
            && !state.is_empty()
        {
            nve_engine::clear_last_ffi_panic();
            let restore_res = catch_unwind(AssertUnwindSafe(|| unsafe {
                (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state));
            }));
            let ffi_panic = nve_engine::take_last_ffi_panic();
            if restore_res.is_err() || ffi_panic.is_some() {
                let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                    (tables.surface.destroy)(handle);
                }));
                nve_engine::clear_last_ffi_panic();
                return Err(PluginError::PluginPanicked(
                    ffi_panic.unwrap_or_else(|| "NveDevApi::restore_state".into()),
                ));
            }
        }

        Ok(Self {
            #[cfg(not(target_arch = "wasm32"))]
            staged: None,
            metadata: PluginMetadata {
                plugin_id,
                name,
                surface_id,
                generation,
                capabilities: tables.capabilities,
                source: PluginSourceKind::InProcess,
            },
            surface: tables.surface,
            layout: tables.layout,
            component: tables.component,
            dev: tables.dev,
            handle,
        })
    }

    /// Loads a compiled dynamic library (`cdylib`) using `nve_engine::loader::StagedDylib`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dylib(
        plugin_id: PluginId,
        name: String,
        surface_id: SurfaceId,
        generation: u64,
        dylib_path: &Path,
        req: PluginRequirement,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, PluginError> {
        let staged = StagedDylib::load(
            dylib_path,
            "nve-plugin-host",
            &format!("p{plugin_id}_g{generation}"),
        )
        .map_err(|err| match err {
            LoaderError::Io(io) => PluginError::Io(io.to_string()),
            LoaderError::Load(load) => PluginError::LibraryLoadFailed(load.to_string()),
            other => PluginError::LibraryLoadFailed(other.to_string()),
        })?;
        let mut slot = Self::from_in_process(
            plugin_id,
            name,
            surface_id,
            generation,
            staged.get_interface(),
            req,
            prior_state,
        )?;
        slot.staged = Some(staged);
        slot.metadata.source = PluginSourceKind::DynamicLibrary(dylib_path.to_path_buf());
        Ok(slot)
    }

    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub fn plugin_id(&self) -> PluginId {
        self.metadata.plugin_id
    }

    pub fn surface_id(&self) -> SurfaceId {
        self.metadata.surface_id
    }

    pub fn generation(&self) -> u64 {
        self.metadata.generation
    }

    pub fn capabilities(&self) -> PluginCapabilities {
        self.metadata.capabilities
    }

    /// Renders one frame, borrowing the plugin's binary `CommandBuffer` slice zero-copy.
    pub fn render(&mut self, frame_id: FrameId) -> Result<&[u8], PluginError> {
        nve_engine::clear_last_ffi_panic();
        let slice = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.surface.render)(self.handle, frame_id)
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::render".into()))?;
        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            return Err(PluginError::PluginPanicked(msg));
        }
        Ok(unsafe { slice.as_slice() })
    }

    /// Dispatches an encoded `EventRing` slice into the plugin.
    pub fn dispatch_events(&mut self, encoded_events: &[u8]) -> Result<bool, PluginError> {
        nve_engine::clear_last_ffi_panic();
        let handled = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.surface.dispatch_events)(self.handle, NveBufferSlice::from_slice(encoded_events))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::dispatch_events".into()))?;
        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            return Err(PluginError::PluginPanicked(msg));
        }
        Ok(handled != 0)
    }

    /// Dispatches zero-copy direct FFI events if the plugin supports `dispatch_events_direct`.
    pub fn dispatch_events_direct(&mut self, events: &[NveDirectEvent]) -> Result<bool, PluginError> {
        let Some(f) = self.surface.dispatch_events_direct else {
            return Err(PluginError::UnsupportedCapability("dispatch_events_direct"));
        };
        nve_engine::clear_last_ffi_panic();
        let handled = catch_unwind(AssertUnwindSafe(|| unsafe {
            f(self.handle, NveEventBatch::from_slice(events))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveSurfaceApi::dispatch_events_direct".into()))?;
        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            return Err(PluginError::PluginPanicked(msg));
        }
        Ok(handled != 0)
    }

    /// Queries intrinsic size via `NVE_IFACE_LAYOUT` if exported.
    pub fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        let api = self.layout?;
        nve_engine::clear_last_ffi_panic();
        let res = catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.measure)(self.handle, constraints)
        }))
        .ok()?;
        if nve_engine::take_last_ffi_panic().is_some() {
            return None;
        }
        Some(res)
    }

    /// Queries gesture disposition via `NVE_IFACE_LAYOUT` if exported.
    pub fn hit_test(&mut self, x: f32, y: f32, pointer_phase: u8) -> NveGestureDisposition {
        let Some(api) = self.layout else {
            return NveGestureDisposition::PassThrough;
        };
        nve_engine::clear_last_ffi_panic();
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.hit_test)(self.handle, x, y, pointer_phase)
        })) {
            Ok(code) if nve_engine::take_last_ffi_panic().is_none() => {
                NveGestureDisposition::from_u8(code)
            }
            _ => {
                nve_engine::clear_last_ffi_panic();
                NveGestureDisposition::PassThrough
            }
        }
    }

    /// Updates plugin props via `NVE_IFACE_COMPONENT`.
    pub fn set_props(&mut self, props_bytes: &[u8]) -> Result<bool, PluginError> {
        let Some(api) = self.component else {
            return Err(PluginError::UnsupportedCapability("NVE_IFACE_COMPONENT"));
        };
        nve_engine::clear_last_ffi_panic();
        let changed = catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.set_props)(self.handle, NveBufferSlice::from_slice(props_bytes))
        }))
        .map_err(|_| PluginError::PluginPanicked("NveComponentApi::set_props".into()))?;
        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            return Err(PluginError::PluginPanicked(msg));
        }
        Ok(changed != 0)
    }

    /// Updates plugin props via zero-copy structured `NveMapEntry` array if exported.
    pub fn set_props_direct(&mut self, entries: &[NveMapEntry]) -> Result<bool, PluginError> {
        let Some(f) = self.component.and_then(|c| c.set_props_direct) else {
            return Err(PluginError::UnsupportedCapability("set_props_direct"));
        };
        nve_engine::clear_last_ffi_panic();
        let changed = catch_unwind(AssertUnwindSafe(|| unsafe {
            f(
                self.handle,
                entries.as_ptr(),
                entries.len().min(u32::MAX as usize) as u32,
            )
        }))
        .map_err(|_| PluginError::PluginPanicked("NveComponentApi::set_props_direct".into()))?;
        if let Some(msg) = nve_engine::take_last_ffi_panic() {
            return Err(PluginError::PluginPanicked(msg));
        }
        Ok(changed != 0)
    }

    /// Drains component-emitted outbound events via `NVE_IFACE_COMPONENT` if exported.
    pub fn drain_emitted_events(&mut self) -> &[u8] {
        let Some(api) = self.component else {
            return &[];
        };
        nve_engine::clear_last_ffi_panic();
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.drain_emitted_events)(self.handle)
        })) {
            Ok(slice) if nve_engine::take_last_ffi_panic().is_none() => unsafe { slice.as_slice() },
            _ => {
                nve_engine::clear_last_ffi_panic();
                &[]
            }
        }
    }

    /// Forwards one binary `NVP1` Tier-1 hot-patch frame via `NVE_IFACE_DEV` if exported.
    pub fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        let Some(api) = self.dev else {
            return false;
        };
        nve_engine::clear_last_ffi_panic();
        let res = catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.apply_hot_patch)(self.handle, NveBufferSlice::from_slice(patch_nvp1)) != 0
        }))
        .unwrap_or(false);
        if nve_engine::take_last_ffi_panic().is_some() {
            return false;
        }
        res
    }

    /// Snapshots plugin state via `NVE_IFACE_DEV` if exported.
    pub fn snapshot_state(&mut self) -> Vec<u8> {
        let Some(api) = self.dev else {
            return Vec::new();
        };
        nve_engine::clear_last_ffi_panic();
        match catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.snapshot_state)(self.handle)
        })) {
            Ok(slice) if nve_engine::take_last_ffi_panic().is_none() => {
                unsafe { slice.as_slice().to_vec() }
            }
            _ => {
                nve_engine::clear_last_ffi_panic();
                Vec::new()
            }
        }
    }

    /// Restores plugin state via `NVE_IFACE_DEV` if exported.
    pub fn restore_state(&mut self, state: &[u8]) -> bool {
        let Some(api) = self.dev else {
            return false;
        };
        nve_engine::clear_last_ffi_panic();
        let ok = catch_unwind(AssertUnwindSafe(|| unsafe {
            (api.restore_state)(self.handle, NveBufferSlice::from_slice(state));
        }))
        .is_ok();
        ok && nve_engine::take_last_ffi_panic().is_none()
    }
}

impl Drop for NvePluginSlot {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            nve_engine::clear_last_ffi_panic();
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
                (self.surface.destroy)(self.handle);
            }));
            nve_engine::clear_last_ffi_panic();
            self.handle = std::ptr::null_mut();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            drop(self.staged.take());
        }
    }
}
