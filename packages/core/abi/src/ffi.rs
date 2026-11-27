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

use nve_abi_macros::NveAbi;

use crate::id::{DurationUs, FrameId, SurfaceId};
use crate::value::{NveEventBatch, NveMapEntry};

/// Compile-time XXH64 schema fingerprint for NVE's Core C ABI, derived directly from
/// `crate::schema::core_abi_descriptor()` (zero hardcoded hex literals).
pub const NVE_CORE_ABI_SCHEMA_HASH: u64 = crate::schema::compute_core_abi_schema_hash();

pub const NVE_IFACE_SURFACE: u32 = 1;
pub const NVE_IFACE_LAYOUT: u32 = 2;
pub const NVE_IFACE_COMPONENT: u32 = 3;
pub const NVE_IFACE_DEV: u32 = 4;
pub const NVE_IFACE_HOST: u32 = 5;

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
            // SAFETY: Non-null and non-zero `len` checked above; caller guarantees `ptr` is valid for `len` bytes for `'a`.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// Surface viewport configuration passed to `nve_fabric_host_create`.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size = 16)]
pub struct NveSurfaceConfig {
    #[nve_abi(offset = 0, c_type = "NveSurfaceId")]
    pub surface_id: SurfaceId,
    #[nve_abi(offset = 4)]
    pub width: f32,
    #[nve_abi(offset = 8)]
    pub height: f32,
    #[nve_abi(offset = 12)]
    pub scale_factor: f32,
}

/// Per-frame timing and mutation counters populated by `nve_fabric_host_submit_frame`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, NveAbi)]
#[nve_abi(size = 40)]
pub struct NveFrameStats {
    #[nve_abi(offset = 0)]
    pub op_count: u32,
    #[nve_abi(offset = 4)]
    pub mutation_count: u32,
    #[nve_abi(offset = 8, c_type = "u64")]
    pub decode_us: DurationUs,
    #[nve_abi(offset = 16, c_type = "u64")]
    pub layout_us: DurationUs,
    #[nve_abi(offset = 24, c_type = "u64")]
    pub diff_us: DurationUs,
    #[nve_abi(offset = 32, c_type = "u64")]
    pub mount_us: DurationUs,
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

/// Native event discriminator matching `enum NveEventKind` in `nve_abi.generated.h`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveAbi)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[nve_abi(name = "NveEventKind", c_prefix = "NVE_EVENT_", dart_fallback = "Custom")]
pub enum NativeEventKind {
    Press = 1,
    LongPress = 2,
    Scroll = 3,
    TextChange = 4,
    Focus = 5,
    Blur = 6,
    Layout = 7,
    Custom = 8,
    CompositionStart = 9,
    CompositionUpdate = 10,
    CompositionEnd = 11,
    KeyDown = 12,
    KeyUp = 13,
    Submit = 14,
    PointerDown = 15,
    PointerMove = 16,
    PointerUp = 17,
    PointerEnter = 18,
    PointerLeave = 19,
    Wheel = 20,
    TableEvent = 21,
}

impl NativeEventKind {
    #[inline]
    pub const fn from_u8_opt(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Press),
            2 => Some(Self::LongPress),
            3 => Some(Self::Scroll),
            4 => Some(Self::TextChange),
            5 => Some(Self::Focus),
            6 => Some(Self::Blur),
            7 => Some(Self::Layout),
            8 => Some(Self::Custom),
            9 => Some(Self::CompositionStart),
            10 => Some(Self::CompositionUpdate),
            11 => Some(Self::CompositionEnd),
            12 => Some(Self::KeyDown),
            13 => Some(Self::KeyUp),
            14 => Some(Self::Submit),
            15 => Some(Self::PointerDown),
            16 => Some(Self::PointerMove),
            17 => Some(Self::PointerUp),
            18 => Some(Self::PointerEnter),
            19 => Some(Self::PointerLeave),
            20 => Some(Self::Wheel),
            21 => Some(Self::TableEvent),
            _ => None,
        }
    }

    #[inline]
    pub const fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match Self::from_u8_opt(v) {
            Some(kind) => Ok(kind),
            None => Err(DecodeError::InvalidEnum("NativeEventKind", v as u32)),
        }
    }
}

/// Event payload wire tag matching `EventPayload` in `packages/core/abi/src/events.rs`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, NveAbi)]
#[nve_abi(c_prefix = "NVE_EVENT_PAYLOAD_")]
pub enum NveEventPayloadTag {
    Empty = 0,
    Point = 1,
    Scroll = 2,
    Text = 3,
    Layout = 4,
    Custom = 5,
    Key = 6,
    Pointer = 7,
    Wheel = 8,
    TableEvent = 9,
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
pub type NveGetInterfaceFn = unsafe extern "C" fn(interface_id: u32) -> *const c_void;

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
            .join("../../../cxx/nve-fabric-bridge/include/nve_abi.generated.h");
        if !header_path.is_file() {
            return;
        }
        let content = std::fs::read_to_string(&header_path)
            .expect("read generated cxx/nve-fabric-bridge/include/nve_abi.generated.h");
        let expected_macro = format!("0x{NVE_CORE_ABI_SCHEMA_HASH:016X}ULL");
        assert!(
            content.contains(&expected_macro),
            "nve_abi.generated.h schema hash drift! Expected {expected_macro} in {}",
            header_path.display()
        );
    }
}
