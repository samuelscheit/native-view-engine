//! Tier-2 C-ABI `cdylib` hot-swap loader.
//!
//! Keeps the native host shell running (macOS Skia, iOS Simulator, Android) and reloads only
//! the user's compiled Rust UI crate in ~150-400ms while snapshotting and restoring app state.

use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use nve_abi::{NveAppHandle, NveAppVTable, NveBufferSlice, SurfaceId, NVE_APP_ABI_VERSION};

#[derive(Debug)]
pub enum LoaderError {
    Io(std::io::Error),
    Load(libloading::Error),
    AbiMismatch { expected: u32, actual: u32 },
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LoaderError {}

pub struct HotAppInstance {
    _lib: Library,
    vtable: NveAppVTable,
    handle: NveAppHandle,
    generation: u64,
    loaded_copy: PathBuf,
}

impl HotAppInstance {
    /// Loads a compiled app cdylib by copying it to a unique generation path (so macOS/Linux
    /// `dlopen` never returns a stale cached image for the same path).
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

        // SAFETY: We check the exported ABI version immediately before invoking any vtable fn.
        let lib = unsafe { Library::new(&loaded_copy) }.map_err(LoaderError::Load)?;
        let vtable = unsafe {
            let sym: Symbol<'_, unsafe extern "C" fn() -> NveAppVTable> =
                lib.get(b"nve_app_vtable").map_err(LoaderError::Load)?;
            sym()
        };

        if vtable.abi_version != NVE_APP_ABI_VERSION {
            return Err(LoaderError::AbiMismatch {
                expected: NVE_APP_ABI_VERSION,
                actual: vtable.abi_version,
            });
        }

        let handle = unsafe { (vtable.create)(surface_id) };
        if let Some(state) = prior_state {
            if !state.is_empty() {
                unsafe { (vtable.restore_state)(handle, NveBufferSlice::from_slice(state)) };
            }
        }

        Ok(Self {
            _lib: lib,
            vtable,
            handle,
            generation,
            loaded_copy,
        })
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn render(&mut self, frame_id: u64) -> Vec<u8> {
        unsafe {
            let slice = (self.vtable.render)(self.handle, frame_id);
            slice.as_slice().to_vec()
        }
    }

    pub fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        unsafe { (self.vtable.dispatch_events)(self.handle, NveBufferSlice::from_slice(encoded_events)) != 0 }
    }

    pub fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        unsafe {
            (self.vtable.apply_hot_patch)(
                self.handle,
                NveBufferSlice::from_slice(patch_json.as_bytes()),
            ) != 0
        }
    }

    pub fn snapshot_state(&mut self) -> Vec<u8> {
        unsafe {
            let slice = (self.vtable.snapshot_state)(self.handle);
            slice.as_slice().to_vec()
        }
    }

    pub fn set_props(&mut self, props_bytes: &[u8]) -> bool {
        unsafe { (self.vtable.set_props)(self.handle, NveBufferSlice::from_slice(props_bytes)) != 0 }
    }

    pub fn drain_emitted_events(&mut self) -> Vec<u8> {
        unsafe {
            let slice = (self.vtable.drain_emitted_events)(self.handle);
            slice.as_slice().to_vec()
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
            unsafe { (self.vtable.destroy)(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        let _ = std::fs::remove_file(&self.loaded_copy);
    }
}
