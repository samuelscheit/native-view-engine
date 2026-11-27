use nve_abi_macros::NveAbi;

use crate::{NativeEventKind, NveEventPayloadTag};

use super::ffi_types::{guarded_slice, NveStrSlice, NveValue};

/// Keyboard/pointer modifier bitflags packed into `NveDirectEvent::modifiers`.
pub const NVE_KEY_MOD_META: u8 = 1 << 0;
pub const NVE_KEY_MOD_CTRL: u8 = 1 << 1;
pub const NVE_KEY_MOD_SHIFT: u8 = 1 << 2;
pub const NVE_KEY_MOD_ALT: u8 = 1 << 3;
pub const NVE_KEY_MOD_REPEAT: u8 = 1 << 4;

/// Event flags packed into `NveDirectEvent::event_flags`.
/// Bits `0..=2` encode pointer button (`0` primary/none, `1` secondary, `2` auxiliary, ...).
pub const NVE_EVENT_FLAG_BUTTON_MASK: u8 = 0x07;
pub const NVE_EVENT_FLAG_CAPTURE: u8 = 1 << 3;
pub const NVE_EVENT_FLAG_COMPOSING: u8 = 1 << 4;

/// Direct FFI event matching `struct NveDirectEvent` in `nve_abi.generated.h`
/// (96 bytes on 64-bit, 72 bytes on 32-bit, align 8).
#[repr(C, align(8))]
#[derive(Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 96)]
pub struct NveDirectEvent {
    #[nve_abi(offset = 0)]
    pub surface_id: u32,
    #[nve_abi(offset = 4)]
    pub node_id: u32,
    #[nve_abi(offset = 8)]
    pub timestamp_us: u64,
    /// `NativeEventKind` (1..=20).
    #[nve_abi(offset = 16)]
    pub kind: u8,
    /// `NveEventPayloadTag` (0..=8).
    #[nve_abi(offset = 17)]
    pub payload_tag: u8,
    /// Modifier bitmask (`NVE_KEY_MOD_META..NVE_KEY_MOD_REPEAT`).
    #[nve_abi(offset = 18)]
    pub modifiers: u8,
    /// Pointer button (`0..=2`), capture (`1<<3`), and IME composing (`1<<4`) flags.
    #[nve_abi(offset = 19)]
    pub event_flags: u8,
    /// Platform/DOM numeric key code (`0` for non-keyboard events).
    #[nve_abi(offset = 20)]
    pub key_code: u32,
    /// Physical key code (for `KeyDown`/`KeyUp`) or custom event name (for `Custom`).
    #[nve_abi(offset = 24)]
    pub name: NveStrSlice,
    #[nve_abi(offset64 = 40)]
    pub x: f32,
    #[nve_abi(offset64 = 44)]
    pub y: f32,
    #[nve_abi(offset64 = 48)]
    pub width: f32,
    #[nve_abi(offset64 = 52)]
    pub height: f32,
    /// W3C key string (for `KeyDown`/`KeyUp`) or text/composition string.
    #[nve_abi(offset64 = 56)]
    pub text: NveStrSlice,
    #[nve_abi(offset64 = 72)]
    pub value: NveValue,
}

impl Default for NveDirectEvent {
    #[inline]
    fn default() -> Self {
        Self::empty(0, 0, 0, NativeEventKind::Focus)
    }
}

impl NveDirectEvent {
    #[inline]
    pub const fn empty(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        kind: NativeEventKind,
    ) -> Self {
        Self {
            surface_id,
            node_id,
            timestamp_us,
            kind: kind as u8,
            payload_tag: NveEventPayloadTag::Empty as u8,
            modifiers: 0,
            event_flags: 0,
            key_code: 0,
            name: NveStrSlice::EMPTY,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            text: NveStrSlice::EMPTY,
            value: NveValue::null(),
        }
    }

    #[inline]
    pub const fn press(surface_id: u32, node_id: u32, timestamp_us: u64, x: f32, y: f32) -> Self {
        Self {
            kind: NativeEventKind::Press as u8,
            payload_tag: NveEventPayloadTag::Point as u8,
            x,
            y,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Press)
        }
    }

    #[inline]
    pub const fn long_press(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        x: f32,
        y: f32,
    ) -> Self {
        Self {
            kind: NativeEventKind::LongPress as u8,
            payload_tag: NveEventPayloadTag::Point as u8,
            x,
            y,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::LongPress)
        }
    }

    #[inline]
    pub const fn scroll(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        x: f32,
        y: f32,
        content_width: f32,
        content_height: f32,
    ) -> Self {
        Self {
            kind: NativeEventKind::Scroll as u8,
            payload_tag: NveEventPayloadTag::Scroll as u8,
            x,
            y,
            width: content_width,
            height: content_height,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Scroll)
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
            kind: NativeEventKind::TextChange as u8,
            payload_tag: NveEventPayloadTag::Text as u8,
            text: NveStrSlice::from_str(text),
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::TextChange)
        }
    }

    #[inline]
    pub const fn layout(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            kind: NativeEventKind::Layout as u8,
            payload_tag: NveEventPayloadTag::Layout as u8,
            x,
            y,
            width,
            height,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Layout)
        }
    }

    #[inline]
    pub const fn composition(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        kind: NativeEventKind,
        text: &str,
    ) -> Self {
        Self {
            kind: kind as u8,
            payload_tag: NveEventPayloadTag::Text as u8,
            text: NveStrSlice::from_str(text),
            ..Self::empty(surface_id, node_id, timestamp_us, kind)
        }
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub const fn key_down(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        key: &str,
        code: &str,
        modifiers: u8,
        key_code: u32,
        is_composing: bool,
    ) -> Self {
        Self {
            kind: NativeEventKind::KeyDown as u8,
            payload_tag: NveEventPayloadTag::Key as u8,
            modifiers,
            event_flags: if is_composing {
                NVE_EVENT_FLAG_COMPOSING
            } else {
                0
            },
            key_code,
            name: NveStrSlice::from_str(code),
            text: NveStrSlice::from_str(key),
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::KeyDown)
        }
    }

    #[inline]
    pub const fn key_up(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        key: &str,
        code: &str,
        modifiers: u8,
        key_code: u32,
    ) -> Self {
        Self {
            kind: NativeEventKind::KeyUp as u8,
            payload_tag: NveEventPayloadTag::Key as u8,
            modifiers,
            key_code,
            name: NveStrSlice::from_str(code),
            text: NveStrSlice::from_str(key),
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::KeyUp)
        }
    }

    #[inline]
    pub const fn submit(surface_id: u32, node_id: u32, timestamp_us: u64, text: &str) -> Self {
        Self {
            kind: NativeEventKind::Submit as u8,
            payload_tag: if text.is_empty() {
                NveEventPayloadTag::Empty as u8
            } else {
                NveEventPayloadTag::Text as u8
            },
            text: NveStrSlice::from_str(text),
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Submit)
        }
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub const fn pointer(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        kind: NativeEventKind,
        x: f32,
        y: f32,
        modifiers: u8,
        event_flags: u8,
    ) -> Self {
        Self {
            kind: kind as u8,
            payload_tag: NveEventPayloadTag::Pointer as u8,
            modifiers,
            event_flags,
            x,
            y,
            ..Self::empty(surface_id, node_id, timestamp_us, kind)
        }
    }

    #[inline]
    pub const fn wheel(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        delta_x: f32,
        delta_y: f32,
        modifiers: u8,
    ) -> Self {
        Self {
            kind: NativeEventKind::Wheel as u8,
            payload_tag: NveEventPayloadTag::Wheel as u8,
            modifiers,
            x: delta_x,
            y: delta_y,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Wheel)
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
            kind: NativeEventKind::Custom as u8,
            payload_tag: NveEventPayloadTag::Custom as u8,
            name: NveStrSlice::from_str(name),
            value,
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::Custom)
        }
    }

    /// Zero-copy direct event for a module/table return event (`NativeEventKind::TableEvent`).
    ///
    /// Packs `table_key` inline in `value` (`NveValue::i64`), `event_id` in `key_code`, and
    /// borrows `payload` bytes in `text` with zero struct-size growth.
    #[inline]
    pub const fn table_event(
        surface_id: u32,
        node_id: u32,
        timestamp_us: u64,
        table_key: crate::Key64,
        event_id: u32,
        payload: &[u8],
    ) -> Self {
        Self {
            kind: NativeEventKind::TableEvent as u8,
            payload_tag: NveEventPayloadTag::TableEvent as u8,
            key_code: event_id,
            text: NveStrSlice {
                ptr: payload.as_ptr(),
                len: payload.len(),
            },
            value: NveValue::i64(table_key.0 as i64),
            ..Self::empty(surface_id, node_id, timestamp_us, NativeEventKind::TableEvent)
        }
    }
}

/// Contiguous batch of direct events matching `struct NveEventBatch` in `nve_abi.generated.h`
/// (16 bytes on 64-bit, 12 bytes on 32-bit, zero implicit tail padding).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 16)]
pub struct NveEventBatch {
    #[nve_abi(offset = 0)]
    pub ptr: *const NveDirectEvent,
    #[nve_abi(offset64 = 8)]
    pub len: u32,
    #[nve_abi(offset64 = 12)]
    pub _pad: u32,
}

impl NveEventBatch {
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
        _pad: 0,
    };

    #[inline]
    pub fn from_slice(events: &[NveDirectEvent]) -> Self {
        Self {
            ptr: events.as_ptr(),
            len: events.len().min(u32::MAX as usize) as u32,
            _pad: 0,
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
