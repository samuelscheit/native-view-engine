//! Tier-2 modular `nve_get_interface` `cdylib` hot-swap loader.
//!
//! Resolves the single `nve_get_interface` entry point once at load time, caches the small
//! `const` interface tables (`NveSurfaceApi`, plus optional `NveLayoutApi`, `NveComponentApi`,
//! `NveDevApi`), and exposes direct zero-cost calls + state-preserving generation reloads.

use std::path::{Path, PathBuf};

use libloading::Library;
use nve_abi::{
    NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch,
    NveGestureDisposition, NveGetInterfaceFn, NveIntrinsicSize, NveLayoutApi, NveMapEntry,
    NveSizeConstraints, NveSurfaceApi, SurfaceId, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

#[derive(Debug)]
pub enum LoaderError {
    Io(std::io::Error),
    Load(libloading::Error),
    MissingSurfaceInterface,
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LoaderError {}

pub struct HotAppInstance {
    _lib: Option<Library>,
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    handle: NveAppHandle,
    generation: u64,
    loaded_copy: PathBuf,
}

impl HotAppInstance {
    /// Loads a compiled app `cdylib` via a unique generation filename so `dlopen` never returns
    /// a stale cached image, resolves `nve_get_interface`, and optionally restores state.
    pub fn load(
        dylib_path: &Path,
        surface_id: SurfaceId,
        generation: u64,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, LoaderError> {
        let ext = dylib_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("dylib");
        let stem = dylib_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("nve_app");
        let temp_dir = std::env::temp_dir().join("nve-hotswap");
        std::fs::create_dir_all(&temp_dir).map_err(LoaderError::Io)?;
        let loaded_copy = temp_dir.join(format!(
            "{stem}_gen{generation}_{}.{ext}",
            std::process::id()
        ));
        std::fs::copy(dylib_path, &loaded_copy).map_err(LoaderError::Io)?;

        // SAFETY: We resolve the single `nve_get_interface` entry point and copy the small
        // static const function-pointer tables out while `_lib` keeps the image pinned.
        let lib = match unsafe { Library::new(&loaded_copy) } {
            Ok(lib) => lib,
            Err(err) => {
                let _ = std::fs::remove_file(&loaded_copy);
                return Err(LoaderError::Load(err));
            }
        };
        let get_iface: NveGetInterfaceFn = match unsafe {
            lib.get::<NveGetInterfaceFn>(b"nve_get_interface")
        } {
            Ok(sym) => *sym,
            Err(err) => {
                drop(lib);
                let _ = std::fs::remove_file(&loaded_copy);
                return Err(LoaderError::Load(err));
            }
        };

        let surface_ptr = unsafe { get_iface(NVE_IFACE_SURFACE) as *const NveSurfaceApi };
        if surface_ptr.is_null() {
            drop(lib);
            let _ = std::fs::remove_file(&loaded_copy);
            return Err(LoaderError::MissingSurfaceInterface);
        }
        let surface = unsafe { *surface_ptr };

        let layout = {
            let p = unsafe { get_iface(NVE_IFACE_LAYOUT) as *const NveLayoutApi };
            (!p.is_null()).then(|| unsafe { *p })
        };
        let component = {
            let p = unsafe { get_iface(NVE_IFACE_COMPONENT) as *const NveComponentApi };
            (!p.is_null()).then(|| unsafe { *p })
        };
        let dev = {
            let p = unsafe { get_iface(NVE_IFACE_DEV) as *const NveDevApi };
            (!p.is_null()).then(|| unsafe { *p })
        };

        let handle = unsafe { (surface.create)(surface_id) };
        if let (Some(dev_api), Some(state)) = (dev, prior_state) {
            if !state.is_empty() {
                unsafe { (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state)) };
            }
        }

        Ok(Self {
            _lib: Some(lib),
            surface,
            layout,
            component,
            dev,
            handle,
            generation,
            loaded_copy,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn loaded_copy_path(&self) -> &Path {
        &self.loaded_copy
    }

    pub fn render(&mut self, frame_id: u64) -> Vec<u8> {
        unsafe {
            let slice = (self.surface.render)(self.handle, frame_id);
            slice.as_slice().to_vec()
        }
    }

    pub fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        unsafe {
            (self.surface.dispatch_events)(
                self.handle,
                NveBufferSlice::from_slice(encoded_events),
            ) != 0
        }
    }

    pub fn dispatch_events_direct(&mut self, events: &[NveDirectEvent]) -> bool {
        match self.surface.dispatch_events_direct {
            Some(f) => unsafe { f(self.handle, NveEventBatch::from_slice(events)) != 0 },
            None => false,
        }
    }

    pub fn measure(&mut self, constraints: NveSizeConstraints) -> Option<NveIntrinsicSize> {
        self.layout
            .map(|api| unsafe { (api.measure)(self.handle, constraints) })
    }

    pub fn hit_test(&mut self, x: f32, y: f32, pointer_phase: u8) -> NveGestureDisposition {
        match self.layout {
            Some(api) => {
                NveGestureDisposition::from_u8(unsafe { (api.hit_test)(self.handle, x, y, pointer_phase) })
            }
            None => NveGestureDisposition::PassThrough,
        }
    }

    pub fn set_props(&mut self, props_bytes: &[u8]) -> bool {
        match self.component {
            Some(api) => unsafe {
                (api.set_props)(self.handle, NveBufferSlice::from_slice(props_bytes)) != 0
            },
            None => false,
        }
    }

    pub fn set_props_direct(&mut self, entries: &[NveMapEntry]) -> bool {
        match self.component.and_then(|api| api.set_props_direct) {
            Some(f) => unsafe {
                f(
                    self.handle,
                    entries.as_ptr(),
                    entries.len().min(u32::MAX as usize) as u32,
                ) != 0
            },
            None => false,
        }
    }

    pub fn drain_emitted_events(&mut self) -> Vec<u8> {
        match self.component {
            Some(api) => unsafe { (api.drain_emitted_events)(self.handle).as_slice().to_vec() },
            None => Vec::new(),
        }
    }

    pub fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        match self.dev {
            Some(api) => unsafe {
                (api.apply_hot_patch)(
                    self.handle,
                    NveBufferSlice::from_slice(patch_json.as_bytes()),
                ) != 0
            },
            None => false,
        }
    }

    pub fn snapshot_state(&mut self) -> Vec<u8> {
        match self.dev {
            Some(api) => unsafe { (api.snapshot_state)(self.handle).as_slice().to_vec() },
            None => Vec::new(),
        }
    }

    /// Performs an atomic hot-swap to a freshly compiled dylib while preserving app state.
    pub fn reload(
        mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<Self, LoaderError> {
        let saved = self.snapshot_state();
        let next_gen = self.generation + 1;
        drop(self);
        Self::load(new_dylib_path, surface_id, next_gen, Some(&saved))
    }
}

impl Drop for HotAppInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.surface.destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        // Unload the dynamic library before removing the temporary copy so Windows
        // releases the OS file lock prior to deletion.
        drop(self._lib.take());
        let _ = std::fs::remove_file(&self.loaded_copy);
    }
}
