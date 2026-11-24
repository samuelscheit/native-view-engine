//! C ABI structs shared between Rust (`nve-engine`), C++ (`nve-fabric-bridge`),
//! Swift/ObjC++ (`hosts/ios-shell`), Kotlin/JNI (`hosts/android-shell`), and Tier-2 hot-swapped app dylibs.

use std::ffi::c_void;

pub const NVE_APP_ABI_VERSION: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveBufferSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl NveBufferSlice {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    pub fn from_slice(s: &[u8]) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    pub unsafe fn as_slice<'a>(self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveSurfaceConfig {
    pub surface_id: u32,
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct NveFrameStats {
    pub op_count: u32,
    pub mutation_count: u32,
    pub decode_us: u64,
    pub layout_us: u64,
    pub diff_us: u64,
    pub mount_us: u64,
}

/// Opaque handle returned by a Tier-2 hot-swappable Rust app `cdylib`.
pub type NveAppHandle = *mut c_void;

/// Stable C ABI vtable exported by any hot-reloadable app `cdylib` (`nve_app_vtable`).
///
/// The prebuilt native Host Shell (macOS Skia / iOS Simulator / Android Emulator) loads the
/// compiled app `.dylib`/`.so`, calls `snapshot_state` on the previous generation, unloads or
/// retires it, calls `create` + `restore_state` on the new generation, and renders the next frame
/// in ~150-400ms without recompiling Xcode, Gradle, or C++ Fabric.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveAppVTable {
    pub abi_version: u32,
    pub create: unsafe extern "C" fn(surface_id: u32) -> NveAppHandle,
    pub destroy: unsafe extern "C" fn(handle: NveAppHandle),
    /// Renders pending mutations into an internal buffer and returns a slice valid until next call.
    pub render: unsafe extern "C" fn(handle: NveAppHandle, frame_id: u64) -> NveBufferSlice,
    /// Feeds a binary `EventRing` batch into the app; returns 1 if a re-render is needed.
    pub dispatch_events:
        unsafe extern "C" fn(handle: NveAppHandle, events: NveBufferSlice) -> u32,
    /// Applies a JSON-encoded Tier-1 hot-reload patch (RSX template / style literal update).
    pub apply_hot_patch:
        unsafe extern "C" fn(handle: NveAppHandle, patch_json: NveBufferSlice) -> u32,
    /// Serializes app state for preservation across a Tier-2 dylib reload.
    pub snapshot_state: unsafe extern "C" fn(handle: NveAppHandle) -> NveBufferSlice,
    /// Restores previously snapshotted state after a Tier-2 dylib reload.
    pub restore_state: unsafe extern "C" fn(handle: NveAppHandle, state: NveBufferSlice),
    /// Pushes host-supplied props (e.g. from React Native `<NveComponent />`) into an exported NVE component.
    /// Encoded as a JSON or binary prop map; returns 1 if a re-render is required.
    pub set_props: unsafe extern "C" fn(handle: NveAppHandle, props: NveBufferSlice) -> u32,
    /// Drains component-emitted outbound events (encoded as an `EventRing` batch) for host RN `EventEmitter`s.
    pub drain_emitted_events: unsafe extern "C" fn(handle: NveAppHandle) -> NveBufferSlice,
}
