use crate::NativeEventKind;

use super::ffi_events::{
    NveDirectEvent, NveEventBatch, NVE_EVENT_FLAG_BUTTON_MASK, NVE_EVENT_FLAG_CAPTURE,
    NVE_EVENT_FLAG_COMPOSING, NVE_KEY_MOD_ALT, NVE_KEY_MOD_CTRL, NVE_KEY_MOD_META,
    NVE_KEY_MOD_REPEAT, NVE_KEY_MOD_SHIFT,
};
use super::ffi_types::{
    guarded_slice, NveCommandSlice, NveMapEntry, NvePropEntry, NvePropSlice, NveValue, NveValueTag,
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
        if self.raw.tag == NveValueTag::Bool as u32 {
            // SAFETY: Tag checked equal to `Bool`, selecting the active union field.
            Some(unsafe { self.raw.data.bool_val } != 0)
        } else {
            None
        }
    }
    #[inline] pub const fn as_i64(&self) -> Option<i64> {
        if self.raw.tag == NveValueTag::I64 as u32 {
            // SAFETY: Tag checked equal to `I64`, selecting the active union field.
            Some(unsafe { self.raw.data.i64_val })
        } else {
            None
        }
    }
    #[inline] pub const fn as_f64(&self) -> Option<f64> {
        if self.raw.tag == NveValueTag::F64 as u32 {
            // SAFETY: Tag checked equal to `F64`, selecting the active union field.
            Some(unsafe { self.raw.data.f64_val })
        } else {
            None
        }
    }
    /// Returns numeric value as `f64` for either `F64` or `I64` tags.
    #[inline] pub const fn as_number(&self) -> Option<f64> {
        match self.tag() {
            // SAFETY: Tag matched as `F64`, selecting the active union field.
            Some(NveValueTag::F64) => Some(unsafe { self.raw.data.f64_val }),
            // SAFETY: Tag matched as `I64`, selecting the active union field.
            Some(NveValueTag::I64) => Some(unsafe { self.raw.data.i64_val } as f64),
            _ => None,
        }
    }
    #[inline] pub fn as_str(&self) -> Option<&'a str> {
        (self.raw.tag == NveValueTag::Str as u32)
            .then(|| {
                // SAFETY: Tag checked equal to `Str` and `NveValueRef` guarantees valid nested pointers for `'a`.
                unsafe { self.raw.data.str_val.as_str() }.ok()
            })
            .flatten()
    }
    /// Like [`Self::as_str`], but says why it fails: `UnexpectedTag`, `Truncated` (a null or
    /// oversized pointer), or `InvalidUtf8`.
    #[inline] pub fn try_as_str(&self) -> Result<&'a str, crate::DecodeError> {
        if self.raw.tag != NveValueTag::Str as u32 {
            return Err(crate::DecodeError::UnexpectedTag { expected: NveValueTag::Str, found: self.raw.tag });
        }
        // SAFETY: Tag checked equal to `Str` and `NveValueRef` guarantees valid nested pointers for `'a`.
        unsafe { self.raw.data.str_val.as_str() }
    }
    #[inline] pub fn as_bytes(&self) -> Option<&'a [u8]> {
        (self.raw.tag == NveValueTag::Bytes as u32).then(|| {
            // SAFETY: Tag checked equal to `Bytes` and `NveValueRef` guarantees valid nested pointers for `'a`.
            unsafe {
                let s = self.raw.data.bytes_val;
                guarded_slice(s.ptr, s.len)
            }
        })?
    }
    #[inline] pub fn as_array(&self) -> Option<NveArrayRef<'a>> {
        (self.raw.tag == NveValueTag::Array as u32).then(|| {
            // SAFETY: Tag checked equal to `Array` and `NveValueRef` guarantees valid nested pointers for `'a`.
            unsafe {
                let a = self.raw.data.array_val;
                guarded_slice(a.ptr, a.len)
            }
            .map(|elems| NveArrayRef { elems })
        })?
    }
    #[inline] pub fn as_map(&self) -> Option<NveMapRef<'a>> {
        (self.raw.tag == NveValueTag::Map as u32).then(|| {
            // SAFETY: Tag checked equal to `Map` and `NveValueRef` guarantees valid nested pointers for `'a`.
            unsafe {
                let m = self.raw.data.map_val;
                guarded_slice(m.ptr, m.len)
            }
            .map(|entries| NveMapRef { entries })
        })?
    }
    /// Zero-copy key lookup when this value is a map.
    #[inline] pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> { self.as_map()?.get(key) }
}

const MAX_DEBUG_DEPTH: usize = 16;
const MAX_DEBUG_BYTES_PREVIEW: usize = 64;
const MAX_DEBUG_STR_PREVIEW: usize = 64;
const MAX_DEBUG_CONTAINER_ITEMS: usize = 32;
const MAX_DEBUG_TOTAL_ITEMS: usize = 256;

/// Formats the value for diagnostics, capping recursion depth (16), total visited items (256),
/// per-container entries (32), and string/byte previews (64 bytes) so deeply nested, cyclic, or
/// wide FFI trees and huge buffers never overflow the stack, blow up exponentially, or flood logs;
/// an invalid pointer or key prints as `Invalid`.
impl std::fmt::Debug for NveValueRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut budget = MAX_DEBUG_TOTAL_ITEMS;
        self.fmt_bounded(f, 0, &mut budget)
    }
}

impl NveValueRef<'_> {
    fn fmt_bounded(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        depth: usize,
        budget: &mut usize,
    ) -> std::fmt::Result {
        struct Invalid;
        impl std::fmt::Debug for Invalid {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Invalid")
            }
        }
        struct More(usize);
        impl std::fmt::Debug for More {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, ".. (+{} more)", self.0)
            }
        }
        struct KvEntry<K, V>(K, V);
        impl<K: std::fmt::Debug, V: std::fmt::Debug> std::fmt::Debug for KvEntry<K, V> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)?;
                f.write_str(": ")?;
                self.1.fmt(f)
            }
        }
        struct Bounded<'a>(NveValueRef<'a>, usize, &'a std::cell::Cell<usize>);
        impl std::fmt::Debug for Bounded<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut rem = self.2.get();
                let res = self.0.fmt_bounded(f, self.1, &mut rem);
                self.2.set(rem);
                res
            }
        }
        struct StrPreview<'a>(&'a str);
        impl std::fmt::Debug for StrPreview<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if self.0.len() <= MAX_DEBUG_STR_PREVIEW {
                    self.0.fmt(f)
                } else {
                    let mut end = MAX_DEBUG_STR_PREVIEW;
                    while !self.0.is_char_boundary(end) {
                        end -= 1;
                    }
                    write!(f, "{:?}.. ({} bytes)", &self.0[..end], self.0.len())
                }
            }
        }
        struct BytesPreview<'a>(&'a [u8]);
        impl std::fmt::Debug for BytesPreview<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                if self.0.len() <= MAX_DEBUG_BYTES_PREVIEW {
                    self.0.fmt(f)
                } else {
                    write!(f, "{:?}.. ({} bytes)", &self.0[..MAX_DEBUG_BYTES_PREVIEW], self.0.len())
                }
            }
        }
        if *budget == 0 {
            return f.write_str("...");
        }
        *budget -= 1;
        match self.tag() {
            Some(NveValueTag::Null) => f.write_str("Null"),
            Some(NveValueTag::Bool) => f.debug_tuple("Bool").field(&self.as_bool()).finish(),
            Some(NveValueTag::I64) => f.debug_tuple("I64").field(&self.as_i64()).finish(),
            Some(NveValueTag::F64) => f.debug_tuple("F64").field(&self.as_f64()).finish(),
            Some(NveValueTag::Str) => {
                f.debug_tuple("Str").field(&self.try_as_str().map(StrPreview)).finish()
            }
            Some(NveValueTag::Bytes) => {
                f.debug_tuple("Bytes").field(&self.as_bytes().map(BytesPreview)).finish()
            }
            Some(NveValueTag::Array) => {
                if depth >= MAX_DEBUG_DEPTH {
                    return f.write_str("[...]");
                }
                match self.as_array() {
                    Some(a) => {
                        let cell = std::cell::Cell::new(*budget);
                        let mut list = f.debug_list();
                        let mut shown = 0usize;
                        for v in a.iter() {
                            if shown >= MAX_DEBUG_CONTAINER_ITEMS || cell.get() == 0 {
                                break;
                            }
                            list.entry(&Bounded(v, depth + 1, &cell));
                            shown += 1;
                        }
                        if let Some(omitted) = a.len().checked_sub(shown).filter(|&n| n > 0) {
                            list.entry(&More(omitted));
                        }
                        let res = list.finish();
                        *budget = cell.get();
                        res
                    }
                    None => f.write_str("Invalid"),
                }
            }
            Some(NveValueTag::Map) => {
                if depth >= MAX_DEBUG_DEPTH {
                    return f.write_str("{...}");
                }
                match self.as_map() {
                    Some(m) => {
                        let cell = std::cell::Cell::new(*budget);
                        let mut set = f.debug_set();
                        let mut shown = 0usize;
                        for e in m.try_iter() {
                            if shown >= MAX_DEBUG_CONTAINER_ITEMS || cell.get() == 0 {
                                break;
                            }
                            match e {
                                Ok((k, v)) => {
                                    set.entry(&KvEntry(
                                        Ok::<_, Invalid>(StrPreview(k)),
                                        Some(Bounded(v, depth + 1, &cell)),
                                    ));
                                }
                                Err(_) => {
                                    cell.set(cell.get().saturating_sub(1));
                                    set.entry(&KvEntry(
                                        Err::<StrPreview<'_>, _>(Invalid),
                                        Option::<Bounded<'_>>::None,
                                    ));
                                }
                            }
                            shown += 1;
                        }
                        if let Some(omitted) = m.len().checked_sub(shown).filter(|&n| n > 0) {
                            set.entry(&More(omitted));
                        }
                        let res = set.finish();
                        *budget = cell.get();
                        res
                    }
                    None => f.write_str("Invalid"),
                }
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
    pub(super) entries: &'a [NveMapEntry],
}

pub type NvePropsRef<'a> = NveMapRef<'a>;

impl<'a> NveMapRef<'a> {
    pub const EMPTY: Self = Self { entries: &[] };

    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items and nested pointers for `'a`.
    #[inline] pub unsafe fn try_from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Option<Self> {
        // SAFETY: Caller guarantees `ptr` is valid for `count` `NveMapEntry` items for `'a`.
        Some(Self { entries: unsafe { guarded_slice(ptr, count as usize) }? })
    }
    /// # Safety
    /// `ptr` must be valid for `count` initialized `NveMapEntry` items and nested pointers for `'a`.
    #[inline] pub unsafe fn from_raw_parts(ptr: *const NveMapEntry, count: u32) -> Self {
        // SAFETY: Caller guarantees `ptr` is valid for `count` `NveMapEntry` items for `'a`.
        unsafe { Self::try_from_raw_parts(ptr, count) }.unwrap_or(Self::EMPTY)
    }
    /// # Safety
    /// All nested pointers inside `entries` must remain valid for `'a`.
    #[inline] pub const unsafe fn from_slice(entries: &'a [NveMapEntry]) -> Self { Self { entries } }
    #[inline] pub const fn len(&self) -> usize { self.entries.len() }
    #[inline] pub const fn is_empty(&self) -> bool { self.entries.is_empty() }
    /// Raw entries, e.g. to forward across `NveComponentApi::set_props` unchanged.
    #[inline] pub const fn entries(&self) -> &'a [NveMapEntry] { self.entries }
    #[inline] pub fn get(&self, key: &str) -> Option<NveValueRef<'a>> {
        let key_bytes = key.as_bytes();
        self.entries.iter().find_map(|entry| {
            // SAFETY: `NveMapRef` guarantees entry key pointers remain valid for `'a`.
            (unsafe { entry.key.try_as_bytes() } == Some(key_bytes))
                .then_some(NveValueRef { raw: &entry.value })
        })
    }
    #[inline] pub fn iter(&self) -> impl Iterator<Item = (&'a str, NveValueRef<'a>)> + 'a {
        self.entries.iter().filter_map(|entry| {
            // SAFETY: `NveMapRef` guarantees entry key pointers remain valid for `'a`.
            Some((unsafe { entry.key.as_str() }.ok()?, NveValueRef { raw: &entry.value }))
        })
    }
    /// Like [`Self::iter`], but a key that is not valid UTF-8 (or has an invalid pointer) is an
    /// error instead of being skipped.
    #[inline] pub fn try_iter(&self) -> impl Iterator<Item = Result<(&'a str, NveValueRef<'a>), crate::DecodeError>> + 'a {
        self.entries.iter().map(|entry| {
            // SAFETY: `NveMapRef` guarantees entry key pointers remain valid for `'a`.
            Ok((unsafe { entry.key.as_str() }?, NveValueRef { raw: &entry.value }))
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
    #[inline] pub const fn modifiers(&self) -> u8 { self.raw.modifiers }
    #[inline] pub const fn event_flags(&self) -> u8 { self.raw.event_flags }
    #[inline] pub const fn key_code(&self) -> u32 { self.raw.key_code }
    #[inline] pub const fn meta(&self) -> bool { (self.raw.modifiers & NVE_KEY_MOD_META) != 0 }
    #[inline] pub const fn ctrl(&self) -> bool { (self.raw.modifiers & NVE_KEY_MOD_CTRL) != 0 }
    #[inline] pub const fn shift(&self) -> bool { (self.raw.modifiers & NVE_KEY_MOD_SHIFT) != 0 }
    #[inline] pub const fn alt(&self) -> bool { (self.raw.modifiers & NVE_KEY_MOD_ALT) != 0 }
    #[inline] pub const fn repeat(&self) -> bool { (self.raw.modifiers & NVE_KEY_MOD_REPEAT) != 0 }
    #[inline] pub const fn has_primary_modifier(&self) -> bool { self.meta() || self.ctrl() }
    #[inline] pub const fn pointer_button(&self) -> u8 { self.raw.event_flags & NVE_EVENT_FLAG_BUTTON_MASK }
    #[inline] pub const fn is_capture(&self) -> bool { (self.raw.event_flags & NVE_EVENT_FLAG_CAPTURE) != 0 }
    #[inline] pub const fn is_composing(&self) -> bool { (self.raw.event_flags & NVE_EVENT_FLAG_COMPOSING) != 0 }
    #[inline] pub fn name(&self) -> &'a str {
        // SAFETY: `NveDirectEventRef` guarantees `raw.name` pointer remains valid for `'a`.
        unsafe { self.raw.name.as_str() }.unwrap_or("")
    }
    #[inline] pub fn code(&self) -> &'a str { self.name() }
    #[inline] pub const fn x(&self) -> f32 { self.raw.x }
    #[inline] pub const fn y(&self) -> f32 { self.raw.y }
    #[inline] pub const fn delta_x(&self) -> f32 { self.raw.x }
    #[inline] pub const fn delta_y(&self) -> f32 { self.raw.y }
    #[inline] pub const fn width(&self) -> f32 { self.raw.width }
    #[inline] pub const fn height(&self) -> f32 { self.raw.height }
    #[inline] pub fn text(&self) -> &'a str {
        // SAFETY: `NveDirectEventRef` guarantees `raw.text` pointer remains valid for `'a`.
        unsafe { self.raw.text.as_str() }.unwrap_or("")
    }
    #[inline] pub fn key(&self) -> &'a str { self.text() }
    #[inline] pub const fn value(&self) -> NveValueRef<'a> { NveValueRef { raw: &self.raw.value } }
    /// Table identity key for `NativeEventKind::TableEvent`.
    #[inline]
    pub fn table_key(&self) -> crate::Key64 {
        crate::Key64(self.value().as_i64().unwrap_or(0) as u64)
    }
    /// Table-scoped event ID for `NativeEventKind::TableEvent`.
    #[inline]
    pub const fn table_event_id(&self) -> u32 {
        self.raw.key_code
    }
    /// Raw payload bytes for `NativeEventKind::TableEvent`.
    #[inline]
    pub fn table_payload(&self) -> &'a [u8] {
        // SAFETY: `NveDirectEventRef` guarantees `raw.text` pointer remains valid for `'a`.
        unsafe { self.raw.text.as_bytes() }
    }
}

/// Zero-copy borrowed batch of `NveDirectEvent`s passed into `NveApp::dispatch_events`.
#[derive(Clone, Copy)]
pub struct NveEventBatchRef<'a> {
    events: &'a [NveDirectEvent],
}

impl<'a> NveEventBatchRef<'a> {
    pub const EMPTY: Self = Self { events: &[] };

    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn try_from_raw(batch: NveEventBatch) -> Option<Self> {
        // SAFETY: Caller guarantees `batch.ptr` and nested pointers are valid for `'a`.
        Some(Self { events: unsafe { batch.try_as_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `batch.ptr` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(batch: NveEventBatch) -> Self {
        // SAFETY: Caller guarantees `batch.ptr` and nested pointers are valid for `'a`.
        unsafe { Self::try_from_raw(batch) }.unwrap_or(Self::EMPTY)
    }
    /// # Safety
    /// Caller must ensure all nested pointers inside `events` are valid for `'a`.
    #[inline] pub const unsafe fn from_slice(events: &'a [NveDirectEvent]) -> Self { Self { events } }
    #[inline] pub const fn as_slice(&self) -> &'a [NveDirectEvent] { self.events }
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
        // SAFETY: Caller guarantees `slice.entries` and nested pointers are valid for `'a`.
        Some(Self { entries: unsafe { slice.try_as_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `slice.entries` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(slice: NvePropSlice) -> Self {
        // SAFETY: Caller guarantees `slice.entries` and nested pointers are valid for `'a`.
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
        // SAFETY: Caller guarantees `cmd.args` and nested pointers are valid for `'a`.
        Some(Self { opcode: cmd.opcode, args: unsafe { cmd.try_args_slice() }? })
    }
    /// # Safety
    /// Caller must ensure `cmd.args` and all nested pointers are valid for `'a`.
    #[inline] pub unsafe fn from_raw(cmd: NveCommandSlice) -> Self {
        // SAFETY: Caller guarantees `cmd.args` and nested pointers are valid for `'a`.
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
