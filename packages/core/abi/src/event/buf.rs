//! Reusable owned [`EventBatchBuf`] builder sealing all [`ValidatedEvents`] invariants by
//! construction (see `wire.rs` for the `NVEV` v2 binary wire codec).

use crate::id::{ArenaOffset, ByteLen, EventFlags, EventId, NodeId, SurfaceId, TimestampUs};
use crate::table::{TableKey, Word};

use super::validated::{validate_one_event, RawEventRef};
use super::{
    EventEnum, EventValidationError, InteractionEvent, LayoutEvent, NveEvent, NveEventBatch,
    RawEventBytes, ScrollEvent, TextInputEvent, ValidatedEvent, ValidatedEvents, NVE_TABLE_CUSTOM,
    NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};

/// Reusable owned batch of uniform 64-byte [`NveEvent`] records + companion byte arena.
///
/// Seals all [`ValidatedEvents`] invariants by construction so [`Self::as_validated`] is O(1)
/// with zero re-scanning and zero allocations across frames after calling [`Self::clear`].
///
/// Every batch fits the `u32` wire limits (`u32::MAX` events, 4 GiB arena): the infallible
/// `push_*` / `extend_*` / `encode_wire` helpers drop each event (or frame) that would exceed them;
/// the `try_*` counterparts report it as [`EventValidationError`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBatchBuf {
    pub(super) events: Vec<NveEvent>,
    pub(super) bytes: Vec<u8>,
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

    #[inline]
    fn check_event_room(&self) -> Result<(), EventValidationError> {
        if self.events.len() >= u32::MAX as usize {
            Err(EventValidationError::EventsLengthOverflow)
        } else {
            Ok(())
        }
    }

    fn try_append_arena(&mut self, data: &[u8]) -> Result<RawEventBytes, EventValidationError> {
        if data.is_empty() {
            return Ok(RawEventBytes::EMPTY);
        }
        let off = u32::try_from(self.bytes.len())
            .map_err(|_| EventValidationError::BytesLengthOverflow)?;
        let len = u32::try_from(data.len())
            .map_err(|_| EventValidationError::BytesLengthOverflow)?;
        if off.checked_add(len).is_none() {
            return Err(EventValidationError::BytesLengthOverflow);
        }
        self.bytes.extend_from_slice(data);
        Ok(RawEventBytes::new(ArenaOffset(off), ByteLen(len)))
    }

    fn try_append_pair(
        &mut self,
        first: &[u8],
        second: &[u8],
    ) -> Result<RawEventBytes, EventValidationError> {
        let total = first
            .len()
            .checked_add(second.len())
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(EventValidationError::BytesLengthOverflow)?;
        if total == 0 {
            return Ok(RawEventBytes::EMPTY);
        }
        let off = u32::try_from(self.bytes.len())
            .map_err(|_| EventValidationError::BytesLengthOverflow)?;
        if off.checked_add(total).is_none() {
            return Err(EventValidationError::BytesLengthOverflow);
        }
        self.bytes.extend_from_slice(first);
        self.bytes.extend_from_slice(second);
        Ok(RawEventBytes::new(ArenaOffset(off), ByteLen(total)))
    }

    fn try_push_sealed(
        &mut self,
        mut ev: NveEvent,
        data: &[u8],
    ) -> Result<(), EventValidationError> {
        self.check_event_room()?;
        ev.bytes = self.try_append_arena(data)?;
        self.events.push(ev);
        Ok(())
    }

    pub fn try_push_validated(&mut self, ev: ValidatedEvent<'_>) -> Result<(), EventValidationError> {
        let mut raw = ev.raw();
        raw.surface = ev.surface();
        raw.node = ev.node();
        self.try_push_sealed(raw, ev.bytes())
    }

    pub fn push_validated(&mut self, ev: ValidatedEvent<'_>) {
        let _ = self.try_push_validated(ev);
    }

    pub fn try_extend_from_validated(
        &mut self,
        other: ValidatedEvents<'_>,
    ) -> Result<(), EventValidationError> {
        let next_len = self
            .events
            .len()
            .checked_add(other.len())
            .ok_or(EventValidationError::EventsLengthOverflow)?;
        if next_len > u32::MAX as usize {
            return Err(EventValidationError::EventsLengthOverflow);
        }
        self.events.reserve(other.len());
        for ev in other.iter() {
            self.try_push_validated(ev)?;
        }
        Ok(())
    }

    pub fn extend_from_validated(&mut self, other: ValidatedEvents<'_>) {
        let _ = self.try_extend_from_validated(other);
    }

    pub fn try_push_raw(
        &mut self,
        mut ev: NveEvent,
        data: &[u8],
    ) -> Result<(), EventValidationError> {
        self.check_event_room()?;
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
        self.try_push_sealed(ev, data)
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
        let _ = self.try_push_sealed(
            NveEvent::new(table, surface, node, timestamp, event)
                .with_flags(flags.without_internal_framing())
                .with_payload(payload),
            data,
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
        let _ = self.try_push_sealed(
            NveEvent::new(table, surface, node, timestamp, event.event_id())
                .with_flags(flags.without_internal_framing().union(EventFlags::UTF8_BYTES))
                .with_payload(payload),
            text.as_bytes(),
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
            let mut f = base_flags.union(EventFlags::INLINE_KEY_UTF8);
            if !code.is_empty() {
                f |= EventFlags::UTF8_BYTES;
            }
            let _ = self.try_push_sealed(
                NveEvent::new(NVE_TABLE_INTERACTION, surface, node, timestamp, event.event_id())
                    .with_flags(f)
                    .with_payload(payload),
                code.as_bytes(),
            );
        } else {
            let Ok(klen) = u32::try_from(key.len()) else {
                return;
            };
            if self.check_event_room().is_err() {
                return;
            }
            let Ok(bytes) = self.try_append_pair(key.as_bytes(), code.as_bytes()) else {
                return;
            };
            payload[1] = klen;
            self.events.push(
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

    pub fn try_push_custom(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        name: &str,
        data: &[u8],
    ) -> Result<(), EventValidationError> {
        self.check_event_room()?;
        let nlen = u32::try_from(name.len())
            .map_err(|_| EventValidationError::BytesLengthOverflow)?;
        let bytes = self.try_append_pair(name.as_bytes(), data)?;
        self.events.push(
            NveEvent::new(NVE_TABLE_CUSTOM, surface, node, timestamp, EventId(0))
                .with_flags(EventFlags::CUSTOM_NAMED)
                .with_payload([nlen, 0, 0, 0, 0, 0])
                .with_bytes(bytes),
        );
        Ok(())
    }

    pub fn push_custom(
        &mut self,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        name: &str,
        data: &[u8],
    ) {
        let _ = self.try_push_custom(surface, node, timestamp, name, data);
    }
}
