use crate::NativeEventKind;

use super::ffi_types::{
    guarded_slice, NveDirectEvent, NveEventBatch, NveMapEntry, NveValue, NveValueTag,
};

/// Zero-copy, zero-allocation borrowed view of an FFI `NveValue`.
#[derive(Clone, Copy)]
pub struct NveValueRef<'a> {
    pub(super) raw: &'a NveValue,
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
            unsafe { guarded_slice(slice.ptr, slice.len) }
        } else {
            None
        }
    }

    #[inline]
    pub fn as_array(&self) -> Option<NveArrayRef<'a>> {
        if self.raw.tag == NveValueTag::Array as u32 {
            let arr = unsafe { self.raw.data.array_val };
            let slice = unsafe { guarded_slice(arr.ptr, arr.len) }?;
            Some(NveArrayRef { elems: slice })
        } else {
            None
        }
    }

    #[inline]
    pub fn as_map(&self) -> Option<NveMapRef<'a>> {
        if self.raw.tag == NveValueTag::Map as u32 {
            let map = unsafe { self.raw.data.map_val };
            let slice = unsafe { guarded_slice(map.ptr, map.len) }?;
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
    pub(super) entries: &'a [NveMapEntry],
}

pub type NvePropsRef<'a> = NveMapRef<'a>;

impl<'a> NveMapRef<'a> {
    pub const EMPTY: Self = Self { entries: &[] };

    /// Validated map view returning `None` on null pointer with `count > 0`, misaligned pointer,
    /// or byte-size overflow (`> isize::MAX`).
    ///
    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items (and all nested pointers)
    /// for lifetime `'a`.
    #[inline]
    pub unsafe fn try_from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Option<Self> {
        let entries = unsafe { guarded_slice(ptr, count as usize) }?;
        Some(Self { entries })
    }

    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items (and all nested pointers)
    /// for lifetime `'a`.
    #[inline]
    pub unsafe fn from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Self {
        unsafe { Self::try_from_raw_parts(ptr, count) }.unwrap_or(Self::EMPTY)
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
            if unsafe { entry.key.try_as_bytes() } == Some(key_bytes) {
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

    /// Validated batch view returning `None` on null pointer with `len > 0`, misaligned pointer,
    /// or byte-size overflow.
    ///
    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline]
    pub unsafe fn try_from_raw(batch: NveEventBatch) -> Option<Self> {
        let events = unsafe { batch.try_as_slice() }?;
        Some(Self { events })
    }

    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline]
    pub unsafe fn from_raw(batch: NveEventBatch) -> Self {
        unsafe { Self::try_from_raw(batch) }.unwrap_or(Self::EMPTY)
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
