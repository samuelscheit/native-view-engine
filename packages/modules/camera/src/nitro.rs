//! Optional zero-JS-VM C-ABI bindings to `packages/modules/nitro-bridge` (`HybridCameraSession`).

use std::ffi::{CStr, c_char, c_void};

/// Canonical registered `HybridObject` name for VisionCamera v5 `HybridCameraSession`.
pub const HYBRID_CAMERA_SESSION_NAME: &str = "CameraSession";

/// Opaque handle mirroring `NveNitroHandle` in `packages/modules/nitro-bridge/include/nve_nitro_core.hpp`.
pub type NveNitroHandle = *mut c_void;

/// Zero-copy C-ABI callback receiving a validated `#[repr(C)]` [`crate::CameraFrameDescriptor`]
/// directly by pointer (no 40-byte `NVCF` byte-slice serialization copy).
#[cfg(feature = "spec")]
pub type CameraFrameCallbackFn =
    unsafe extern "C" fn(user_data: *mut c_void, frame: *const crate::CameraFrameDescriptor);

/// Zero-copy C-ABI callback receiving a validated 40-byte `#[repr(C)]` [`crate::CameraFrameWireLayout`]
/// directly by pointer.
#[cfg(feature = "spec")]
pub type CameraFrameWireCallbackFn =
    unsafe extern "C" fn(user_data: *mut c_void, frame: *const crate::CameraFrameWireLayout);

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NitroStatus {
    Ok = 0,
    InvalidHandle = 1,
    NotFound = 2,
    Disposed = 3,
    BufferTooSmall = 4,
    InvocationFailed = 5,
}

impl NitroStatus {
    pub const fn from_raw(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::InvalidHandle,
            2 => Self::NotFound,
            3 => Self::Disposed,
            4 => Self::BufferTooSmall,
            _ => Self::InvocationFailed,
        }
    }
}

/// Function table matching the `extern "C"` C ABI exported by `nve_nitro_bridge`.
#[derive(Debug, Clone, Copy)]
pub struct NitroBridgeVTable {
    pub create: unsafe extern "C" fn(name: *const c_char) -> NveNitroHandle,
    pub retain: unsafe extern "C" fn(handle: NveNitroHandle),
    pub release: unsafe extern "C" fn(handle: NveNitroHandle),
    pub dispose: unsafe extern "C" fn(handle: NveNitroHandle),
    pub is_disposed: unsafe extern "C" fn(handle: NveNitroHandle) -> i32,
    pub get_name: unsafe extern "C" fn(handle: NveNitroHandle) -> *const c_char,
    pub invoke_bytes: unsafe extern "C" fn(
        handle: NveNitroHandle,
        method: *const c_char,
        arg_bytes: *const u8,
        arg_len: usize,
        out_buf: *mut u8,
        out_cap: usize,
        out_len: *mut usize,
    ) -> i32,
}

/// Safe RAII wrapper around a `HybridCameraSession` handle obtained from `nve_nitro_bridge`.
pub struct NitroHybridHandle {
    raw: NveNitroHandle,
    vtable: NitroBridgeVTable,
}

impl NitroHybridHandle {
    /// Creates a new handle by name using `vtable`.
    ///
    /// # Safety
    /// The caller must supply valid function pointers conforming to `nve_nitro_core.hpp`.
    pub unsafe fn create(vtable: NitroBridgeVTable, name: &CStr) -> Option<Self> {
        let raw = unsafe { (vtable.create)(name.as_ptr()) };
        if raw.is_null() {
            None
        } else {
            Some(Self { raw, vtable })
        }
    }

    pub fn is_disposed(&self) -> bool {
        unsafe { (self.vtable.is_disposed)(self.raw) != 0 }
    }

    pub fn dispose(&self) {
        unsafe { (self.vtable.dispose)(self.raw) };
    }

    pub fn invoke_bytes(&self, method: &CStr, args: &[u8]) -> Result<Vec<u8>, NitroStatus> {
        let mut out = vec![0u8; 256];
        let mut out_len = 0usize;
        let rc = unsafe {
            (self.vtable.invoke_bytes)(
                self.raw,
                method.as_ptr(),
                args.as_ptr(),
                args.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut out_len,
            )
        };
        let status = NitroStatus::from_raw(rc);
        if status == NitroStatus::BufferTooSmall {
            out.resize(out_len, 0);
            let rc2 = unsafe {
                (self.vtable.invoke_bytes)(
                    self.raw,
                    method.as_ptr(),
                    args.as_ptr(),
                    args.len(),
                    out.as_mut_ptr(),
                    out.len(),
                    &mut out_len,
                )
            };
            let status2 = NitroStatus::from_raw(rc2);
            if status2 != NitroStatus::Ok {
                return Err(status2);
            }
            out.truncate(out_len);
            return Ok(out);
        }
        if status != NitroStatus::Ok {
            return Err(status);
        }
        out.truncate(out_len);
        Ok(out)
    }
}

impl Drop for NitroHybridHandle {
    fn drop(&mut self) {
        unsafe { (self.vtable.release)(self.raw) };
    }
}
