//! Tier-2 modular `nve_get_interface` `cdylib` hot-swap loader.
//!
//! Resolves the single `nve_get_interface` entry point once at load time, caches the small
//! `const` interface tables (`NveSurfaceApi`, plus optional `NveLayoutApi`, `NveComponentApi`,
//! `NveDevApi`), and exposes direct zero-cost calls + state-preserving generation reloads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use libloading::Library;
use nve_abi::{
    NveAppHandle, NveBufferSlice, NveComponentApi, NveDevApi, NveDirectEvent, NveEventBatch,
    NveGestureDisposition, NveGetInterfaceFn, NveHostApi, NveHostCallbacks, NveIntrinsicSize,
    NveLayoutApi, NveMapEntry, NveSizeConstraints, NveSurfaceApi, SurfaceId, NVE_IFACE_COMPONENT,
    NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};

static STAGED_COPY_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum LoaderError {
    Io(std::io::Error),
    Load(libloading::Error),
    MissingSurfaceInterface,
    NullHandle,
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LoaderError {}

/// Canonical RAII dynamic-library loader that stages a unique copy before calling `dlopen`
/// (avoiding OS image caching and Windows DLL locks) and unloads before removing the temp copy.
pub struct StagedDylib {
    lib: Option<Library>,
    get_interface: NveGetInterfaceFn,
    loaded_copy: PathBuf,
}

impl StagedDylib {
    pub fn load(dylib_path: &Path, subdir: &str, label: &str) -> Result<Self, LoaderError> {
        let ext = dylib_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("dylib");
        let stem = dylib_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("nve_app");
        let seq = STAGED_COPY_SEQ.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(subdir);
        std::fs::create_dir_all(&temp_dir).map_err(LoaderError::Io)?;
        let loaded_copy = temp_dir.join(format!(
            "{stem}_{label}_{}_{seq}.{ext}",
            std::process::id()
        ));
        std::fs::copy(dylib_path, &loaded_copy).map_err(LoaderError::Io)?;

        let lib = match unsafe { Library::new(&loaded_copy) } {
            Ok(lib) => lib,
            Err(err) => {
                let _ = std::fs::remove_file(&loaded_copy);
                return Err(LoaderError::Load(err));
            }
        };
        let get_interface: NveGetInterfaceFn = match unsafe {
            lib.get::<NveGetInterfaceFn>(b"nve_get_interface")
        } {
            Ok(sym) => *sym,
            Err(err) => {
                drop(lib);
                let _ = std::fs::remove_file(&loaded_copy);
                return Err(LoaderError::Load(err));
            }
        };
        Ok(Self {
            lib: Some(lib),
            get_interface,
            loaded_copy,
        })
    }

    pub fn get_interface(&self) -> NveGetInterfaceFn {
        self.get_interface
    }

    pub fn loaded_copy_path(&self) -> &Path {
        &self.loaded_copy
    }
}

impl Drop for StagedDylib {
    fn drop(&mut self) {
        // Unload the dynamic library before removing the temporary copy so Windows
        // releases the OS file lock prior to deletion.
        drop(self.lib.take());
        let _ = std::fs::remove_file(&self.loaded_copy);
    }
}

/// Summary returned after a successful transactional [`HotAppInstance::reload`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadOutcome {
    pub previous_generation: u64,
    pub generation: u64,
    pub preserved_state_bytes: usize,
}

pub struct HotAppInstance {
    staged: Option<StagedDylib>,
    surface: NveSurfaceApi,
    layout: Option<NveLayoutApi>,
    component: Option<NveComponentApi>,
    dev: Option<NveDevApi>,
    host: Option<NveHostApi>,
    bound_host: Option<NveHostCallbacks>,
    handle: NveAppHandle,
    surface_id: SurfaceId,
    generation: u64,
}

impl HotAppInstance {
    /// Loads a compiled app `cdylib` via a unique staged filename so `dlopen` never returns
    /// a stale cached image (and Windows never hits DLL lock collisions during hot-swap),
    /// resolves `nve_get_interface`, and optionally restores state.
    pub fn load(
        dylib_path: &Path,
        surface_id: SurfaceId,
        generation: u64,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, LoaderError> {
        let staged = StagedDylib::load(dylib_path, "nve-hotswap", &format!("gen{generation}"))?;
        let get_iface = staged.get_interface();

        let surface_ptr = unsafe { get_iface(NVE_IFACE_SURFACE) as *const NveSurfaceApi };
        if surface_ptr.is_null() {
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
        let host = {
            let p = unsafe { get_iface(NVE_IFACE_HOST) as *const NveHostApi };
            (!p.is_null()).then(|| unsafe { *p })
        };

        let handle = unsafe { (surface.create)(surface_id) };
        if handle.is_null() {
            return Err(LoaderError::NullHandle);
        }
        if let (Some(dev_api), Some(state)) = (dev, prior_state)
            && !state.is_empty()
        {
            unsafe { (dev_api.restore_state)(handle, NveBufferSlice::from_slice(state)) };
        }

        Ok(Self {
            staged: Some(staged),
            surface,
            layout,
            component,
            dev,
            host,
            bound_host: None,
            handle,
            surface_id,
            generation,
        })
    }

    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn loaded_copy_path(&self) -> &Path {
        self.staged
            .as_ref()
            .map(StagedDylib::loaded_copy_path)
            .unwrap_or_else(|| Path::new(""))
    }

    /// Renders a frame and borrows the binary `CommandBuffer` slice directly from the loaded
    /// app module's internal frame buffer (zero heap allocation / zero `.to_vec()` copy per frame).
    ///
    /// The borrowed slice remains valid until the next `&mut self` call on this [`HotAppInstance`].
    pub fn render(&mut self, frame_id: u64) -> &[u8] {
        unsafe { (self.surface.render)(self.handle, frame_id).as_slice() }
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

    /// Drains outbound component events and borrows the encoded slice directly from the loaded
    /// app module's internal buffer without copying.
    pub fn drain_emitted_events(&mut self) -> &[u8] {
        match self.component {
            Some(api) => unsafe { (api.drain_emitted_events)(self.handle).as_slice() },
            None => &[],
        }
    }

    /// Forwards one binary `NVP1` hot-patch frame (as received from `nve dev`) to the module.
    pub fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        match self.dev {
            Some(api) => unsafe {
                (api.apply_hot_patch)(self.handle, NveBufferSlice::from_slice(patch_nvp1)) != 0
            },
            None => false,
        }
    }

    /// Captures serialized app state and borrows the slice directly from the loaded app module's
    /// internal snapshot buffer without copying.
    pub fn snapshot_state(&mut self) -> &[u8] {
        match self.dev {
            Some(api) => unsafe { (api.snapshot_state)(self.handle).as_slice() },
            None => &[],
        }
    }

    pub fn has_host_interface(&self) -> bool {
        self.host.is_some()
    }

    /// Binds (or unbinds when `None`) host capability callbacks via `NVE_IFACE_HOST = 5`.
    /// Returns `true` if the loaded module exposes `NveHostApi`.
    pub fn bind_host(&mut self, callbacks: Option<NveHostCallbacks>) -> bool {
        self.bound_host = callbacks;
        match self.host {
            Some(api) => {
                let ptr = callbacks
                    .as_ref()
                    .map_or(std::ptr::null(), |cb| cb as *const NveHostCallbacks);
                unsafe { (api.bind_host)(self.handle, ptr) };
                true
            }
            None => false,
        }
    }

    /// Dispatches a host-event payload buffer via `NveHostApi::dispatch_host_events`.
    pub fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        match self.host {
            Some(api) => unsafe {
                (api.dispatch_host_events)(self.handle, NveBufferSlice::from_slice(batch)) != 0
            },
            None => false,
        }
    }

    /// Transactionally hot-swaps this live instance to a freshly compiled dylib while preserving
    /// app state.
    ///
    /// Snapshots state from the live app, then stages (under a unique filename to avoid Windows
    /// DLL lock collisions), loads, validates (`nve_get_interface` + `NVE_IFACE_SURFACE`),
    /// instantiates, and restores state on the candidate dylib BEFORE swapping out and unloading
    /// the existing instance. If any step fails, `self` stays 100% alive and untouched.
    pub fn reload(&mut self, new_dylib_path: &Path) -> Result<ReloadOutcome, LoaderError> {
        let surface_id = self.surface_id;
        let previous_generation = self.generation;
        let next_gen = previous_generation + 1;
        let bound_host = self.bound_host;

        let (mut candidate, preserved_state_bytes) = {
            let saved = self.snapshot_state();
            let len = saved.len();
            let prior = (!saved.is_empty()).then_some(saved);
            (Self::load(new_dylib_path, surface_id, next_gen, prior)?, len)
        };
        if let Some(cb) = bound_host {
            candidate.bind_host(Some(cb));
        }
        let old = std::mem::replace(self, candidate);
        drop(old);
        Ok(ReloadOutcome {
            previous_generation,
            generation: next_gen,
            preserved_state_bytes,
        })
    }
}

impl Drop for HotAppInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.surface.destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        drop(self.staged.take());
    }
}
