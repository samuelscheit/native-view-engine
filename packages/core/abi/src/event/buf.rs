//! Reusable owned [`EventBatchBuf`] builder and compact `NVEV` v2 binary wire codec.

use crate::ffi::DecodeError;
use crate::id::{ArenaOffset, ByteLen, EventFlags, EventId, NodeId, SurfaceId, TimestampUs};
use crate::table::{TableKey, Word};

use super::validated::{validate_one_event, RawEventRef};
use super::{
    EventEnum, EventValidationError, InteractionEvent, LayoutEvent, NveEvent, NveEventBatch,
    RawEventBytes, ScrollEvent, TextInputEvent, ValidatedEvent, ValidatedEvents, NVE_EVENT_SIZE,
    NVE_TABLE_CUSTOM, NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL,
    NVE_TABLE_TEXT_INPUT,
};

/// Wire magic bytes for a binary [`EventBatchBuf`] frame (`"NVEV"`).
pub const NVE_EVENT_WIRE_MAGIC: [u8; 4] = *b"NVEV";
/// Wire format version for 64-byte [`NveEvent`] batches.
pub const NVE_EVENT_WIRE_VERSION: u16 = 2;
/// Fixed header size (`magic[4] + version:u16 + count:u32 + bytes_len:u32`).
pub const NVE_EVENT_WIRE_HEADER_SIZE: usize = 14;

/// Reusable owned batch of uniform 64-byte [`NveEvent`] records + companion byte arena.
///
/// Seals all [`ValidatedEvents`] invariants by construction so [`Self::as_validated`] is O(1)
/// with zero re-scanning and zero allocations across frames after calling [`Self::clear`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBatchBuf {
    events: Vec<NveEvent>,
    bytes: Vec<u8>,
}

impl EventBatchBuf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(events: usize, bytes: usize) -> Self {
        Self {
            events: Vec::with_capacity(events),
            bytes: Vec::with_capacity(bytes),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.events.clear();
        self.bytes.clear();
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[inline(always)]
    pub fn events(&self) -> &[NveEvent] {
        &self.events
    }

    #[inline(always)]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[inline(always)]
    pub fn as_validated(&self) -> ValidatedEvents<'_> {
        ValidatedEvents::from_sealed_aligned(&self.events, &self.bytes)
    }

    #[inline(always)]
    pub fn as_raw_batch(&self) -> NveEventBatch {
        NveEventBatch::from_parts(&self.events, &self.bytes)
    }

    fn append_arena(&mut self, data: &[u8]) -> RawEventBytes {
        if data.is_empty() {
            return RawEventBytes::EMPTY;
        }
        let off = u32::try_from(self.bytes.len()).expect("EventBatchBuf arena exceeds 4 GiB");
        let len = u32::try_from(data.len()).expect("EventBatchBuf slice exceeds 4 GiB");
        assert!(
            off.checked_add(len).is_some(),
            "EventBatchBuf arena offset + len overflows u32"
        );
        self.bytes.extend_from_slice(data);
        RawEventBytes::new(ArenaOffset(off), ByteLen(len))
    }

    fn push_sealed(&mut self, ev: NveEvent) {
        assert!(
            self.events.len() < u32::MAX as usize,
            "EventBatchBuf event count exceeds u32::MAX"
        );
        self.events.push(ev);
    }

    pub fn push_validated(&mut self, ev: ValidatedEvent<'_>) {
        let mut raw = ev.raw();
        raw.surface = ev.surface();
        raw.node = ev.node();
        raw.bytes = self.append_arena(ev.bytes());
        self.push_sealed(raw);
    }

    pub fn extend_from_validated(&mut self, other: ValidatedEvents<'_>) {
        self.events.reserve(other.len());
        for ev in other.iter() {
            self.push_validated(ev);
        }
    }

    pub fn try_push_raw(
        &mut self,
        mut ev: NveEvent,
        data: &[u8],
    ) -> Result<(), EventValidationError> {
        let idx = self.events.len();
        ev.bytes = RawEventBytes::new(
            ArenaOffset::ZERO,
            ByteLen(
                u32::try_from(data.len()).map_err(|_| EventValidationError::BytesLengthOverflow)?,
            ),
        );
        validate_one_event(idx, RawEventRef::Aligned(&ev), data)?;
        ev.surface = SurfaceId::from_raw_or_null(ev.surface.raw());
        ev.node = NodeId::from_raw_or_null(ev.node.raw());
        ev.bytes = self.append_arena(data);
        self.push_sealed(ev);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_table_event(
        &mut self,
        table: TableKey,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: EventId,
        flags: EventFlags,
        payload: [Word; 6],
        data: &[u8],
    ) {
        let bytes = self.append_arena(data);
        self.push_sealed(
            NveEvent::new(table, surface, node, timestamp, event)
                .with_flags(flags.without_internal_framing())
                .with_payload(payload)
                .with_bytes(bytes),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_event<E: EventEnum>(
        &mut self,
        table: TableKey,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: E,
        flags: EventFlags,
        payload: [Word; 6],
        data: &[u8],
    ) {
        self.push_table_event(
            table,
            surface,
            node,
            timestamp,
            event.event_id(),
            flags,
            payload,
            data,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_event_text<E: EventEnum>(
        &mut self,
        table: TableKey,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: E,
        flags: EventFlags,
        payload: [Word; 6],
        text: &str,
    ) {
        let bytes = self.append_arena(text.as_bytes());
        self.push_sealed(
            NveEvent::new(table, surface, node, timestamp, event.event_id())
                .with_flags(flags.without_internal_framing().union(EventFlags::UTF8_BYTES))
                .with_payload(payload)
                .with_bytes(bytes),
        );
    }

    #[inline]
    pub fn push_press(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        x: f32,
        y: f32,
    ) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            InteractionEvent::Press,
            EventFlags::EMPTY,
            [x.to_bits(), y.to_bits(), 0, 0, 0, 0],
            &[],
        );
    }

    #[inline]
    pub fn push_long_press(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        x: f32,
        y: f32,
    ) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            InteractionEvent::LongPress,
            EventFlags::EMPTY,
            [x.to_bits(), y.to_bits(), 0, 0, 0, 0],
            &[],
        );
    }

    #[inline]
    pub fn push_focus(&mut self, surface: SurfaceId, node: NodeId, timestamp: TimestampUs) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            InteractionEvent::Focus,
            EventFlags::EMPTY,
            [0; 6],
            &[],
        );
    }

    #[inline]
    pub fn push_blur(&mut self, surface: SurfaceId, node: NodeId, timestamp: TimestampUs) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            InteractionEvent::Blur,
            EventFlags::EMPTY,
            [0; 6],
            &[],
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_key(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: InteractionEvent,
        key: &str,
        code: &str,
        flags: EventFlags,
        key_code: u32,
    ) {
        let mut payload = [0u32; 6];
        payload[0] = key_code;
        let base_flags = flags.without_internal_framing();
        if NveEvent::pack_inline_key(&mut payload, key) {
            let bytes = self.append_arena(code.as_bytes());
            let mut f = base_flags.union(EventFlags::INLINE_KEY_UTF8);
            if !code.is_empty() {
                f |= EventFlags::UTF8_BYTES;
            }
            self.push_sealed(
                NveEvent::new(NVE_TABLE_INTERACTION, surface, node, timestamp, event.event_id())
                    .with_flags(f)
                    .with_payload(payload)
                    .with_bytes(bytes),
            );
        } else {
            let klen = u32::try_from(key.len()).expect("key length exceeds u32::MAX");
            payload[1] = klen;
            let off = u32::try_from(self.bytes.len()).expect("EventBatchBuf arena exceeds 4 GiB");
            let total = key
                .len()
                .checked_add(code.len())
                .and_then(|n| u32::try_from(n).ok())
                .expect("key + code exceeds u32::MAX");
            assert!(
                off.checked_add(total).is_some(),
                "EventBatchBuf arena offset + len overflows u32"
            );
            self.bytes.extend_from_slice(key.as_bytes());
            self.bytes.extend_from_slice(code.as_bytes());
            let bytes = RawEventBytes::new(ArenaOffset(off), ByteLen(total));
            self.push_sealed(
                NveEvent::new(NVE_TABLE_INTERACTION, surface, node, timestamp, event.event_id())
                    .with_flags(
                        base_flags
                            .union(EventFlags::KEY_SPLIT_BYTES)
                            .union(EventFlags::UTF8_BYTES),
                    )
                    .with_payload(payload)
                    .with_bytes(bytes),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_key_down(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        key: &str,
        code: &str,
        flags: EventFlags,
        key_code: u32,
    ) {
        self.push_key(
            surface,
            node,
            timestamp,
            InteractionEvent::KeyDown,
            key,
            code,
            flags,
            key_code,
        );
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_key_up(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        key: &str,
        code: &str,
        flags: EventFlags,
        key_code: u32,
    ) {
        self.push_key(
            surface,
            node,
            timestamp,
            InteractionEvent::KeyUp,
            key,
            code,
            flags,
            key_code,
        );
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_pointer(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: InteractionEvent,
        x: f32,
        y: f32,
        pointer_id: u32,
        flags: EventFlags,
    ) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            event,
            flags,
            [x.to_bits(), y.to_bits(), pointer_id, 0, 0, 0],
            &[],
        );
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_wheel(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        delta_x: f32,
        delta_y: f32,
        flags: EventFlags,
    ) {
        self.push_event(
            NVE_TABLE_INTERACTION,
            surface,
            node,
            timestamp,
            InteractionEvent::Wheel,
            flags,
            [delta_x.to_bits(), delta_y.to_bits(), 0, 0, 0, 0],
            &[],
        );
    }

    #[inline]
    pub fn push_text_change(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        text: &str,
    ) {
        self.push_event_text(
            NVE_TABLE_TEXT_INPUT,
            surface,
            node,
            timestamp,
            TextInputEvent::Change,
            EventFlags::EMPTY,
            [0; 6],
            text,
        );
    }

    #[inline]
    pub fn push_submit(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        text: &str,
    ) {
        self.push_event_text(
            NVE_TABLE_TEXT_INPUT,
            surface,
            node,
            timestamp,
            TextInputEvent::Submit,
            EventFlags::EMPTY,
            [0; 6],
            text,
        );
    }

    #[inline]
    pub fn push_composition(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: TextInputEvent,
        text: &str,
    ) {
        self.push_event_text(
            NVE_TABLE_TEXT_INPUT,
            surface,
            node,
            timestamp,
            event,
            EventFlags::EMPTY,
            [0; 6],
            text,
        );
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_scroll(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        x: f32,
        y: f32,
        content_width: f32,
        content_height: f32,
    ) {
        self.push_event(
            NVE_TABLE_SCROLL,
            surface,
            node,
            timestamp,
            ScrollEvent::Scroll,
            EventFlags::EMPTY,
            [
                x.to_bits(),
                y.to_bits(),
                content_width.to_bits(),
                content_height.to_bits(),
                0,
                0,
            ],
            &[],
        );
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn push_layout(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        self.push_event(
            NVE_TABLE_LAYOUT,
            surface,
            node,
            timestamp,
            LayoutEvent::Layout,
            EventFlags::EMPTY,
            [x.to_bits(), y.to_bits(), width.to_bits(), height.to_bits(), 0, 0],
            &[],
        );
    }

    pub fn push_custom(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        name: &str,
        data: &[u8],
    ) {
        let nlen = u32::try_from(name.len()).expect("custom event name exceeds u32::MAX");
        let off = u32::try_from(self.bytes.len()).expect("EventBatchBuf arena exceeds 4 GiB");
        let total = name
            .len()
            .checked_add(data.len())
            .and_then(|n| u32::try_from(n).ok())
            .expect("custom event payload exceeds u32::MAX");
        assert!(
            off.checked_add(total).is_some(),
            "EventBatchBuf arena offset + len overflows u32"
        );
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.extend_from_slice(data);
        let bytes = RawEventBytes::new(ArenaOffset(off), ByteLen(total));
        self.push_sealed(
            NveEvent::new(NVE_TABLE_CUSTOM, surface, node, timestamp, EventId(0))
                .with_flags(EventFlags::CUSTOM_NAMED)
                .with_payload([nlen, 0, 0, 0, 0, 0])
                .with_bytes(bytes),
        );
    }

    /// Appends this batch as a little-endian `NVEV` v2 wire frame into `out`.
    pub fn encode_wire(&self, out: &mut Vec<u8>) {
        let count = u32::try_from(self.events.len()).expect("event count exceeds u32::MAX");
        let blen = u32::try_from(self.bytes.len()).expect("event arena exceeds u32::MAX");
        out.reserve(
            NVE_EVENT_WIRE_HEADER_SIZE + self.events.len() * NVE_EVENT_SIZE + self.bytes.len(),
        );
        out.extend_from_slice(&NVE_EVENT_WIRE_MAGIC);
        out.extend_from_slice(&NVE_EVENT_WIRE_VERSION.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&blen.to_le_bytes());
        for ev in &self.events {
            out.extend_from_slice(ev.as_bytes());
        }
        out.extend_from_slice(&self.bytes);
    }

    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_wire(&mut out);
        out
    }

    /// Decodes and validates an `NVEV` v2 wire frame into an owned [`EventBatchBuf`].
    pub fn decode_wire(raw: &[u8]) -> Result<Self, DecodeError> {
        if raw.len() < NVE_EVENT_WIRE_HEADER_SIZE {
            return Err(DecodeError::Truncated);
        }
        let magic = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        if magic != u32::from_le_bytes(NVE_EVENT_WIRE_MAGIC) {
            return Err(DecodeError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([raw[4], raw[5]]);
        if version != NVE_EVENT_WIRE_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let count = u32::from_le_bytes([raw[6], raw[7], raw[8], raw[9]]) as usize;
        let bytes_len = u32::from_le_bytes([raw[10], raw[11], raw[12], raw[13]]) as usize;

        let ev_bytes = count
            .checked_mul(NVE_EVENT_SIZE)
            .ok_or(DecodeError::Truncated)?;
        let body_len = ev_bytes
            .checked_add(bytes_len)
            .ok_or(DecodeError::Truncated)?;
        let expected_total = NVE_EVENT_WIRE_HEADER_SIZE
            .checked_add(body_len)
            .ok_or(DecodeError::Truncated)?;
        if raw.len() < expected_total {
            return Err(DecodeError::Truncated);
        }
        if raw.len() > expected_total {
            return Err(DecodeError::TrailingBytes(raw.len() - expected_total));
        }

        let ev_slice = &raw[NVE_EVENT_WIRE_HEADER_SIZE..NVE_EVENT_WIRE_HEADER_SIZE + ev_bytes];
        let arena_slice = &raw[NVE_EVENT_WIRE_HEADER_SIZE + ev_bytes..expected_total];

        let mut events = Vec::with_capacity(count);
        let (chunks, _) = ev_slice.as_chunks::<NVE_EVENT_SIZE>();
        for (idx, arr) in chunks.iter().enumerate() {
            let mut ev = RawEventRef::Foreign(arr).read();
            ev.surface = SurfaceId::from_raw_or_null(ev.surface.raw());
            ev.node = NodeId::from_raw_or_null(ev.node.raw());
            validate_one_event(idx, RawEventRef::Aligned(&ev), arena_slice).map_err(|e| match e {
                EventValidationError::InvalidUtf8 { .. } => DecodeError::InvalidUtf8,
                _ => DecodeError::Truncated,
            })?;
            events.push(ev);
        }

        Ok(Self {
            events,
            bytes: arena_slice.to_vec(),
        })
    }
}
