//! Modular zero-cost C ABI (`nve_get_interface`) shared between Rust (`nve-engine`),
//! `libnve_runtime` (which attaches app modules to its surfaces), ObjC++ (`hosts/apple-common`),
//! Kotlin/JNI (`hosts/android-shell`, `hosts/compose-kit`), Dart FFI (`hosts/flutter-plugin`),
//! Swift (`hosts/swiftui-kit`), and Tier-2 hot-swappable app `cdylib`s.
//!
//! Instead of a monolithic vtable that grows on every feature, each NVE module exports ONE
//! entry point `nve_get_interface(uint32_t interface_id) -> *const c_void`. Hosts query the
//! small 2–4 function `const` interface tables they need once at load time and call them directly
//! in CPU registers (~1–2 ns overhead for scalar ops like `measure`/`hit_test`, and single-slice
//! batched `CommandBuffer` / `EventRing` transfers for bulk per-frame tree mutations).

use std::ffi::c_void;

use nve_abi_macros::NveAbi;

use crate::event::NveEventBatch;
use crate::id::{FrameId, InterfaceId, SurfaceId};
use crate::value::{NveMapEntry, NveValueTag};

/// Compile-time XXH64 schema fingerprint for NVE's Core C ABI, derived directly from
/// `crate::schema::core_abi_descriptor()` (zero hardcoded hex literals).
pub const NVE_CORE_ABI_SCHEMA_HASH: u64 = crate::schema::compute_core_abi_schema_hash();

pub const NVE_IFACE_SURFACE: InterfaceId = InterfaceId(1);
pub const NVE_IFACE_LAYOUT: InterfaceId = InterfaceId(2);
pub const NVE_IFACE_COMPONENT: InterfaceId = InterfaceId(3);
pub const NVE_IFACE_DEV: InterfaceId = InterfaceId(4);
pub const NVE_IFACE_HOST: InterfaceId = InterfaceId(5);

/// Pointer of a borrowed slab published across the C ABI: NULL when empty (every zero-count slab
/// is published as NULL, never as a dangling non-null pointer).
#[inline]
pub fn slab_ptr<T>(slab: &[T]) -> *const T {
    if slab.is_empty() {
        core::ptr::null()
    } else {
        slab.as_ptr()
    }
}

/// Borrowed immutable byte slice across the C ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 16)]
pub struct NveBufferSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl NveBufferSlice {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    /// Borrows `s`; empty slices are `EMPTY` (NULL pointer, never a dangling one).
    pub fn from_slice(s: &[u8]) -> Self {
        if s.is_empty() {
            return Self::EMPTY;
        }
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// Validated raw-slice view returning `None` on a null pointer with `len > 0` or on
    /// `len > isize::MAX`.
    ///
    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime when `len > 0`.
    #[inline]
    pub unsafe fn try_as_slice<'a>(self) -> Option<&'a [u8]> {
        // SAFETY: Caller guarantees `self.ptr` is valid for `self.len` bytes for `'a`.
        unsafe { crate::value::guarded_slice(self.ptr, self.len) }
    }

    /// [`Self::try_as_slice`], with an invalid slice read as empty.
    ///
    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    #[inline]
    pub unsafe fn as_slice<'a>(self) -> &'a [u8] {
        // SAFETY: Forwarded caller contract.
        unsafe { self.try_as_slice() }.unwrap_or(&[])
    }
}

/// Box layout constraints passed to `NveLayoutApi::measure`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, NveAbi)]
#[nve_abi(size = 16)]
pub struct NveSizeConstraints {
    #[nve_abi(offset = 0)]
    pub min_w: f32,
    #[nve_abi(offset = 4)]
    pub min_h: f32,
    #[nve_abi(offset = 8)]
    pub max_w: f32,
    #[nve_abi(offset = 12)]
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

/// Measured intrinsic dimensions returned by `NveLayoutApi::measure`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default, NveAbi)]
#[nve_abi(size = 8)]
pub struct NveIntrinsicSize {
    #[nve_abi(offset = 0)]
    pub width: f32,
    #[nve_abi(offset = 4)]
    pub height: f32,
}


/// Gesture arbitration disposition returned by `NveLayoutApi::hit_test`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveAbi)]
#[nve_abi(c_prefix = "NVE_GESTURE_", dart_fallback = "PassThrough")]
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

/// Validation / decoding errors shared across zero-copy FFI views and binary wire codecs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    InvalidMagic(u32),
    UnsupportedVersion(u16),
    UnknownOpcode(u8),
    InvalidEnum(&'static str, u32),
    InvalidUtf8,
    TrailingBytes(usize),
    MaxDepthExceeded,
    /// A value's wire tag differs from the tag its declared type requires (`found` is the raw tag).
    UnexpectedTag { expected: NveValueTag, found: u32 },
    /// An integer does not fit the named declared type.
    OutOfRange(&'static str),
    /// A required struct field is absent.
    MissingField(&'static str),
    /// A string, byte run, container payload, or element count exceeds the wire format's `u32`.
    TooLarge,
    InvalidEvent(crate::event::EventValidationError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DecodeError {}

/// Opaque handle to an instantiated NVE app/component.
pub type NveAppHandle = *mut c_void;

/// Single exported entry point per NVE module (`nve_get_interface`).
pub type NveGetInterfaceFn = unsafe extern "C" fn(interface_id: InterfaceId) -> *const c_void;

/// Required core surface interface (`NVE_IFACE_SURFACE = 1`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveSurfaceApi {
    pub create: unsafe extern "C" fn(surface_id: SurfaceId) -> NveAppHandle,
    pub destroy: unsafe extern "C" fn(handle: NveAppHandle),
    pub render: unsafe extern "C" fn(handle: NveAppHandle, frame_id: FrameId) -> NveBufferSlice,
    pub dispatch_events: unsafe extern "C" fn(handle: NveAppHandle, batch: NveEventBatch) -> u32,
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
    pub set_props:
        unsafe extern "C" fn(handle: NveAppHandle, entries: *const NveMapEntry, count: u32) -> u32,
    pub drain_emitted_events: unsafe extern "C" fn(handle: NveAppHandle) -> NveEventBatch,
}

/// Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveDevApi {
    pub apply_hot_patch:
        unsafe extern "C" fn(handle: NveAppHandle, patch_nvp1: NveBufferSlice) -> u32,
    pub snapshot_state: unsafe extern "C" fn(handle: NveAppHandle) -> NveBufferSlice,
    pub restore_state: unsafe extern "C" fn(handle: NveAppHandle, state: NveBufferSlice),
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn core_c_header_matches_compile_time_schema_hash_when_present() {
        assert_ne!(NVE_CORE_ABI_SCHEMA_HASH, 0);
        let header_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../host-cxx/include/nve/generated/nve_abi.generated.h");
        if !header_path.is_file() {
            return;
        }
        let content = std::fs::read_to_string(&header_path)
            .expect("read generated host-cxx nve_abi.generated.h");
        let expected_macro = format!("0x{NVE_CORE_ABI_SCHEMA_HASH:016X}ULL");
        assert!(
            content.contains(&expected_macro),
            "nve_abi.generated.h schema hash drift! Expected {expected_macro} in {}",
            header_path.display()
        );
    }
}
