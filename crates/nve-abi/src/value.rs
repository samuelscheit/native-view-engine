//! Zero-copy `#[repr(C)]` FFI value & direct event ABI types + compact binary `NveBinValue` codec.
//!
//! Two complementary zero-JSON paths share the exact same value model (`NveValueTag`):
//! 1. **In-process direct FFI** (`NveValue`, `NveMapEntry`, `NveDirectEvent`, `NveEventBatch`):
//!    Passed directly by pointer across C++ ↔ Rust (e.g. from `nve::BumpArena`) with zero copy,
//!    zero heap allocation, and zero serialization, accessed safely via borrowed `NveValueRef<'a>`
//!    and `NveDirectEventRef<'a>`.
//! 2. **Compact binary wire format** (`encode_bin_value` / borrowed `BinValueView<'a>`):
//!    Self-describing little-endian binary encoding with O(1) container skipping (`u32 byte_len`)
//!    and a strict nesting depth guard (`NVE_BIN_VALUE_MAX_DEPTH = 64`) for `EventPayload::Custom`
//!    crossing non-shared boundaries (WASM, IPC, replay logs).

use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};

use crate::{DecodeError, NativeEventKind, NveBufferSlice};

/// Maximum permitted container nesting depth for `NveBinValue` encoding & decoding.
pub const NVE_BIN_VALUE_MAX_DEPTH: usize = 64;

/// Borrowed UTF-8 string slice matching `NveStrSlice` in `nve_abi.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveStrSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl NveStrSlice {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    #[inline]
    pub const fn from_str(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    #[inline]
    pub unsafe fn as_bytes<'a>(self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    #[inline]
    pub unsafe fn as_str<'a>(self) -> Result<&'a str, DecodeError> {
        let bytes = unsafe { self.as_bytes() };
        std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
    }
}

/// Discriminant tags shared between `NveValue::tag` and the binary `NveBinValue` wire format.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NveValueTag {
    Null = 0,
    Bool = 1,
    I64 = 2,
    F64 = 3,
    Str = 4,
    Bytes = 5,
    Array = 6,
    Map = 7,
}

impl NveValueTag {
    #[inline]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::I64),
            3 => Some(Self::F64),
            4 => Some(Self::Str),
            5 => Some(Self::Bytes),
            6 => Some(Self::Array),
            7 => Some(Self::Map),
            _ => None,
        }
    }

    #[inline]
    pub const fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match Self::from_u32(v as u32) {
            Some(t) => Ok(t),
            None => Err(DecodeError::InvalidEnum("NveValueTag", v as u32)),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveValueArray {
    pub ptr: *const NveValue,
    pub len: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveValueMap {
    pub ptr: *const NveMapEntry,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union NveValueData {
    pub bool_val: u8,
    pub i64_val: i64,
    pub f64_val: f64,
    pub str_val: NveStrSlice,
    pub bytes_val: NveBufferSlice,
    pub array_val: NveValueArray,
    pub map_val: NveValueMap,
}

/// Tagged union value matching `struct NveValue` in `nve_abi.h` (24 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveValue {
    pub tag: u32,
    pub data: NveValueData,
}

impl NveValue {
    #[inline]
    pub const fn null() -> Self {
        Self {
            tag: NveValueTag::Null as u32,
            data: NveValueData { i64_val: 0 },
        }
    }

    #[inline]
    pub const fn bool(v: bool) -> Self {
        Self {
            tag: NveValueTag::Bool as u32,
            data: NveValueData {
                bool_val: if v { 1 } else { 0 },
            },
        }
    }

    #[inline]
    pub const fn i64(v: i64) -> Self {
        Self {
            tag: NveValueTag::I64 as u32,
            data: NveValueData { i64_val: v },
        }
    }

    #[inline]
    pub const fn f64(v: f64) -> Self {
        Self {
            tag: NveValueTag::F64 as u32,
            data: NveValueData { f64_val: v },
        }
    }

    #[inline]
    pub const fn str(s: NveStrSlice) -> Self {
        Self {
            tag: NveValueTag::Str as u32,
            data: NveValueData { str_val: s },
        }
    }

    #[inline]
    pub const fn from_str(s: &str) -> Self {
        Self::str(NveStrSlice::from_str(s))
    }

    #[inline]
    pub const fn bytes(b: NveBufferSlice) -> Self {
        Self {
            tag: NveValueTag::Bytes as u32,
            data: NveValueData { bytes_val: b },
        }
    }

    #[inline]
    pub const fn from_bytes(b: &[u8]) -> Self {
        Self::bytes(NveBufferSlice {
            ptr: b.as_ptr(),
            len: b.len(),
        })
    }

    #[inline]
    pub const fn array(elems: &[NveValue]) -> Self {
        Self {
            tag: NveValueTag::Array as u32,
            data: NveValueData {
                array_val: NveValueArray {
                    ptr: elems.as_ptr(),
                    len: elems.len(),
                },
            },
        }
    }

    #[inline]
    pub const fn map(entries: &[NveMapEntry]) -> Self {
        Self {
            tag: NveValueTag::Map as u32,
            data: NveValueData {
                map_val: NveValueMap {
                    ptr: entries.as_ptr(),
                    len: entries.len(),
                },
            },
        }
    }

    /// Borrow a zero-copy `NveValueRef<'a>` view over this FFI value.
    ///
    /// # Safety
    /// All nested pointers reachable from `self` (`str_val`, `bytes_val`, `array_val`, `map_val`)
    /// must remain valid and immutable for `'a`.
    #[inline]
    pub unsafe fn as_ref<'a>(&'a self) -> NveValueRef<'a> {
        NveValueRef { raw: self }
    }
}

impl Default for NveValue {
    #[inline]
    fn default() -> Self {
        Self::null()
    }
}

/// Key/value entry matching `struct NveMapEntry` in `nve_abi.h` (40 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveMapEntry {
    pub key: NveStrSlice,
    pub value: NveValue,
}

impl NveMapEntry {
    #[inline]
    pub const fn new(key: &str, value: NveValue) -> Self {
        Self {
            key: NveStrSlice::from_str(key),
            value,
        }
    }
}

/// Direct FFI event matching `struct NveDirectEvent` in `nve_abi.h` (96 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NveDirectEvent {
    pub surface_id: u32,
    pub node_id: u32,
    pub timestamp_us: u64,
    pub kind: u8,
    pub payload_tag: u8,
    pub name: NveStrSlice,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: NveStrSlice,
    pub value: NveValue,
}

impl Default for NveDirectEvent {
    #[inline]
    fn default() -> Self {
        Self {
            surface_id: 0,
            node_id: 0,
            timestamp_us: 0,
            kind: 0,
            payload_tag: 0,
            name: NveStrSlice::EMPTY,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            text: NveStrSlice::EMPTY,
            value: NveValue::null(),
        }
    }
}

impl NveDirectEvent {
    #[inline]
    pub const fn press(surface_id: u32, node_id: u32, timestamp_us: u64, x: f32, y: f32) -> Self {
        Self {
            surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::Press as u8,
            payload_tag: 1,
            name: NveStrSlice::EMPTY,
            x,
            y,
            width: 0.0,
            height: 0.0,
            text: NveStrSlice::EMPTY,
            value: NveValue::null(),
        }
    }

    #[inline]
    pub const fn text_change(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        text: &str,
    ) -> Self {
        Self {
            surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::TextChange as u8,
            payload_tag: 3,
            name: NveStrSlice::EMPTY,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            text: NveStrSlice::from_str(text),
            value: NveValue::null(),
        }
    }

    #[inline]
    pub const fn custom(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        name: &str,
        value: NveValue,
    ) -> Self {
        Self {
            surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::Custom as u8,
            payload_tag: 5,
            name: NveStrSlice::from_str(name),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            text: NveStrSlice::EMPTY,
            value,
        }
    }
}

/// Contiguous batch of direct events matching `struct NveEventBatch` in `nve_abi.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveEventBatch {
    pub ptr: *const NveDirectEvent,
    pub len: u32,
}

impl NveEventBatch {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };

    #[inline]
    pub fn from_slice(events: &[NveDirectEvent]) -> Self {
        Self {
            ptr: events.as_ptr(),
            len: events.len().min(u32::MAX as usize) as u32,
        }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` initialized `NveDirectEvent`s for `'a`.
    #[inline]
    pub unsafe fn as_slice<'a>(self) -> &'a [NveDirectEvent] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.ptr, self.len as usize) }
        }
    }
}

// Compile-time C ABI layout verification matching `cxx/nve-fabric-bridge/include/nve_abi.h`.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NveStrSlice>() == 16);
    assert!(align_of::<NveStrSlice>() == 8);
    assert!(size_of::<NveValue>() == 24);
    assert!(align_of::<NveValue>() == 8);
    assert!(size_of::<NveMapEntry>() == 40);
    assert!(align_of::<NveMapEntry>() == 8);
    assert!(size_of::<NveDirectEvent>() == 96);
    assert!(align_of::<NveDirectEvent>() == 8);
    assert!(size_of::<NveEventBatch>() == 16);
    assert!(align_of::<NveEventBatch>() == 8);

    assert!(offset_of!(NveValue, tag) == 0);
    assert!(offset_of!(NveValue, data) == 8);
    assert!(offset_of!(NveMapEntry, key) == 0);
    assert!(offset_of!(NveMapEntry, value) == 16);
    assert!(offset_of!(NveDirectEvent, surface_id) == 0);
    assert!(offset_of!(NveDirectEvent, node_id) == 4);
    assert!(offset_of!(NveDirectEvent, timestamp_us) == 8);
    assert!(offset_of!(NveDirectEvent, kind) == 16);
    assert!(offset_of!(NveDirectEvent, payload_tag) == 17);
    assert!(offset_of!(NveDirectEvent, name) == 24);
    assert!(offset_of!(NveDirectEvent, x) == 40);
    assert!(offset_of!(NveDirectEvent, y) == 44);
    assert!(offset_of!(NveDirectEvent, width) == 48);
    assert!(offset_of!(NveDirectEvent, height) == 52);
    assert!(offset_of!(NveDirectEvent, text) == 56);
    assert!(offset_of!(NveDirectEvent, value) == 72);
};

// ============================================================================
// Safe Zero-Copy Borrowed Views (`NveValueRef`, `NvePropsRef`, `NveDirectEventRef`)
// ============================================================================

/// Zero-copy, zero-allocation borrowed view of an FFI `NveValue`.
#[derive(Clone, Copy)]
pub struct NveValueRef<'a> {
    raw: &'a NveValue,
}

impl<'a> NveValueRef<'a> {
    /// # Safety
    /// All nested pointers reachable from `raw` must remain valid and immutable for `'a`.
    #[inline]
    pub const unsafe fn from_raw(raw: &'a NveValue) -> Self {
        Self { raw }
    }

    #[inline]
    pub const fn raw(&self) -> &'a NveValue {
        self.raw
    }

    #[inline]
    pub const fn tag(&self) -> Option<NveValueTag> {
        NveValueTag::from_u32(self.raw.tag)
    }

    #[inline]
    pub const fn is_null(&self) -> bool {
        self.raw.tag == NveValueTag::Null as u32
    }

    #[inline]
    pub const fn as_bool(&self) -> Option<bool> {
        if self.raw.tag == NveValueTag::Bool as u32 {
            Some(unsafe { self.raw.data.bool_val } != 0)
        } else {
            None
        }
    }

    #[inline]
    pub const fn as_i64(&self) -> Option<i64> {
        if self.raw.tag == NveValueTag::I64 as u32 {
            Some(unsafe { self.raw.data.i64_val })
        } else {
            None
        }
    }

    #[inline]
    pub const fn as_f64(&self) -> Option<f64> {
        if self.raw.tag == NveValueTag::F64 as u32 {
            Some(unsafe { self.raw.data.f64_val })
        } else {
            None
        }
    }

    /// Returns numeric value as `f64` for either `F64` or `I64` tags.
    #[inline]
    pub const fn as_number(&self) -> Option<f64> {
        match self.tag() {
            Some(NveValueTag::F64) => Some(unsafe { self.raw.data.f64_val }),
            Some(NveValueTag::I64) => Some(unsafe { self.raw.data.i64_val } as f64),
            _ => None,
        }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&'a str> {
        if self.raw.tag == NveValueTag::Str as u32 {
            let slice = unsafe { self.raw.data.str_val };
            unsafe { slice.as_str() }.ok()
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        if self.raw.tag == NveValueTag::Bytes as u32 {
            let slice = unsafe { self.raw.data.bytes_val };
            Some(unsafe { slice.as_slice() })
        } else {
            None
        }
    }

    #[inline]
    pub fn as_array(&self) -> Option<NveArrayRef<'a>> {
        if self.raw.tag == NveValueTag::Array as u32 {
            let arr = unsafe { self.raw.data.array_val };
            let slice = if arr.ptr.is_null() || arr.len == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(arr.ptr, arr.len) }
            };
            Some(NveArrayRef { elems: slice })
        } else {
            None
        }
    }

    #[inline]
    pub fn as_map(&self) -> Option<NveMapRef<'a>> {
        if self.raw.tag == NveValueTag::Map as u32 {
            let map = unsafe { self.raw.data.map_val };
            let slice = if map.ptr.is_null() || map.len == 0 {
                &[]
            } else {
                unsafe { std::slice::from_raw_parts(map.ptr, map.len) }
            };
            Some(NveMapRef { entries: slice })
        } else {
            None
        }
    }

    /// Zero-copy key lookup when this value is a map.
    #[inline]
    pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> {
        self.as_map()?.get(key)
    }
}

impl std::fmt::Debug for NveValueRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            Some(NveValueTag::Null) => f.write_str("Null"),
            Some(NveValueTag::Bool) => f.debug_tuple("Bool").field(&self.as_bool()).finish(),
            Some(NveValueTag::I64) => f.debug_tuple("I64").field(&self.as_i64()).finish(),
            Some(NveValueTag::F64) => f.debug_tuple("F64").field(&self.as_f64()).finish(),
            Some(NveValueTag::Str) => f.debug_tuple("Str").field(&self.as_str()).finish(),
            Some(NveValueTag::Bytes) => {
                f.debug_tuple("Bytes").field(&self.as_bytes().map(|b| b.len())).finish()
            }
            Some(NveValueTag::Array) => {
                f.debug_tuple("Array").field(&self.as_array().map(|a| a.len())).finish()
            }
            Some(NveValueTag::Map) => {
                f.debug_tuple("Map").field(&self.as_map().map(|m| m.len())).finish()
            }
            None => f.debug_tuple("InvalidTag").field(&self.raw.tag).finish(),
        }
    }
}

/// Zero-copy borrowed slice of `NveValue` elements.
#[derive(Clone, Copy)]
pub struct NveArrayRef<'a> {
    elems: &'a [NveValue],
}

impl<'a> NveArrayRef<'a> {
    #[inline]
    pub const fn len(&self) -> usize {
        self.elems.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.elems.is_empty()
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<NveValueRef<'a>> {
        self.elems.get(index).map(|raw| NveValueRef { raw })
    }

    #[inline]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = NveValueRef<'a>> + 'a {
        self.elems.iter().map(|raw| NveValueRef { raw })
    }
}

/// Zero-copy borrowed slice of `NveMapEntry` key/value pairs (also used for direct props).
#[derive(Clone, Copy)]
pub struct NveMapRef<'a> {
    entries: &'a [NveMapEntry],
}

pub type NvePropsRef<'a> = NveMapRef<'a>;

impl<'a> NveMapRef<'a> {
    pub const EMPTY: Self = Self { entries: &[] };

    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items (and all nested pointers)
    /// for lifetime `'a`.
    #[inline]
    pub unsafe fn from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Self {
        if ptr.is_null() || count == 0 {
            Self::EMPTY
        } else {
            Self {
                entries: unsafe { std::slice::from_raw_parts(ptr, count as usize) },
            }
        }
    }

    /// # Safety
    /// All nested pointers inside `entries` must remain valid for `'a`.
    #[inline]
    pub const unsafe fn from_slice(entries: &'a [NveMapEntry]) -> Self {
        Self { entries }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> {
        let key_bytes = key.as_bytes();
        for entry in self.entries {
            if unsafe { entry.key.as_bytes() } == key_bytes {
                return Some(NveValueRef { raw: &entry.value });
            }
        }
        None
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, NveValueRef<'a>)> + 'a {
        self.entries.iter().filter_map(|entry| {
            let k = unsafe { entry.key.as_str() }.ok()?;
            Some((k, NveValueRef { raw: &entry.value }))
        })
    }
}

/// Zero-copy borrowed view of a single `NveDirectEvent`.
#[derive(Clone, Copy)]
pub struct NveDirectEventRef<'a> {
    raw: &'a NveDirectEvent,
}

impl<'a> NveDirectEventRef<'a> {
    /// # Safety
    /// All pointers inside `raw` must remain valid and immutable for `'a`.
    #[inline]
    pub const unsafe fn from_raw(raw: &'a NveDirectEvent) -> Self {
        Self { raw }
    }

    #[inline]
    pub const fn raw(&self) -> &'a NveDirectEvent {
        self.raw
    }

    #[inline]
    pub const fn surface_id(&self) -> u32 {
        self.raw.surface_id
    }

    #[inline]
    pub const fn node_id(&self) -> u32 {
        self.raw.node_id
    }

    #[inline]
    pub const fn timestamp_us(&self) -> u64 {
        self.raw.timestamp_us
    }

    #[inline]
    pub fn kind(&self) -> Option<NativeEventKind> {
        match self.raw.kind {
            1 => Some(NativeEventKind::Press),
            2 => Some(NativeEventKind::LongPress),
            3 => Some(NativeEventKind::Scroll),
            4 => Some(NativeEventKind::TextChange),
            5 => Some(NativeEventKind::Focus),
            6 => Some(NativeEventKind::Blur),
            7 => Some(NativeEventKind::Layout),
            8 => Some(NativeEventKind::Custom),
            _ => None,
        }
    }

    #[inline]
    pub const fn payload_tag(&self) -> u8 {
        self.raw.payload_tag
    }

    #[inline]
    pub fn name(&self) -> &'a str {
        unsafe { self.raw.name.as_str() }.unwrap_or("")
    }

    #[inline]
    pub const fn x(&self) -> f32 {
        self.raw.x
    }

    #[inline]
    pub const fn y(&self) -> f32 {
        self.raw.y
    }

    #[inline]
    pub const fn width(&self) -> f32 {
        self.raw.width
    }

    #[inline]
    pub const fn height(&self) -> f32 {
        self.raw.height
    }

    #[inline]
    pub fn text(&self) -> &'a str {
        unsafe { self.raw.text.as_str() }.unwrap_or("")
    }

    #[inline]
    pub const fn value(&self) -> NveValueRef<'a> {
        NveValueRef {
            raw: &self.raw.value,
        }
    }
}

/// Zero-copy borrowed batch of `NveDirectEvent`s passed into `NveApp::dispatch_events_direct`.
#[derive(Clone, Copy)]
pub struct NveEventBatchRef<'a> {
    events: &'a [NveDirectEvent],
}

impl<'a> NveEventBatchRef<'a> {
    pub const EMPTY: Self = Self { events: &[] };

    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline]
    pub unsafe fn from_raw(batch: NveEventBatch) -> Self {
        Self {
            events: unsafe { batch.as_slice() },
        }
    }

    /// # Safety
    /// Caller must ensure all nested pointers inside `events` are valid for `'a`.
    #[inline]
    pub const unsafe fn from_slice(events: &'a [NveDirectEvent]) -> Self {
        Self { events }
    }

    #[inline]
    pub const fn len(&self) -> usize {
        self.events.len()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<NveDirectEventRef<'a>> {
        self.events.get(index).map(|raw| NveDirectEventRef { raw })
    }

    #[inline]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = NveDirectEventRef<'a>> + 'a {
        self.events.iter().map(|raw| NveDirectEventRef { raw })
    }
}

// ============================================================================
// Compact Binary `NveBinValue` Wire Encoder & Zero-Copy `BinValueView<'a>`
// ============================================================================
//
// Wire layout (all integers little-endian):
//   tag: u8 (0=Null, 1=Bool, 2=I64, 3=F64, 4=Str, 5=Bytes, 6=Array, 7=Map)
//   - Null:  [0]
//   - Bool:  [1, b: u8 (0|1)]
//   - I64:   [2, v: i64_le]
//   - F64:   [3, v: f64_le]
//   - Str:   [4, len: u32_le, utf8_bytes[len]]
//   - Bytes: [5, len: u32_le, raw_bytes[len]]
//   - Array: [6, payload_byte_len: u32_le, count: u32_le, elem_0, ..., elem_{count-1}]
//   - Map:   [7, payload_byte_len: u32_le, count: u32_le, (klen: u32_le, key_utf8[klen], val), ...]
//
// `payload_byte_len` covers everything AFTER the `payload_byte_len` field (i.e. `4 + children_bytes`),
// enabling O(1) subtree skipping during borrowed lookups.

/// Encodes an `NveValueRef` into the compact binary `NveBinValue` wire format.
pub fn encode_bin_value(value: NveValueRef<'_>, out: &mut Vec<u8>) -> Result<(), DecodeError> {
    encode_bin_value_depth(value, out, 0)
}

/// Convenience helper allocating a `Vec<u8>` and encoding `value` into binary `NveBinValue`.
pub fn encode_bin_value_to_vec(value: NveValueRef<'_>) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::with_capacity(64);
    encode_bin_value(value, &mut out)?;
    Ok(out)
}

fn encode_bin_value_depth(
    value: NveValueRef<'_>,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), DecodeError> {
    let tag = value
        .tag()
        .ok_or(DecodeError::InvalidEnum("NveValueTag", value.raw.tag))?;
    match tag {
        NveValueTag::Null => {
            out.push(NveValueTag::Null as u8);
        }
        NveValueTag::Bool => {
            out.push(NveValueTag::Bool as u8);
            out.push(u8::from(value.as_bool().unwrap_or(false)));
        }
        NveValueTag::I64 => {
            out.push(NveValueTag::I64 as u8);
            out.extend_from_slice(&value.as_i64().unwrap_or(0).to_le_bytes());
        }
        NveValueTag::F64 => {
            out.push(NveValueTag::F64 as u8);
            out.extend_from_slice(&value.as_f64().unwrap_or(0.0).to_le_bytes());
        }
        NveValueTag::Str => {
            let s = unsafe { value.raw.data.str_val.as_str()? };
            let b = s.as_bytes();
            let len = u32::try_from(b.len()).map_err(|_| DecodeError::Truncated)?;
            out.push(NveValueTag::Str as u8);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        NveValueTag::Bytes => {
            let b = value.as_bytes().unwrap_or(&[]);
            let len = u32::try_from(b.len()).map_err(|_| DecodeError::Truncated)?;
            out.push(NveValueTag::Bytes as u8);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(b);
        }
        NveValueTag::Array => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let arr = value.as_array().unwrap();
            let count = u32::try_from(arr.len()).map_err(|_| DecodeError::Truncated)?;
            out.push(NveValueTag::Array as u8);
            let byte_len_pos = out.len();
            out.extend_from_slice(&0u32.to_le_bytes());
            let payload_start = out.len();
            out.extend_from_slice(&count.to_le_bytes());
            for elem in arr.iter() {
                encode_bin_value_depth(elem, out, depth + 1)?;
            }
            let payload_len =
                u32::try_from(out.len() - payload_start).map_err(|_| DecodeError::Truncated)?;
            out[byte_len_pos..byte_len_pos + 4].copy_from_slice(&payload_len.to_le_bytes());
        }
        NveValueTag::Map => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let map = value.as_map().unwrap();
            let count = u32::try_from(map.len()).map_err(|_| DecodeError::Truncated)?;
            out.push(NveValueTag::Map as u8);
            let byte_len_pos = out.len();
            out.extend_from_slice(&0u32.to_le_bytes());
            let payload_start = out.len();
            out.extend_from_slice(&count.to_le_bytes());
            for entry in map.entries {
                let k = unsafe { entry.key.as_str()? };
                let kb = k.as_bytes();
                let klen = u32::try_from(kb.len()).map_err(|_| DecodeError::Truncated)?;
                out.extend_from_slice(&klen.to_le_bytes());
                out.extend_from_slice(kb);
                encode_bin_value_depth(NveValueRef { raw: &entry.value }, out, depth + 1)?;
            }
            let payload_len =
                u32::try_from(out.len() - payload_start).map_err(|_| DecodeError::Truncated)?;
            out[byte_len_pos..byte_len_pos + 4].copy_from_slice(&payload_len.to_le_bytes());
        }
    }
    Ok(())
}

/// Zero-copy, zero-allocation borrowed reader over a validated binary `NveBinValue` byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinValueView<'a> {
    bytes: &'a [u8],
    tag: NveValueTag,
}

#[inline]
fn read_u32_le(bytes: &[u8], pos: usize) -> Result<u32, DecodeError> {
    let end = pos.checked_add(4).ok_or(DecodeError::Truncated)?;
    let slice = bytes.get(pos..end).ok_or(DecodeError::Truncated)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Returns the byte length of one encoded value starting at `bytes[start..]`, validating structure,
/// UTF-8, bool range, bounds, and nesting depth (`<= NVE_BIN_VALUE_MAX_DEPTH`).
fn validate_value_at(bytes: &[u8], start: usize, depth: usize) -> Result<(NveValueTag, usize), DecodeError> {
    let raw_tag = *bytes.get(start).ok_or(DecodeError::Truncated)?;
    let tag = NveValueTag::from_u8(raw_tag)?;
    let mut pos = start + 1;
    match tag {
        NveValueTag::Null => Ok((tag, pos)),
        NveValueTag::Bool => {
            let b = *bytes.get(pos).ok_or(DecodeError::Truncated)?;
            if b > 1 {
                return Err(DecodeError::InvalidEnum("NveBinBool", b as u32));
            }
            Ok((tag, pos + 1))
        }
        NveValueTag::I64 | NveValueTag::F64 => {
            let end = pos.checked_add(8).ok_or(DecodeError::Truncated)?;
            if end > bytes.len() {
                return Err(DecodeError::Truncated);
            }
            Ok((tag, end))
        }
        NveValueTag::Str => {
            let len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let end = pos.checked_add(len).ok_or(DecodeError::Truncated)?;
            let slice = bytes.get(pos..end).ok_or(DecodeError::Truncated)?;
            std::str::from_utf8(slice).map_err(|_| DecodeError::InvalidUtf8)?;
            Ok((tag, end))
        }
        NveValueTag::Bytes => {
            let len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let end = pos.checked_add(len).ok_or(DecodeError::Truncated)?;
            if end > bytes.len() {
                return Err(DecodeError::Truncated);
            }
            Ok((tag, end))
        }
        NveValueTag::Array => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let payload_len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let container_end = pos.checked_add(payload_len).ok_or(DecodeError::Truncated)?;
            if container_end > bytes.len() || payload_len < 4 {
                return Err(DecodeError::Truncated);
            }
            let count = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            // Each element occupies at least 1 byte.
            if count > container_end - pos {
                return Err(DecodeError::Truncated);
            }
            let bounded = &bytes[..container_end];
            for _ in 0..count {
                let (_, next_pos) = validate_value_at(bounded, pos, depth + 1)?;
                pos = next_pos;
            }
            if pos != container_end {
                return Err(DecodeError::TrailingBytes(container_end - pos));
            }
            Ok((tag, container_end))
        }
        NveValueTag::Map => {
            if depth >= NVE_BIN_VALUE_MAX_DEPTH {
                return Err(DecodeError::MaxDepthExceeded);
            }
            let payload_len = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            let container_end = pos.checked_add(payload_len).ok_or(DecodeError::Truncated)?;
            if container_end > bytes.len() || payload_len < 4 {
                return Err(DecodeError::Truncated);
            }
            let count = read_u32_le(bytes, pos)? as usize;
            pos += 4;
            // Each map entry occupies at least 5 bytes (u32 key_len + 1-byte value).
            if count > (container_end - pos) / 5 {
                return Err(DecodeError::Truncated);
            }
            let bounded = &bytes[..container_end];
            for _ in 0..count {
                let klen = read_u32_le(bounded, pos)? as usize;
                pos += 4;
                let kend = pos.checked_add(klen).ok_or(DecodeError::Truncated)?;
                let kslice = bounded.get(pos..kend).ok_or(DecodeError::Truncated)?;
                std::str::from_utf8(kslice).map_err(|_| DecodeError::InvalidUtf8)?;
                pos = kend;
                let (_, next_pos) = validate_value_at(bounded, pos, depth + 1)?;
                pos = next_pos;
            }
            if pos != container_end {
                return Err(DecodeError::TrailingBytes(container_end - pos));
            }
            Ok((tag, container_end))
        }
    }
}

/// Fast O(1) skip over one already-validated value starting at `bytes[start..]`.
#[inline]
fn skip_validated_value(bytes: &[u8], start: usize) -> Option<(NveValueTag, usize)> {
    let tag = NveValueTag::from_u32(*bytes.get(start)? as u32)?;
    let pos = start + 1;
    let end = match tag {
        NveValueTag::Null => pos,
        NveValueTag::Bool => pos + 1,
        NveValueTag::I64 | NveValueTag::F64 => pos + 8,
        NveValueTag::Str | NveValueTag::Bytes | NveValueTag::Array | NveValueTag::Map => {
            let len = read_u32_le(bytes, pos).ok()? as usize;
            pos.checked_add(4)?.checked_add(len)?
        }
    };
    (end <= bytes.len()).then_some((tag, end))
}

impl<'a> BinValueView<'a> {
    /// Validates `bytes` once (zero heap allocation) and returns a borrowed `BinValueView<'a>`.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        let (tag, end) = validate_value_at(bytes, 0, 0)?;
        if end != bytes.len() {
            return Err(DecodeError::TrailingBytes(bytes.len() - end));
        }
        Ok(Self { bytes, tag })
    }

    #[inline]
    pub const fn tag(&self) -> NveValueTag {
        self.tag
    }

    #[inline]
    pub const fn as_raw_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub const fn is_null(&self) -> bool {
        matches!(self.tag, NveValueTag::Null)
    }

    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        (self.tag == NveValueTag::Bool).then(|| self.bytes[1] != 0)
    }

    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        if self.tag != NveValueTag::I64 {
            return None;
        }
        let b: [u8; 8] = self.bytes[1..9].try_into().ok()?;
        Some(i64::from_le_bytes(b))
    }

    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        if self.tag != NveValueTag::F64 {
            return None;
        }
        let b: [u8; 8] = self.bytes[1..9].try_into().ok()?;
        Some(f64::from_le_bytes(b))
    }

    #[inline]
    pub fn as_number(&self) -> Option<f64> {
        match self.tag {
            NveValueTag::F64 => self.as_f64(),
            NveValueTag::I64 => self.as_i64().map(|v| v as f64),
            _ => None,
        }
    }

    #[inline]
    pub fn as_str(&self) -> Option<&'a str> {
        if self.tag != NveValueTag::Str {
            return None;
        }
        let len = read_u32_le(self.bytes, 1).ok()? as usize;
        std::str::from_utf8(self.bytes.get(5..5 + len)?).ok()
    }

    #[inline]
    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        if self.tag != NveValueTag::Bytes {
            return None;
        }
        let len = read_u32_le(self.bytes, 1).ok()? as usize;
        self.bytes.get(5..5 + len)
    }

    #[inline]
    pub fn array_len(&self) -> Option<usize> {
        if self.tag != NveValueTag::Array {
            return None;
        }
        Some(read_u32_le(self.bytes, 5).ok()? as usize)
    }

    /// O(index) single-pass skip to element `index` (nested containers skipped in O(1)).
    pub fn array_at(&self, index: usize) -> Option<BinValueView<'a>> {
        let count = self.array_len()?;
        if index >= count {
            return None;
        }
        let mut pos = 9usize;
        for i in 0..=index {
            let (tag, end) = skip_validated_value(self.bytes, pos)?;
            if i == index {
                return Some(BinValueView {
                    bytes: &self.bytes[pos..end],
                    tag,
                });
            }
            pos = end;
        }
        None
    }

    #[inline]
    pub fn as_array(&self) -> Option<BinArrayIter<'a>> {
        let remaining = self.array_len()?;
        Some(BinArrayIter {
            bytes: self.bytes,
            pos: 9,
            remaining,
            _marker: PhantomData,
        })
    }

    #[inline]
    pub fn map_len(&self) -> Option<usize> {
        if self.tag != NveValueTag::Map {
            return None;
        }
        Some(read_u32_le(self.bytes, 5).ok()? as usize)
    }

    /// Zero-copy borrowed key lookup on a binary map (nested values skipped in O(1)).
    pub fn get(&self, key: &str) -> Option<BinValueView<'a>> {
        let count = self.map_len()?;
        let target = key.as_bytes();
        let mut pos = 9usize;
        for _ in 0..count {
            let klen = read_u32_le(self.bytes, pos).ok()? as usize;
            pos += 4;
            let kbytes = self.bytes.get(pos..pos + klen)?;
            pos += klen;
            let (vtag, vend) = skip_validated_value(self.bytes, pos)?;
            if kbytes == target {
                return Some(BinValueView {
                    bytes: &self.bytes[pos..vend],
                    tag: vtag,
                });
            }
            pos = vend;
        }
        None
    }

    #[inline]
    pub fn map_get(&self, key: &str) -> Option<BinValueView<'a>> {
        self.get(key)
    }

    #[inline]
    pub fn as_map(&self) -> Option<BinMapIter<'a>> {
        let remaining = self.map_len()?;
        Some(BinMapIter {
            bytes: self.bytes,
            pos: 9,
            remaining,
            _marker: PhantomData,
        })
    }
}

pub struct BinArrayIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    remaining: usize,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Iterator for BinArrayIter<'a> {
    type Item = BinValueView<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (tag, end) = skip_validated_value(self.bytes, self.pos)?;
        let item = BinValueView {
            bytes: &self.bytes[self.pos..end],
            tag,
        };
        self.pos = end;
        self.remaining -= 1;
        Some(item)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for BinArrayIter<'_> {}

pub struct BinMapIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    remaining: usize,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Iterator for BinMapIter<'a> {
    type Item = (&'a str, BinValueView<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let klen = read_u32_le(self.bytes, self.pos).ok()? as usize;
        let kstart = self.pos + 4;
        let kend = kstart + klen;
        let key = std::str::from_utf8(self.bytes.get(kstart..kend)?).ok()?;
        let (tag, vend) = skip_validated_value(self.bytes, kend)?;
        let val = BinValueView {
            bytes: &self.bytes[kend..vend],
            tag,
        };
        self.pos = vend;
        self.remaining -= 1;
        Some((key, val))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for BinMapIter<'_> {}
