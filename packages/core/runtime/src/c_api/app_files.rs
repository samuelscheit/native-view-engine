//! App-file & Frame-0 C ABI entry points: staged `cdylib` app modules, `StateFile` persistence,
//! and `Frame0File` save/boot (`boot_frame0_bytes` also works on `wasm32`).

use std::path::Path;

use nve_abi::{NveBufferSlice, NveRuntimeHandle, NveStrSlice};

use super::{host_bytes, with_runtime};
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

pub(super) unsafe extern "C" fn rt_save_frame0_file(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
    app_build_id: u64,
    env_hash: u64,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::save_frame0_file", handle, utf8_path, |rt, path| {
            rt.save_frame0_file_shared(path, app_build_id, env_hash)
        })
    }
}

pub(super) unsafe extern "C" fn rt_boot_frame0_file(
    handle: NveRuntimeHandle,
    utf8_path: NveStrSlice,
    app_build_id: u64,
    env_hash: u64,
) -> i32 {
    // SAFETY: Forwarded caller contract for `utf8_path`.
    unsafe {
        with_path("nve_runtime::boot_frame0_file", handle, utf8_path, |rt, path| {
            rt.boot_frame0_file_shared(path, app_build_id, env_hash)
        })
    }
}

pub(super) unsafe extern "C" fn rt_boot_frame0_bytes(
    handle: NveRuntimeHandle,
    envelope: NveBufferSlice,
    app_build_id: u64,
    env_hash: u64,
) -> i32 {
    with_runtime("nve_runtime::boot_frame0_bytes", handle, |rt| {
        // SAFETY: Caller guarantees `envelope` is readable for `envelope.len` bytes when valid.
        let bytes = unsafe { host_bytes(envelope, "invalid frame0 envelope NveBufferSlice") }?;
        rt.boot_frame0_bytes_shared(bytes, app_build_id, env_hash)
    })
}
