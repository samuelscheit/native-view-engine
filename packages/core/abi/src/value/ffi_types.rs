use core::mem::{align_of, size_of};

use crate::{DecodeError, NativeEventKind, NveBufferSlice};

use super::ffi_views::NveValueRef;

/// Validates raw FFI slice preconditions before calling `std::slice::from_raw_parts`:
/// - Non-null pointers must satisfy `ptr.is_aligned()` whenever `align_of::<T>() > 1`.
/// - `len == 0` with a null or aligned pointer safely returns `Some(&[])`.
/// - `len > 0` requires `!ptr.is_null()` and `len * size_of::<T>() <= isize::MAX as usize`
///   without overflow.
///
/// # Safety
/// When `len > 0` and structural preconditions hold, caller must guarantee `ptr` points to `len`
/// valid, initialized, immutable `T` values for lifetime `'a`.
#[inline]
pub(super) unsafe fn guarded_slice<'a, T>(ptr: *const T, len: usize) -> Option<&'a [T]> {
    if !ptr.is_null() && align_of::<T>() > 1 && !ptr.is_aligned() {
        return None;
    }
    if len == 0 {
        return Some(&[]);
    }
    if ptr.is_null() {
        return None;
    }
    let byte_len = len.checked_mul(size_of::<T>())?;
    if byte_len > isize::MAX as usize {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Borrowed UTF-8 string slice matching `NveStrSlice` in `nve_abi.generated.h`.
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

    /// Validated raw-slice view returning `None` on null pointer with `len > 0` or `len > isize::MAX`.
    ///
    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime when `len > 0`.
    #[inline]
    pub unsafe fn try_as_bytes<'a>(self) -> Option<&'a [u8]> {
        unsafe { guarded_slice(self.ptr, self.len) }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    #[inline]
    pub unsafe fn as_bytes<'a>(self) -> &'a [u8] {
        unsafe { self.try_as_bytes() }.unwrap_or(&[])
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` bytes for the returned lifetime.
    #[inline]
    pub unsafe fn as_str<'a>(self) -> Result<&'a str, DecodeError> {
        let bytes = unsafe { self.try_as_bytes() }.ok_or(DecodeError::Truncated)?;
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

/// Tagged union value matching `struct NveValue` in `nve_abi.generated.h` (24 bytes, align 8 on 64-bit).
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
        unsafe { NveValueRef::from_raw(self) }
    }
}

impl Default for NveValue {
    #[inline]
    fn default() -> Self {
        Self::null()
    }
}

/// Key/value entry matching `struct NveMapEntry` in `nve_abi.generated.h` (40 bytes, align 8 on 64-bit).
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

/// Direct FFI event matching `struct NveDirectEvent` in `nve_abi.generated.h` (96 bytes, align 8 on 64-bit).
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

/// Contiguous batch of direct events matching `struct NveEventBatch` in `nve_abi.generated.h`.
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

    /// Validated slice view returning `None` on null pointer with `len > 0`, misaligned pointer,
    /// or byte-size overflow (`> isize::MAX`).
    ///
    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` initialized `NveDirectEvent`s for `'a`.
    #[inline]
    pub unsafe fn try_as_slice<'a>(self) -> Option<&'a [NveDirectEvent]> {
        unsafe { guarded_slice(self.ptr, self.len as usize) }
    }

    /// # Safety
    /// Caller must ensure `ptr` is valid for `len` initialized `NveDirectEvent`s for `'a`.
    #[inline]
    pub unsafe fn as_slice<'a>(self) -> &'a [NveDirectEvent] {
        unsafe { self.try_as_slice() }.unwrap_or(&[])
    }
}

/// Direct zero-copy typed prop entry matching `struct NvePropEntry` in `nve_abi.generated.h`
/// (32 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NvePropEntry {
    pub field_id: u16,
    pub _pad: [u8; 6],
    pub value: NveValue,
}

impl NvePropEntry {
    #[inline]
    pub const fn new(field_id: u16, value: NveValue) -> Self {
        Self {
            field_id,
            _pad: [0; 6],
            value,
        }
    }
}

/// Zero-copy raw pointer slice of typed prop entries matching `struct NvePropSlice` in
/// `nve_abi.generated.h` (16 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NvePropSlice {
    pub entries: *const NvePropEntry,
    pub len: u32,
    pub _pad: u32,
}

impl NvePropSlice {
    pub const EMPTY: Self = Self {
        entries: core::ptr::null(),
        len: 0,
        _pad: 0,
    };

    #[inline]
    pub const fn from_slice(entries: &[NvePropEntry]) -> Self {
        let len = if entries.len() > u32::MAX as usize {
            u32::MAX
        } else {
            entries.len() as u32
        };
        Self {
            entries: entries.as_ptr(),
            len,
            _pad: 0,
        }
    }

    /// Validated slice view returning `None` on null pointer with `len > 0`, misaligned pointer,
    /// or byte-size overflow (`> isize::MAX`).
    ///
    /// # Safety
    /// Caller must ensure `entries` is valid for `len` initialized `NvePropEntry` items for `'a`.
    #[inline]
    pub unsafe fn try_as_slice<'a>(self) -> Option<&'a [NvePropEntry]> {
        unsafe { guarded_slice(self.entries, self.len as usize) }
    }

    /// # Safety
    /// Caller must ensure `entries` is valid for `len` initialized `NvePropEntry` items for `'a`.
    #[inline]
    pub unsafe fn as_slice<'a>(self) -> &'a [NvePropEntry] {
        unsafe { self.try_as_slice() }.unwrap_or(&[])
    }
}

/// Zero-copy direct imperative command dispatch record matching `struct NveCommandSlice` in
/// `nve_abi.generated.h` (16 bytes, align 8 on 64-bit).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveCommandSlice {
    pub opcode: u16,
    pub arg_count: u16,
    pub _pad: u32,
    pub args: *const NveValue,
}

impl NveCommandSlice {
    #[inline]
    pub const fn empty(opcode: u16) -> Self {
        Self {
            opcode,
            arg_count: 0,
            _pad: 0,
            args: core::ptr::null(),
        }
    }

    #[inline]
    pub const fn from_args(opcode: u16, args: &[NveValue]) -> Self {
        let arg_count = if args.len() > u16::MAX as usize {
            u16::MAX
        } else {
            args.len() as u16
        };
        Self {
            opcode,
            arg_count,
            _pad: 0,
            args: args.as_ptr(),
        }
    }

    /// Validated args slice view returning `None` on null pointer with `arg_count > 0`,
    /// misaligned pointer, or byte-size overflow (`> isize::MAX`).
    ///
    /// # Safety
    /// Caller must ensure `args` is valid for `arg_count` initialized `NveValue` items for `'a`.
    #[inline]
    pub unsafe fn try_args_slice<'a>(self) -> Option<&'a [NveValue]> {
        unsafe { guarded_slice(self.args, self.arg_count as usize) }
    }

    /// # Safety
    /// Caller must ensure `args` is valid for `arg_count` initialized `NveValue` items for `'a`.
    #[inline]
    pub unsafe fn args_slice<'a>(self) -> &'a [NveValue] {
        unsafe { self.try_args_slice() }.unwrap_or(&[])
    }
}
