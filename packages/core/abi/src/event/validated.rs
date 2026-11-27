//! Single-pass ingress-validated [`ValidatedEvents`] and [`ValidatedEvent`] zero-copy views.

use core::fmt;
use core::mem::{align_of, size_of};

use crate::id::{ByteLen, EventFlags, EventId, NodeId, SurfaceId, TimestampUs};
use crate::table::{TableKey, Word};

use super::{
    EventEnum, InteractionEvent, LayoutEvent, NveEvent, NveEventBatch, ScrollEvent, TextInputEvent,
    NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE, NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION,
    NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};

/// Validation errors returned at [`ValidatedEvents`] FFI/wire ingress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventValidationError {
    NullEventsPointer,
    NullBytesPointer,
    EventsLengthOverflow,
    BytesLengthOverflow,
    SpanOutOfBounds {
        index: usize,
        offset: u32,
        len: u32,
        arena_len: usize,
    },
    InvalidUtf8 {
        index: usize,
    },
    InvalidInlineKeyLen {
        index: usize,
        len: u32,
    },
    InvalidSplitKeyLen {
        index: usize,
        key_len: u32,
        bytes_len: usize,
    },
    InvalidCustomNameLen {
        index: usize,
        name_len: u32,
        bytes_len: usize,
    },
    ConflictingFlags {
        index: usize,
    },
}

impl fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EventValidationError {}

#[derive(Clone, Copy)]
pub(super) enum EventSlice<'a> {
    Aligned(&'a [NveEvent]),
    Foreign(&'a [[u8; NVE_EVENT_SIZE]]),
}

impl<'a> EventSlice<'a> {
    #[inline(always)]
    const fn len(self) -> usize {
        match self {
            Self::Aligned(s) => s.len(),
            Self::Foreign(s) => s.len(),
        }
    }

    #[inline(always)]
    fn get(self, idx: usize) -> Option<RawEventRef<'a>> {
        match self {
            Self::Aligned(s) => s.get(idx).map(RawEventRef::Aligned),
            Self::Foreign(s) => s.get(idx).map(RawEventRef::Foreign),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum RawEventRef<'a> {
    Aligned(&'a NveEvent),
    Foreign(&'a [u8; NVE_EVENT_SIZE]),
}

impl<'a> RawEventRef<'a> {
    #[inline(always)]
    pub(super) fn read(self) -> NveEvent {
        match self {
            Self::Aligned(ev) => *ev,
            Self::Foreign(bytes) => {
                // SAFETY: `NveEvent` is `#[repr(C)]`, exactly 64 bytes, and every field accepts
                // all bit patterns.
                unsafe { core::ptr::read_unaligned(bytes.as_ptr().cast::<NveEvent>()) }
            }
        }
    }

    #[inline(always)]
    const fn as_bytes(self) -> &'a [u8; NVE_EVENT_SIZE] {
        match self {
            Self::Aligned(ev) => ev.as_bytes(),
            Self::Foreign(b) => b,
        }
    }
}

/// Validates one 64-byte event's span, framing flags, and UTF-8 invariants against `arena`.
pub(super) fn validate_one_event(
    index: usize,
    raw_ref: RawEventRef<'_>,
    arena: &[u8],
) -> Result<(), EventValidationError> {
    let ev = raw_ref.read();
    let off = ev.bytes.offset.as_usize();
    let len = ev.bytes.len.as_usize();
    let end = off
        .checked_add(len)
        .ok_or(EventValidationError::SpanOutOfBounds {
            index,
            offset: ev.bytes.offset.raw(),
            len: ev.bytes.len.raw(),
            arena_len: arena.len(),
        })?;
    let slice = arena
        .get(off..end)
        .ok_or(EventValidationError::SpanOutOfBounds {
            index,
            offset: ev.bytes.offset.raw(),
            len: ev.bytes.len.raw(),
            arena_len: arena.len(),
        })?;

    let flags = ev.flags;
    let has_utf8 = flags.contains(EventFlags::UTF8_BYTES);
    let has_inline_key = flags.contains(EventFlags::INLINE_KEY_UTF8);
    let has_split_key = flags.contains(EventFlags::KEY_SPLIT_BYTES);
    let has_custom = flags.contains(EventFlags::CUSTOM_NAMED);

    if (has_inline_key && has_split_key) || (has_custom && (has_inline_key || has_split_key)) {
        return Err(EventValidationError::ConflictingFlags { index });
    }
    if has_utf8 && core::str::from_utf8(slice).is_err() {
        return Err(EventValidationError::InvalidUtf8 { index });
    }
    if has_inline_key {
        let klen = ev.payload[1];
        if klen as usize > NVE_EVENT_INLINE_KEY_CAP {
            return Err(EventValidationError::InvalidInlineKeyLen { index, len: klen });
        }
        let inline_slice = &raw_ref.as_bytes()[40..40 + klen as usize];
        if core::str::from_utf8(inline_slice).is_err() {
            return Err(EventValidationError::InvalidUtf8 { index });
        }
    }
    if has_split_key {
        let klen = ev.payload[1] as usize;
        if klen > slice.len() {
            return Err(EventValidationError::InvalidSplitKeyLen {
                index,
                key_len: ev.payload[1],
                bytes_len: slice.len(),
            });
        }
        if has_utf8 {
            // SAFETY: `slice` was validated as UTF-8 under `has_utf8` above.
            let full = unsafe { core::str::from_utf8_unchecked(slice) };
            if !full.is_char_boundary(klen) {
                return Err(EventValidationError::InvalidUtf8 { index });
            }
        } else if core::str::from_utf8(&slice[..klen]).is_err()
            || core::str::from_utf8(&slice[klen..]).is_err()
        {
            return Err(EventValidationError::InvalidUtf8 { index });
        }
    }
    if has_custom {
        let nlen = ev.payload[0] as usize;
        if nlen > slice.len() {
            return Err(EventValidationError::InvalidCustomNameLen {
                index,
                name_len: ev.payload[0],
                bytes_len: slice.len(),
            });
        }
        if core::str::from_utf8(&slice[..nlen]).is_err() {
            return Err(EventValidationError::InvalidUtf8 { index });
        }
    }
    Ok(())
}

/// Ingress-validated zero-copy view over an [`NveEventBatch`].
#[derive(Clone, Copy)]
pub struct ValidatedEvents<'a> {
    events: EventSlice<'a>,
    bytes: &'a [u8],
}

impl<'a> ValidatedEvents<'a> {
    pub const EMPTY: Self = Self {
        events: EventSlice::Aligned(&[]),
        bytes: &[],
    };

    #[inline(always)]
    pub(super) const fn from_sealed_aligned(events: &'a [NveEvent], bytes: &'a [u8]) -> Self {
        Self {
            events: EventSlice::Aligned(events),
            bytes,
        }
    }

    /// Validates a Rust slice pair `(events, bytes)` in a single linear pass.
    pub fn validate(events: &'a [NveEvent], bytes: &'a [u8]) -> Result<Self, EventValidationError> {
        for (idx, ev) in events.iter().enumerate() {
            validate_one_event(idx, RawEventRef::Aligned(ev), bytes)?;
        }
        Ok(Self {
            events: EventSlice::Aligned(events),
            bytes,
        })
    }

    /// Validates an FFI [`NveEventBatch`] in a single linear pass, accepting both 64-byte-aligned
    /// Rust slabs and naturally-aligned foreign C/Swift/Dart 64-byte-stride arrays.
    ///
    /// # Safety
    /// When `batch.event_count > 0`, `batch.events` must point to `batch.event_count` initialized
    /// 64-byte `NveEvent` records valid and immutable for `'a`. When `batch.bytes_len > 0`,
    /// `batch.bytes` must point to `batch.bytes_len` initialized bytes valid and immutable for `'a`.
    pub unsafe fn try_from_raw(batch: NveEventBatch) -> Result<Self, EventValidationError> {
        let ev_count = batch.event_count as usize;
        let bytes_len = batch.bytes_len.as_usize();

        let bytes = if bytes_len == 0 {
            &[]
        } else {
            if batch.bytes.is_null() {
                return Err(EventValidationError::NullBytesPointer);
            }
            if bytes_len > isize::MAX as usize {
                return Err(EventValidationError::BytesLengthOverflow);
            }
            // SAFETY: Non-null and `<= isize::MAX` checked; caller guarantees valid readable bytes.
            unsafe { core::slice::from_raw_parts(batch.bytes, bytes_len) }
        };

        let events = if ev_count == 0 {
            EventSlice::Aligned(&[])
        } else {
            if batch.events.is_null() {
                return Err(EventValidationError::NullEventsPointer);
            }
            let total_bytes = ev_count
                .checked_mul(size_of::<NveEvent>())
                .ok_or(EventValidationError::EventsLengthOverflow)?;
            if total_bytes > isize::MAX as usize {
                return Err(EventValidationError::EventsLengthOverflow);
            }
            if (batch.events as usize).is_multiple_of(align_of::<NveEvent>()) {
                // SAFETY: Non-null, 64-byte aligned, and `<= isize::MAX` verified; caller guarantees initialized records.
                EventSlice::Aligned(unsafe { core::slice::from_raw_parts(batch.events, ev_count) })
            } else {
                // SAFETY: Non-null and `<= isize::MAX` verified; `[u8; 64]` has alignment 1 and exact stride 64.
                EventSlice::Foreign(unsafe {
                    core::slice::from_raw_parts(
                        batch.events.cast::<[u8; NVE_EVENT_SIZE]>(),
                        ev_count,
                    )
                })
            }
        };

        for idx in 0..events.len() {
            if let Some(raw_ref) = events.get(idx) {
                validate_one_event(idx, raw_ref, bytes)?;
            }
        }
        Ok(Self { events, bytes })
    }

    /// Validates `batch` at an `extern "C"` boundary, returning [`Self::EMPTY`] on malformed input.
    ///
    /// # Safety
    /// Same pointer-validity requirements as [`Self::try_from_raw`].
    #[inline]
    pub unsafe fn from_raw(batch: NveEventBatch) -> Self {
        // SAFETY: Forwarded directly to `try_from_raw`.
        unsafe { Self::try_from_raw(batch) }.unwrap_or(Self::EMPTY)
    }

    #[inline(always)]
    pub const fn as_aligned_slice(&self) -> Option<&'a [NveEvent]> {
        match self.events {
            EventSlice::Aligned(s) => Some(s),
            EventSlice::Foreign(_) => None,
        }
    }

    #[inline(always)]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[inline]
    pub const fn as_raw_batch(&self) -> NveEventBatch {
        let (ptr, count) = match self.events {
            EventSlice::Aligned(s) => (s.as_ptr(), s.len()),
            EventSlice::Foreign(s) => (s.as_ptr().cast::<NveEvent>(), s.len()),
        };
        NveEventBatch {
            events: ptr,
            bytes: self.bytes.as_ptr(),
            event_count: if count > u32::MAX as usize {
                u32::MAX
            } else {
                count as u32
            },
            bytes_len: ByteLen(if self.bytes.len() > u32::MAX as usize {
                u32::MAX
            } else {
                self.bytes.len() as u32
            }),
        }
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.events.len()
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.events.len() == 0
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<ValidatedEvent<'a>> {
        let raw_ref = self.events.get(index)?;
        let ev = raw_ref.read();
        let off = ev.bytes.offset.as_usize();
        let len = ev.bytes.len.as_usize();
        // SAFETY: `off..off+len` was validated against `self.bytes.len()` at `ValidatedEvents` ingress.
        let slice = unsafe { self.bytes.get_unchecked(off..off + len) };
        Some(ValidatedEvent {
            raw: ev,
            raw_ref,
            bytes: slice,
        })
    }

    #[inline]
    pub const fn iter(&self) -> ValidatedEventsIter<'a> {
        ValidatedEventsIter {
            batch: *self,
            index: 0,
        }
    }
}

impl fmt::Debug for ValidatedEvents<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedEvents")
            .field("len", &self.len())
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}

impl<'a> IntoIterator for ValidatedEvents<'a> {
    type Item = ValidatedEvent<'a>;
    type IntoIter = ValidatedEventsIter<'a>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over [`ValidatedEvent`] records inside [`ValidatedEvents`].
#[derive(Clone, Copy)]
pub struct ValidatedEventsIter<'a> {
    batch: ValidatedEvents<'a>,
    index: usize,
}

impl<'a> Iterator for ValidatedEventsIter<'a> {
    type Item = ValidatedEvent<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let ev = self.batch.get(self.index)?;
        self.index += 1;
        Some(ev)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.batch.len().saturating_sub(self.index);
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for ValidatedEventsIter<'_> {}

/// One pre-validated 64-byte event borrowing its companion slice from [`ValidatedEvents`].
#[derive(Clone, Copy)]
pub struct ValidatedEvent<'a> {
    raw: NveEvent,
    raw_ref: RawEventRef<'a>,
    bytes: &'a [u8],
}

impl<'a> ValidatedEvent<'a> {
    #[inline(always)] pub const fn raw(&self) -> NveEvent { self.raw }
    #[inline(always)] pub const fn timestamp(&self) -> TimestampUs { self.raw.timestamp }
    #[inline(always)] pub const fn table(&self) -> TableKey { self.raw.table }
    #[inline(always)] pub const fn surface(&self) -> SurfaceId { SurfaceId::from_raw_or_null(self.raw.surface.raw()) }
    #[inline(always)] pub const fn node(&self) -> NodeId { NodeId::from_raw_or_null(self.raw.node.raw()) }
    #[inline(always)] pub const fn event(&self) -> EventId { self.raw.event }
    #[inline(always)] pub const fn flags(&self) -> EventFlags { self.raw.flags }
    #[inline(always)] pub const fn payload(&self) -> [Word; 6] { self.raw.payload }
    #[inline(always)] pub const fn bytes(&self) -> &'a [u8] { self.bytes }

    #[inline(always)]
    pub fn as_event<E: EventEnum>(&self, table: TableKey) -> Option<E> {
        if self.raw.table == table { E::from_event_id(self.raw.event) } else { None }
    }

    #[inline(always)] pub fn interaction(&self) -> Option<InteractionEvent> { self.as_event(NVE_TABLE_INTERACTION) }
    #[inline(always)] pub fn text_input(&self) -> Option<TextInputEvent> { self.as_event(NVE_TABLE_TEXT_INPUT) }
    #[inline(always)] pub fn scroll_event(&self) -> Option<ScrollEvent> { self.as_event(NVE_TABLE_SCROLL) }
    #[inline(always)] pub fn layout_event(&self) -> Option<LayoutEvent> { self.as_event(NVE_TABLE_LAYOUT) }
    #[inline(always)] pub const fn is_custom(&self) -> bool { self.raw.table.0 == NVE_TABLE_CUSTOM.0 && self.raw.flags.contains(EventFlags::CUSTOM_NAMED) }

    #[inline(always)] pub const fn payload_f32(&self, idx: usize) -> f32 { self.raw.payload_f32(idx) }
    #[inline(always)] pub const fn payload_u32(&self, idx: usize) -> u32 { self.raw.payload_u32(idx) }
    #[inline(always)] pub const fn payload_u64(&self, idx: usize) -> u64 { self.raw.payload_u64(idx) }
    #[inline(always)] pub const fn payload_f64(&self, idx: usize) -> f64 { self.raw.payload_f64(idx) }
    #[inline(always)] pub const fn x(&self) -> f32 { self.raw.payload_f32(0) }
    #[inline(always)] pub const fn y(&self) -> f32 { self.raw.payload_f32(1) }
    #[inline(always)] pub const fn delta_x(&self) -> f32 { self.raw.payload_f32(0) }
    #[inline(always)] pub const fn delta_y(&self) -> f32 { self.raw.payload_f32(1) }
    #[inline(always)] pub const fn width(&self) -> f32 { self.raw.payload_f32(2) }
    #[inline(always)] pub const fn height(&self) -> f32 { self.raw.payload_f32(3) }
    #[inline(always)] pub const fn pointer_id(&self) -> u32 { self.raw.payload_u32(2) }
    #[inline(always)] pub const fn key_code(&self) -> u32 { self.raw.payload_u32(0) }

    #[inline(always)] pub const fn meta(&self) -> bool { self.raw.flags.meta() }
    #[inline(always)] pub const fn ctrl(&self) -> bool { self.raw.flags.ctrl() }
    #[inline(always)] pub const fn shift(&self) -> bool { self.raw.flags.shift() }
    #[inline(always)] pub const fn alt(&self) -> bool { self.raw.flags.alt() }
    #[inline(always)] pub const fn repeat(&self) -> bool { self.raw.flags.repeat() }
    #[inline(always)] pub const fn has_primary_modifier(&self) -> bool { self.raw.flags.has_primary_modifier() }
    #[inline(always)] pub const fn pointer_button(&self) -> u8 { self.raw.flags.pointer_button() }
    #[inline(always)] pub const fn is_capture(&self) -> bool { self.raw.flags.is_capture() }
    #[inline(always)] pub const fn is_composing(&self) -> bool { self.raw.flags.is_composing() }

    /// Zero-alloc pre-validated UTF-8 text payload.
    #[inline(always)]
    pub fn text(&self) -> &'a str {
        if self.raw.flags.contains(EventFlags::UTF8_BYTES)
            && !self.raw.flags.contains(EventFlags::KEY_SPLIT_BYTES)
        {
            // SAFETY: `EventFlags::UTF8_BYTES` was validated on `self.bytes` at ingress.
            unsafe { core::str::from_utf8_unchecked(self.bytes) }
        } else {
            ""
        }
    }

    /// Zero-alloc W3C key string (`<= 16 B` inline fast path or `KEY_SPLIT_BYTES` arena).
    #[inline(always)]
    pub fn key(&self) -> &'a str {
        if self.raw.flags.contains(EventFlags::INLINE_KEY_UTF8) {
            let len = self.raw.payload[1] as usize;
            // SAFETY: `len <= 16` and UTF-8 validity of `raw_bytes[40..40+len]` were verified at ingress.
            unsafe { core::str::from_utf8_unchecked(&self.raw_ref.as_bytes()[40..40 + len]) }
        } else if self.raw.flags.contains(EventFlags::KEY_SPLIT_BYTES) {
            let klen = self.raw.payload[1] as usize;
            // SAFETY: `klen <= self.bytes.len()` and UTF-8 validity of `self.bytes[..klen]` were verified at ingress.
            unsafe { core::str::from_utf8_unchecked(self.bytes.get_unchecked(..klen)) }
        } else {
            ""
        }
    }

    /// Zero-alloc physical key code string.
    #[inline(always)]
    pub fn code(&self) -> &'a str {
        if self.raw.flags.contains(EventFlags::INLINE_KEY_UTF8)
            && self.raw.flags.contains(EventFlags::UTF8_BYTES)
        {
            // SAFETY: `EventFlags::UTF8_BYTES` was validated on `self.bytes` at ingress.
            unsafe { core::str::from_utf8_unchecked(self.bytes) }
        } else if self.raw.flags.contains(EventFlags::KEY_SPLIT_BYTES) {
            let klen = self.raw.payload[1] as usize;
            // SAFETY: `klen <= self.bytes.len()` and UTF-8 validity of `self.bytes[klen..]` were verified at ingress.
            unsafe { core::str::from_utf8_unchecked(self.bytes.get_unchecked(klen..)) }
        } else {
            ""
        }
    }

    /// Zero-alloc custom event name (`NVE_TABLE_CUSTOM` + `EventFlags::CUSTOM_NAMED`).
    #[inline(always)]
    pub fn custom_name(&self) -> &'a str {
        if self.raw.flags.contains(EventFlags::CUSTOM_NAMED) {
            let nlen = self.raw.payload[0] as usize;
            // SAFETY: `nlen <= self.bytes.len()` and UTF-8 validity of `self.bytes[..nlen]` were verified at ingress.
            unsafe { core::str::from_utf8_unchecked(self.bytes.get_unchecked(..nlen)) }
        } else {
            ""
        }
    }

    /// Raw custom event payload bytes following the name prefix.
    #[inline(always)]
    pub fn custom_data(&self) -> &'a [u8] {
        if self.raw.flags.contains(EventFlags::CUSTOM_NAMED) {
            let nlen = self.raw.payload[0] as usize;
            // SAFETY: `nlen <= self.bytes.len()` was verified at ingress.
            unsafe { self.bytes.get_unchecked(nlen..) }
        } else {
            self.bytes
        }
    }
}

impl fmt::Debug for ValidatedEvent<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedEvent")
            .field("table", &self.raw.table)
            .field("surface", &self.surface())
            .field("node", &self.node())
            .field("event", &self.raw.event)
            .field("flags", &self.raw.flags)
            .field("bytes_len", &self.bytes.len())
            .finish()
    }
}
