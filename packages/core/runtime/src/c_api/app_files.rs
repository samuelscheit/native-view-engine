//! App-file entry points: staged `cdylib` app modules and `StateFile` app-state persistence
//! (`UNSUPPORTED` on wasm32, which has neither dynamic libraries nor a filesystem).

use std::path::Path;

use nve_abi::{NveRuntimeHandle, NveStrSlice};

use super::with_runtime;
use crate::builder::RuntimeError;
use crate::surface::SurfaceRuntime;

/// Runs `f` with the UTF-8 path in `path` (`InvalidArg` when not valid UTF-8).
///
/// # Safety
/// `path` is readable for `path.len` bytes when `path.len > 0`.
unsafe fn with_path(
    op: &'static str,
    handle: NveRuntimeHandle,
    path: NveStrSlice,
    f: impl FnOnce(&SurfaceRuntime, &Path) -> Result<(), RuntimeError>,
) -> i32 {
    with_runtime(op, handle, |rt| {
        // SAFETY: Forwarded caller contract (`as_str` validates UTF-8).
        let path = unsafe { path.as_str() }
            .map_err(|_| RuntimeError::InvalidArg("path is not a valid UTF-8 NveStrSlice"))?;
        f(rt, Path::new(path))
    })
}

/// wasm32 has neither dynamic libraries nor a filesystem (`crate::app_files` is native-only).
#[cfg(target_arch = "wasm32")]
impl SurfaceRuntime {
    const NO_FILES: RuntimeError = RuntimeError::Unsupported("app files need a native filesystem");

    fn load_app_module_shared(&self, _: &Path, _: bool) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    fn save_state_file_shared(&self, _: &Path, _: u32) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    fn restore_state_file_shared(&self, _: &Path, _: u32) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
    fn clear_state_file_shared(&self, _: &Path) -> Result<(), RuntimeError> {
        Err(Self::NO_FILES)
    }
}

pub(super) unsafe extern "C" fn rt_load_app_module(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
    carry_state: u32,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::load_app_module", handle, utf8_path, |rt, path| {
            rt.load_app_module_shared(path, carry_state != 0)
        })
    }
}

pub(super) unsafe extern "C" fn rt_save_state_file(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
    schema_version: u32,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::save_state_file", handle, utf8_path, |rt, path| {
            rt.save_state_file_shared(path, schema_version)
        })
    }
}

pub(super) unsafe extern "C" fn rt_restore_state_file(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
    schema_version: u32,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::restore_state_file", handle, utf8_path, |rt, path| {
            rt.restore_state_file_shared(path, schema_version)
        })
    }
}

pub(super) unsafe extern "C" fn rt_clear_state_file(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::clear_state_file", handle, utf8_path, |rt, path| {
            rt.clear_state_file_shared(path)
        })
    }
}
