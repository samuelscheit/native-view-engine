//! Modular zero-cost C ABI (`nve_get_interface`) shared between Rust (`nve-engine`),
//! C++ (`nve-fabric-bridge`), ObjC++ (`hosts/ios-shell`, `hosts/macos-shell`),
//! Kotlin/JNI (`hosts/android-shell`, `hosts/compose-kit`), Dart FFI (`hosts/flutter-plugin`),
//! Swift (`hosts/swiftui-kit`), and Tier-2 hot-swappable app `cdylib`s.
//!
//! Instead of a monolithic vtable that grows on every feature, each NVE module exports ONE
//! entry point `nve_get_interface(uint32_t interface_id) -> *const c_void`. Hosts query the
//! small 2–4 function `const` interface tables they need once at load time and call them directly
//! in CPU registers (~1–2 ns overhead for scalar ops like `measure`/`hit_test`, and single-slice
//! batched `CommandBuffer` / `EventRing` transfers for bulk per-frame tree mutations).

use std::ffi::c_void;

pub const NVE_IFACE_SURFACE: u32 = 1;
pub const NVE_IFACE_LAYOUT: u32 = 2;
pub const NVE_IFACE_COMPONENT: u32 = 3;
pub const NVE_IFACE_DEV: u32 = 4;

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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NveSizeConstraints {
    pub min_w: f32,
    pub min_h: f32,
    pub max_w: f32,
    pub max_h: f32,
}

impl NveSizeConstraints {
    pub const fn loose(max_w: f32, max_h: f32) -> Self {
        Self {
            min_w: 0.0,
            min_h: 0.0,
            max_w,
            max_h,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NveIntrinsicSize {
    pub width: f32,
    pub height: f32,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NveGestureDisposition {
    Claim = 1,
    PassThrough = 2,
    DeferToParent = 3,
}

impl NveGestureDisposition {
    pub const fn from_u8(code: u8) -> Self {
        match code {
            1 => Self::Claim,
            3 => Self::DeferToParent,
            _ => Self::PassThrough,
        }
    }
}

/// Opaque handle to an instantiated NVE app/component.
pub type NveAppHandle = *mut c_void;

/// Single exported entry point per NVE module (`nve_get_interface`).
pub type NveGetInterfaceFn = unsafe extern "C" fn(interface_id: u32) -> *const c_void;

/// Required core surface interface (`NVE_IFACE_SURFACE = 1`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveSurfaceApi {
    pub create: unsafe extern "C" fn(surface_id: u32) -> NveAppHandle,
    pub destroy: unsafe extern "C" fn(handle: NveAppHandle),
    pub render: unsafe extern "C" fn(handle: NveAppHandle, frame_id: u64) -> NveBufferSlice,
    pub dispatch_events:
        unsafe extern "C" fn(handle: NveAppHandle, events: NveBufferSlice) -> u32,
}

/// Optional layout & gesture negotiation interface (`NVE_IFACE_LAYOUT = 2`).
/// Pure register-to-register scalar calls (~1–2 ns, zero serialization).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveLayoutApi {
    pub measure:
        unsafe extern "C" fn(handle: NveAppHandle, constraints: NveSizeConstraints) -> NveIntrinsicSize,
    pub hit_test: unsafe extern "C" fn(
        handle: NveAppHandle,
        x: f32,
        y: f32,
        pointer_phase: u8,
    ) -> u8,
}

/// Optional embedded component props & outbound event interface (`NVE_IFACE_COMPONENT = 3`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveComponentApi {
    pub set_props: unsafe extern "C" fn(handle: NveAppHandle, props: NveBufferSlice) -> u32,
    pub drain_emitted_events: unsafe extern "C" fn(handle: NveAppHandle) -> NveBufferSlice,
}

/// Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveDevApi {
    pub apply_hot_patch:
        unsafe extern "C" fn(handle: NveAppHandle, patch_json: NveBufferSlice) -> u32,
    pub snapshot_state: unsafe extern "C" fn(handle: NveAppHandle) -> NveBufferSlice,
    pub restore_state: unsafe extern "C" fn(handle: NveAppHandle, state: NveBufferSlice),
}
