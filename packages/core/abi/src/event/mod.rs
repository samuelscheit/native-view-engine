//! Uniform pointer-free 64-byte [`NveEvent`], [`NveEventBatch`], [`EventEnum`], [`EventSet`],
//! ingress-validated [`ValidatedEvents`], and reusable [`EventBatchBuf`].

mod buf;
mod set;
#[cfg(test)]
mod tests;
mod validated;
mod wire;

use nve_abi_macros::{EventEnum, NveAbi};
use nve_hash::Key64;

use crate::id::{ArenaOffset, ByteLen, EventFlags, EventId, NodeId, SurfaceId, TimestampUs};
use crate::table::{TableKey, Word};

pub use buf::EventBatchBuf;
pub use wire::{NVE_EVENT_WIRE_HEADER_SIZE, NVE_EVENT_WIRE_MAGIC, NVE_EVENT_WIRE_VERSION};
pub use set::{EventEnum, EventSet, EventSetIter};
pub use validated::{
    EventValidationError, ValidatedEvent, ValidatedEvents, ValidatedEventsIter,
};

const _: () = assert!(
    cfg!(target_endian = "little"),
    "NveEvent inline UTF-8 packing and cross-language ABI require little-endian byte order"
);

/// Exact byte size of one uniform [`NveEvent`] record (one 64-byte cache line).
pub const NVE_EVENT_SIZE: usize = 64;
/// Cache-line alignment of [`NveEvent`] in Rust slabs.
pub const NVE_EVENT_ALIGN: usize = 64;
/// Maximum inline UTF-8 key byte length stored inside [`NveEvent::payload`]`[2..6]`.
pub const NVE_EVENT_INLINE_KEY_CAP: usize = 16;

/// Canonical compile-time table key for `Interaction` (`nve.interaction`).
pub const NVE_TABLE_INTERACTION: TableKey = Key64::of("nve.interaction");
/// Canonical compile-time table key for `TextInput` (`nve.text_input`).
pub const NVE_TABLE_TEXT_INPUT: TableKey = Key64::of("nve.text_input");
/// Canonical compile-time table key for `Scroll` (`nve.scroll`).
pub const NVE_TABLE_SCROLL: TableKey = Key64::of("nve.scroll");
/// Canonical compile-time table key for `Layout` (`nve.layout`).
pub const NVE_TABLE_LAYOUT: TableKey = Key64::of("nve.layout");
/// Canonical compile-time table key for ad-hoc named custom host events (`nve.custom`).
pub const NVE_TABLE_CUSTOM: TableKey = Key64::of("nve.custom");

impl EventFlags {
    /// Meta / Command modifier flag (bit 0).
    pub const MOD_META: Self = Self(1 << 0);
    /// Control modifier flag (bit 1).
    pub const MOD_CTRL: Self = Self(1 << 1);
    /// Shift modifier flag (bit 2).
    pub const MOD_SHIFT: Self = Self(1 << 2);
    /// Alt / Option modifier flag (bit 3).
    pub const MOD_ALT: Self = Self(1 << 3);
    /// Key auto-repeat flag (bit 4).
    pub const MOD_REPEAT: Self = Self(1 << 4);
    /// Combined keyboard/pointer modifier bitmask (`bits 0..=4`).
    pub const MOD_MASK: Self = Self(0x1F);
    /// Bit shift for pointer button index (`bits 8..=10`).
    pub const BUTTON_SHIFT: u32 = 8;
    /// Bitmask for pointer button index (`bits 8..=10`).
    pub const BUTTON_MASK: Self = Self(0x07 << 8);
    /// Pointer capture active flag (bit 11).
    pub const CAPTURE: Self = Self(1 << 11);
    /// IME composition active flag (bit 12).
    pub const COMPOSING: Self = Self(1 << 12);
    /// Target-only (non-bubbling) dispatch flag (bit 13, e.g. DOM `close` / `toggle`).
    pub const NO_BUBBLE: Self = Self(1 << 13);
    /// Ingress-validated flag: `NveEvent::bytes` slice is valid UTF-8 (bit 16).
    pub const UTF8_BYTES: Self = Self(1 << 16);
    /// Ingress-validated flag: `payload[1]` holds `0..=16` inline UTF-8 key length in `payload[2..6]` (bit 17).
    pub const INLINE_KEY_UTF8: Self = Self(1 << 17);
    /// Ingress-validated flag: `payload[0]` holds custom event name byte length prefixing `bytes` (bit 18).
    pub const CUSTOM_NAMED: Self = Self(1 << 18);
    /// Ingress-validated flag: `payload[1]` splits `bytes` into UTF-8 `[key, code]` for keys `> 16 B` (bit 19).
    pub const KEY_SPLIT_BYTES: Self = Self(1 << 19);

    const INTERNAL_FRAMING_MASK: u32 = Self::UTF8_BYTES.0
        | Self::INLINE_KEY_UTF8.0
        | Self::CUSTOM_NAMED.0
        | Self::KEY_SPLIT_BYTES.0;

    #[inline(always)]
    pub const fn from_modifiers(
        meta: bool,
        ctrl: bool,
        shift: bool,
        alt: bool,
        repeat: bool,
    ) -> Self {
        Self(
            (if meta { Self::MOD_META.0 } else { 0 })
                | (if ctrl { Self::MOD_CTRL.0 } else { 0 })
                | (if shift { Self::MOD_SHIFT.0 } else { 0 })
                | (if alt { Self::MOD_ALT.0 } else { 0 })
                | (if repeat { Self::MOD_REPEAT.0 } else { 0 }),
        )
    }

    #[inline(always)]
    pub const fn with_pointer_button(self, button: u8) -> Self {
        Self((self.0 & !Self::BUTTON_MASK.0) | (((button as u32) & 0x07) << Self::BUTTON_SHIFT))
    }

    #[inline(always)]
    pub const fn pointer_button(self) -> u8 {
        ((self.0 & Self::BUTTON_MASK.0) >> Self::BUTTON_SHIFT) as u8
    }

    #[inline(always)]
    pub const fn meta(self) -> bool {
        self.contains(Self::MOD_META)
    }

    #[inline(always)]
    pub const fn ctrl(self) -> bool {
        self.contains(Self::MOD_CTRL)
    }

    #[inline(always)]
    pub const fn shift(self) -> bool {
        self.contains(Self::MOD_SHIFT)
    }

    #[inline(always)]
    pub const fn alt(self) -> bool {
        self.contains(Self::MOD_ALT)
    }

    #[inline(always)]
    pub const fn repeat(self) -> bool {
        self.contains(Self::MOD_REPEAT)
    }

    #[inline(always)]
    pub const fn has_primary_modifier(self) -> bool {
        self.meta() || self.ctrl()
    }

    #[inline(always)]
    pub const fn is_capture(self) -> bool {
        self.contains(Self::CAPTURE)
    }

    #[inline(always)]
    pub const fn is_composing(self) -> bool {
        self.contains(Self::COMPOSING)
    }

    /// `false` when the event targets its node only ([`Self::NO_BUBBLE`]); unset events bubble.
    #[inline(always)]
    pub const fn bubbles(self) -> bool {
        !self.contains(Self::NO_BUBBLE)
    }

    /// Sets or clears [`Self::NO_BUBBLE`].
    #[inline(always)]
    pub const fn with_bubbles(self, bubbles: bool) -> Self {
        if bubbles {
            Self(self.0 & !Self::NO_BUBBLE.0)
        } else {
            Self(self.0 | Self::NO_BUBBLE.0)
        }
    }

    #[inline(always)]
    pub const fn without_internal_framing(self) -> Self {
        Self(self.0 & !Self::INTERNAL_FRAMING_MASK)
    }

    /// Returns `true` iff `bytes` carries a plain UTF-8 text payload (and NOT inline key physical
    /// `code`, split key `[key, code]`, or `CUSTOM_NAMED` `[name, data]`).
    #[inline(always)]
    pub const fn has_utf8_text(self) -> bool {
        const NON_TEXT_FRAMING: EventFlags = EventFlags(
            EventFlags::INLINE_KEY_UTF8.0
                | EventFlags::KEY_SPLIT_BYTES.0
                | EventFlags::CUSTOM_NAMED.0,
        );
        self.contains(Self::UTF8_BYTES) && !self.intersects(NON_TEXT_FRAMING)
    }
}

/// Events emitted by the `Interaction` table (`nve.interaction`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EventEnum, NveAbi)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[nve_abi(name = "NveInteractionEvent", c_prefix = "NVE_INTERACTION_EVENT_")]
pub enum InteractionEvent {
    Press = 0,
    LongPress = 1,
    Focus = 2,
    Blur = 3,
    KeyDown = 4,
    KeyUp = 5,
    PointerDown = 6,
    PointerMove = 7,
    PointerUp = 8,
    PointerEnter = 9,
    PointerLeave = 10,
    Wheel = 11,
}

/// Events emitted by the `TextInput` table (`nve.text_input`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EventEnum, NveAbi)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[nve_abi(name = "NveTextInputEvent", c_prefix = "NVE_TEXT_INPUT_EVENT_")]
pub enum TextInputEvent {
    Change = 0,
    Submit = 1,
    CompositionStart = 2,
    CompositionUpdate = 3,
    CompositionEnd = 4,
}

/// Events emitted by the `Scroll` table (`nve.scroll`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EventEnum, NveAbi)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[nve_abi(name = "NveScrollEvent", c_prefix = "NVE_SCROLL_EVENT_")]
pub enum ScrollEvent {
    Scroll = 0,
}

/// Events emitted by the `Layout` table (`nve.layout`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EventEnum, NveAbi)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[nve_abi(name = "NveLayoutEvent", c_prefix = "NVE_LAYOUT_EVENT_")]
pub enum LayoutEvent {
    Layout = 0,
}

/// Byte span inside an [`NveEventBatch`] byte arena (8 bytes, pointer-free).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, NveAbi)]
#[nve_abi(size = 8)]
pub struct RawEventBytes {
    #[nve_abi(offset = 0)]
    pub offset: ArenaOffset,
    #[nve_abi(offset = 4)]
    pub len: ByteLen,
}

impl RawEventBytes {
    pub const EMPTY: Self = Self {
        offset: ArenaOffset::ZERO,
        len: ByteLen::ZERO,
    };

    #[inline(always)]
    pub const fn new(offset: ArenaOffset, len: ByteLen) -> Self {
        Self { offset, len }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len.raw() == 0
    }
}

/// Uniform, pointer-free 64-byte event record (one CPU cache line; identical field offsets on
/// 64-bit, 32-bit ARMv7/i686, and `wasm32`).
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, NveAbi)]
#[nve_abi(size = 64)]
pub struct NveEvent {
    #[nve_abi(offset = 0)]
    pub timestamp: TimestampUs,
    #[nve_abi(offset = 8)]
    pub table: TableKey,
    #[nve_abi(offset = 16)]
    pub surface: SurfaceId,
    #[nve_abi(offset = 20)]
    pub node: NodeId,
    #[nve_abi(offset = 24)]
    pub event: EventId,
    #[nve_abi(offset = 28)]
    pub flags: EventFlags,
    #[nve_abi(offset = 32)]
    pub payload: [Word; 6],
    #[nve_abi(offset = 56)]
    pub bytes: RawEventBytes,
}

impl NveEvent {
    #[inline(always)]
    pub const fn new(
        table: TableKey,
        surface: SurfaceId,
        node: NodeId,
        timestamp: TimestampUs,
        event: EventId,
    ) -> Self {
        Self {
            timestamp,
            table,
            surface,
            node,
            event,
            flags: EventFlags::EMPTY,
            payload: [0; 6],
            bytes: RawEventBytes::EMPTY,
        }
    }

    #[inline(always)]
    pub const fn with_flags(mut self, flags: EventFlags) -> Self {
        self.flags = flags;
        self
    }

    #[inline(always)]
    pub const fn with_payload(mut self, payload: [Word; 6]) -> Self {
        self.payload = payload;
        self
    }

    #[inline(always)]
    pub const fn with_bytes(mut self, bytes: RawEventBytes) -> Self {
        self.bytes = bytes;
        self
    }

    #[inline(always)]
    pub const fn payload_f32(&self, idx: usize) -> f32 {
        f32::from_bits(self.payload[idx])
    }

    #[inline(always)]
    pub const fn payload_u32(&self, idx: usize) -> u32 {
        self.payload[idx]
    }

    #[inline(always)]
    pub const fn payload_u64(&self, idx: usize) -> u64 {
        (self.payload[idx] as u64) | ((self.payload[idx + 1] as u64) << 32)
    }

    #[inline(always)]
    pub const fn payload_f64(&self, idx: usize) -> f64 {
        f64::from_bits(self.payload_u64(idx))
    }

    #[inline(always)]
    pub fn set_payload_u64(&mut self, idx: usize, v: u64) {
        self.payload[idx] = v as u32;
        self.payload[idx + 1] = (v >> 32) as u32;
    }

    #[inline(always)]
    pub fn set_payload_f64(&mut self, idx: usize, v: f64) {
        self.set_payload_u64(idx, v.to_bits());
    }

    #[inline]
    pub fn pack_inline_key(payload: &mut [Word; 6], key: &str) -> bool {
        let bytes = key.as_bytes();
        if bytes.len() > NVE_EVENT_INLINE_KEY_CAP {
            return false;
        }
        let mut buf = [0u8; NVE_EVENT_INLINE_KEY_CAP];
        buf[..bytes.len()].copy_from_slice(bytes);
        payload[1] = bytes.len() as u32;
        payload[2] = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        payload[3] = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        payload[4] = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        payload[5] = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        true
    }

    #[inline(always)]
    pub const fn as_bytes(&self) -> &[u8; NVE_EVENT_SIZE] {
        // SAFETY: `NveEvent` is `#[repr(C, align(64))]`, exactly 64 bytes with zero uninitialized
        // padding bytes (8+8+4+4+4+4+24+8 = 64), and alignment 64 >= 1.
        unsafe { &*(self as *const Self).cast::<[u8; NVE_EVENT_SIZE]>() }
    }
}

/// Zero-copy FFI event batch pairing a contiguous [`NveEvent`] array with its shared byte arena
/// (24 bytes on 64-bit, 16 bytes on 32-bit, zero implicit padding).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 24)]
pub struct NveEventBatch {
    #[nve_abi(offset = 0)]
    pub events: *const NveEvent,
    #[nve_abi(offset64 = 8)]
    pub bytes: *const u8,
    #[nve_abi(offset64 = 16)]
    pub event_count: u32,
    #[nve_abi(offset64 = 20)]
    pub bytes_len: ByteLen,
}

impl NveEventBatch {
    pub const EMPTY: Self = Self {
        events: core::ptr::null(),
        bytes: core::ptr::null(),
        event_count: 0,
        bytes_len: ByteLen::ZERO,
    };

    #[inline]
    pub const fn from_parts(events: &[NveEvent], bytes: &[u8]) -> Self {
        let event_count = if events.len() > u32::MAX as usize {
            u32::MAX
        } else {
            events.len() as u32
        };
        let bytes_len = if bytes.len() > u32::MAX as usize {
            u32::MAX
        } else {
            bytes.len() as u32
        };
        Self {
            events: if event_count == 0 {
                core::ptr::null()
            } else {
                events.as_ptr()
            },
            bytes: if bytes_len == 0 {
                core::ptr::null()
            } else {
                bytes.as_ptr()
            },
            event_count,
            bytes_len: ByteLen(bytes_len),
        }
    }
}

impl Default for NveEventBatch {
    #[inline(always)]
    fn default() -> Self {
        Self::EMPTY
    }
}
