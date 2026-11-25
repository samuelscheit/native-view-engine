use crate::NativeEventKind;

use super::ffi_types::{
    guarded_slice, NveCommandSlice, NveDirectEvent, NveEventBatch, NveMapEntry, NvePropEntry,
    NvePropSlice, NveValue, NveValueTag,
};

/// Zero-copy, zero-allocation borrowed view of an FFI `NveValue`.
#[derive(Clone, Copy)]
pub struct NveValueRef<'a> {
    pub(super) raw: &'a NveValue,
}

impl<'a> NveValueRef<'a> {
    /// # Safety
    /// All nested pointers reachable from `raw` must remain valid and immutable for `'a`.
    #[inline] pub const unsafe fn from_raw(raw: &'a NveValue) -> Self { Self { raw } }
    #[inline] pub const fn raw(&self) -> &'a NveValue { self.raw }
    #[inline] pub const fn tag(&self) -> Option<NveValueTag> { NveValueTag::from_u32(self.raw.tag) }
    #[inline] pub const fn is_null(&self) -> bool { self.raw.tag == NveValueTag::Null as u32 }
    #[inline] pub const fn as_bool(&self) -> Option<bool> {
        if self.raw.tag == NveValueTag::Bool as u32 { Some(unsafe { self.raw.data.bool_val } != 0) } else { None }
    }
    #[inline] pub const fn as_i64(&self) -> Option<i64> {
        if self.raw.tag == NveValueTag::I64 as u32 { Some(unsafe { self.raw.data.i64_val }) } else { None }
    }
    #[inline] pub const fn as_f64(&self) -> Option<f64> {
        if self.raw.tag == NveValueTag::F64 as u32 { Some(unsafe { self.raw.data.f64_val }) } else { None }
    }
    /// Returns numeric value as `f64` for either `F64` or `I64` tags.
    #[inline] pub const fn as_number(&self) -> Option<f64> {
        match self.tag() {
            Some(NveValueTag::F64) => Some(unsafe { self.raw.data.f64_val }),
            Some(NveValueTag::I64) => Some(unsafe { self.raw.data.i64_val } as f64),
            _ => None,
        }
    }
    #[inline] pub fn as_str(&self) -> Option<&'a str> {
        (self.raw.tag == NveValueTag::Str as u32)
            .then(|| unsafe { self.raw.data.str_val.as_str() }.ok())
            .flatten()
    }
    #[inline] pub fn as_bytes(&self) -> Option<&'a [u8]> {
        (self.raw.tag == NveValueTag::Bytes as u32).then(|| {
            let s = unsafe { self.raw.data.bytes_val };
            unsafe { guarded_slice(s.ptr, s.len) }
        })?
    }
    #[inline] pub fn as_array(&self) -> Option<NveArrayRef<'a>> {
        (self.raw.tag == NveValueTag::Array as u32).then(|| {
            let a = unsafe { self.raw.data.array_val };
            unsafe { guarded_slice(a.ptr, a.len) }.map(|elems| NveArrayRef { elems })
        })?
    }
    #[inline] pub fn as_map(&self) -> Option<NveMapRef<'a>> {
        (self.raw.tag == NveValueTag::Map as u32).then(|| {
            let m = unsafe { self.raw.data.map_val };
            unsafe { guarded_slice(m.ptr, m.len) }.map(|entries| NveMapRef { entries })
        })?
    }
    /// Zero-copy key lookup when this value is a map.
    #[inline] pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> { self.as_map()?.get(key) }
}

impl std::fmt::Debug for NveValueRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.tag() {
            Some(NveValueTag::Null) => f.write_str("Null"),
            Some(NveValueTag::Bool) => f.debug_tuple("Bool").field(&self.as_bool()).finish(),
            Some(NveValueTag::I64) => f.debug_tuple("I64").field(&self.as_i64()).finish(),
            Some(NveValueTag::F64) => f.debug_tuple("F64").field(&self.as_f64()).finish(),
            Some(NveValueTag::Str) => f.debug_tuple("Str").field(&self.as_str()).finish(),
            Some(NveValueTag::Bytes) => f.debug_tuple("Bytes").field(&self.as_bytes().map(<[u8]>::len)).finish(),
            Some(NveValueTag::Array) => f.debug_tuple("Array").field(&self.as_array().map(|a| a.len())).finish(),
            Some(NveValueTag::Map) => f.debug_tuple("Map").field(&self.as_map().map(|m| m.len())).finish(),
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
    #[inline] pub const fn len(&self) -> usize { self.elems.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.elems.is_empty() }
    #[inline] pub fn get(&self, index: usize) -> Option<NveValueRef<'a>> {
        self.elems.get(index).map(|raw| NveValueRef { raw })
    }
    #[inline] pub fn iter(&self) -> impl ExactSizeIterator<Item = NveValueRef<'a>> + 'a {
        self.elems.iter().map(|raw| NveValueRef { raw })
    }
}

/// Zero-copy borrowed slice of `NveMapEntry` key/value pairs (also used for direct props).
#[derive(Clone, Copy)]
pub struct NveMapRef<'a> {
    #[cfg_attr(not(feature = "wire"), allow(dead_code))]
    pub(super) entries: &'a [NveMapEntry],
}

pub type NvePropsRef<'a> = NveMapRef<'a>;

impl<'a> NveMapRef<'a> {
    pub const EMPTY: Self = Self { entries: &[] };

    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items and nested pointers for `'a`.
    #[inline] pub unsafe fn try_from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Option<Self> {
        Some(Self { entries: unsafe { guarded_slice(ptr, count as usize) }? })
    }
    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items and nested pointers for `'a`.
    #[inline] pub unsafe fn from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Self {
        unsafe { Self::try_from_raw_parts(ptr, count) }.unwrap_or(Self::EMPTY)
    }
    /// # Safety
    /// All nested pointers inside `entries` must remain valid for `'a`.
    #[inline] pub const unsafe fn from_slice(entries: &'a [NveMapEntry]) -> Self { Self { entries } }
    #[inline] pub const fn len(&self) -> usize { self.entries.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.entries.is_empty() }
    #[inline] pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> {
        let key_bytes = key.as_bytes();
        self.entries.iter().find_map(|entry| {
            (unsafe { entry.key.try_as_bytes() } == Some(key_bytes))
                .then_some(NveValueRef { raw: &entry.value })
        })
    }
    #[inline] pub fn iter(&self) -> impl Iterator<Item = (&'a str, NveValueRef<'a>)> + 'a {
        self.entries.iter().filter_map(|entry| {
            Some((unsafe { entry.key.as_str() }.ok()?, NveValueRef { raw: &entry.value }))
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
    #[inline] pub const unsafe fn from_raw(raw: &'a NveDirectEvent) -> Self { Self { raw } }
    #[inline] pub const fn raw(&self) -> &'a NveDirectEvent { self.raw }
    #[inline] pub const fn surface_id(&self) -> u32 { self.raw.surface_id }
    #[inline] pub const fn node_id(&self) -> u32 { self.raw.node_id }
    #[inline] pub const fn timestamp_us(&self) -> u64 { self.raw.timestamp_us }
    #[inline] pub const fn kind(&self) -> Option<NativeEventKind> { NativeEventKind::from_u8_opt(self.raw.kind) }
    #[inline] pub const fn payload_tag(&self) -> u8 { self.raw.payload_tag }
    #[inline] pub fn name(&self) -> &'a str { unsafe { self.raw.name.as_str() }.unwrap_or("") }
    #[inline] pub const fn x(&self) -> f32 { self.raw.x }
    #[inline] pub const fn y(&self) -> f32 { self.raw.y }
    #[inline] pub const fn width(&self) -> f32 { self.raw.width }
    #[inline] pub const fn height(&self) -> f32 { self.raw.height }
    #[inline] pub fn text(&self) -> &'a str { unsafe { self.raw.text.as_str() }.unwrap_or("") }
    #[inline] pub const fn value(&self) -> NveValueRef<'a> { NveValueRef { raw: &self.raw.value } }
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
    #[inline] pub unsafe fn try_from_raw(batch: NveEventBatch) -> Option<Self> {
        Some(Self { events: unsafe { batch.try_as_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(batch: NveEventBatch) -> Self {
        unsafe { Self::try_from_raw(batch) }.unwrap_or(Self::EMPTY)
    }
    /// # Safety
    /// Caller must ensure all nested pointers inside `events` are valid for `'a`.
    #[inline] pub const unsafe fn from_slice(events: &'a [NveDirectEvent]) -> Self { Self { events } }
    #[inline] pub const fn len(&self) -> usize { self.events.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.events.is_empty() }
    #[inline] pub fn get(&self, index: usize) -> Option<NveDirectEventRef<'a>> {
        self.events.get(index).map(|raw| NveDirectEventRef { raw })
    }
    #[inline] pub fn iter(&self) -> impl ExactSizeIterator<Item = NveDirectEventRef<'a>> + 'a {
        self.events.iter().map(|raw| NveDirectEventRef { raw })
    }
}

/// Zero-copy borrowed view over an `NvePropSlice` (`field_id` -> `NveValueRef`).
#[derive(Clone, Copy)]
pub struct NvePropSliceRef<'a> {
    entries: &'a [NvePropEntry],
}

impl<'a> NvePropSliceRef<'a> {
    pub const EMPTY: Self = Self { entries: &[] };

    /// # Safety
    /// Caller must ensure `slice.entries` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn try_from_raw(slice: NvePropSlice) -> Option<Self> {
        Some(Self { entries: unsafe { slice.try_as_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `slice.entries` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(slice: NvePropSlice) -> Self {
        unsafe { Self::try_from_raw(slice) }.unwrap_or(Self::EMPTY)
    }
    /// # Safety
    /// All nested pointers inside `entries` must remain valid for `'a`.
    #[inline] pub const unsafe fn from_slice(entries: &'a [NvePropEntry]) -> Self { Self { entries } }
    #[inline] pub const fn len(&self) -> usize { self.entries.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.entries.is_empty() }
    /// Zero-allocation `O(n)` lookup by `field_id`.
    #[inline] pub fn get(&self, field_id: u16) -> Option<NveValueRef<'a>> {
        self.entries.iter().find_map(|e| (e.field_id == field_id).then_some(NveValueRef { raw: &e.value }))
    }
    #[inline] pub fn entry(&self, index: usize) -> Option<(u16, NveValueRef<'a>)> {
        self.entries.get(index).map(|e| (e.field_id, NveValueRef { raw: &e.value }))
    }
    #[inline] pub fn iter(&self) -> impl ExactSizeIterator<Item = (u16, NveValueRef<'a>)> + 'a {
        self.entries.iter().map(|e| (e.field_id, NveValueRef { raw: &e.value }))
    }
}

/// Zero-copy borrowed view over an `NveCommandSlice` (`opcode` + `NveValueRef` arguments).
#[derive(Clone, Copy)]
pub struct NveCommandSliceRef<'a> {
    opcode: u16,
    args: &'a [NveValue],
}

impl<'a> NveCommandSliceRef<'a> {
    /// # Safety
    /// Caller must ensure `cmd.args` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn try_from_raw(cmd: NveCommandSlice) -> Option<Self> {
        Some(Self { opcode: cmd.opcode, args: unsafe { cmd.try_args_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `cmd.args` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(cmd: NveCommandSlice) -> Self {
        unsafe { Self::try_from_raw(cmd) }.unwrap_or(Self { opcode: cmd.opcode, args: &[] })
    }
    /// # Safety
    /// All nested pointers inside `args` must remain valid for `'a`.
    #[inline] pub const unsafe fn from_parts(opcode: u16, args: &'a [NveValue]) -> Self { Self { opcode, args } }
    #[inline] pub const fn opcode(&self) -> u16 { self.opcode }
    #[inline] pub const fn len(&self) -> usize { self.args.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.args.is_empty() }
    #[inline] pub const fn args(&self) -> NveArrayRef<'a> { NveArrayRef { elems: self.args } }
    #[inline] pub fn arg(&self, index: usize) -> Option<NveValueRef<'a>> {
        self.args.get(index).map(|raw| NveValueRef { raw })
    }
    #[inline] pub fn iter(&self) -> impl ExactSizeIterator<Item = NveValueRef<'a>> + 'a {
        self.args.iter().map(|raw| NveValueRef { raw })
    }
}
