use serde::{Deserialize, Serialize};

pub use crate::NativeEventKind;
use crate::{
    DecodeError, NodeId, NveDirectEvent, NveDirectEventRef, NveEventPayloadTag, NveValue,
    SurfaceId, NVE_EVENT_FLAG_BUTTON_MASK, NVE_EVENT_FLAG_COMPOSING, NVE_KEY_MOD_ALT,
    NVE_KEY_MOD_CTRL, NVE_KEY_MOD_META, NVE_KEY_MOD_REPEAT, NVE_KEY_MOD_SHIFT,
};

/// Event subscription bitflags encoded in `Command::SetEventMask`.
pub struct EventMask;

impl EventMask {
    pub const NONE: u64 = 0;
    pub const PRESS: u64 = 1 << 0;
    pub const LONG_PRESS: u64 = 1 << 1;
    pub const SCROLL: u64 = 1 << 2;
    pub const TEXT_CHANGE: u64 = 1 << 3;
    pub const FOCUS: u64 = 1 << 4;
    pub const BLUR: u64 = 1 << 5;
    pub const LAYOUT: u64 = 1 << 6;
    pub const CUSTOM: u64 = 1 << 7;
    pub const COMPOSITION: u64 = 1 << 8;
    pub const KEY_DOWN: u64 = 1 << 9;
    pub const KEY_UP: u64 = 1 << 10;
    pub const SUBMIT: u64 = 1 << 11;
    pub const POINTER_DOWN: u64 = 1 << 12;
    pub const POINTER_MOVE: u64 = 1 << 13;
    pub const POINTER_UP: u64 = 1 << 14;
    pub const POINTER_ENTER: u64 = 1 << 15;
    pub const POINTER_LEAVE: u64 = 1 << 16;
    pub const WHEEL: u64 = 1 << 17;
    pub const TABLE_EVENT: u64 = 1 << 18;

    /// The subscription bit a node needs to receive `kind`. Not `1 << (kind - 1)`: the three
    /// composition kinds share [`Self::COMPOSITION`].
    pub const fn of(kind: NativeEventKind) -> u64 {
        match kind {
            NativeEventKind::Press => Self::PRESS,
            NativeEventKind::LongPress => Self::LONG_PRESS,
            NativeEventKind::Scroll => Self::SCROLL,
            NativeEventKind::TextChange => Self::TEXT_CHANGE,
            NativeEventKind::Focus => Self::FOCUS,
            NativeEventKind::Blur => Self::BLUR,
            NativeEventKind::Layout => Self::LAYOUT,
            NativeEventKind::Custom => Self::CUSTOM,
            NativeEventKind::CompositionStart
            | NativeEventKind::CompositionUpdate
            | NativeEventKind::CompositionEnd => Self::COMPOSITION,
            NativeEventKind::KeyDown => Self::KEY_DOWN,
            NativeEventKind::KeyUp => Self::KEY_UP,
            NativeEventKind::Submit => Self::SUBMIT,
            NativeEventKind::PointerDown => Self::POINTER_DOWN,
            NativeEventKind::PointerMove => Self::POINTER_MOVE,
            NativeEventKind::PointerUp => Self::POINTER_UP,
            NativeEventKind::PointerEnter => Self::POINTER_ENTER,
            NativeEventKind::PointerLeave => Self::POINTER_LEAVE,
            NativeEventKind::Wheel => Self::WHEEL,
            NativeEventKind::TableEvent => Self::TABLE_EVENT,
        }
    }

    /// Host mirror names of every bit (`NVE_EVENT_MASK_*`, via `nve.interaction.event_mask`).
    pub const VALUES: crate::table::ValueSetDesc = crate::table::ValueSetDesc {
        name: "EventMask",
        values: &[
            ("Press", Self::PRESS as u32),
            ("LongPress", Self::LONG_PRESS as u32),
            ("Scroll", Self::SCROLL as u32),
            ("TextChange", Self::TEXT_CHANGE as u32),
            ("Focus", Self::FOCUS as u32),
            ("Blur", Self::BLUR as u32),
            ("Layout", Self::LAYOUT as u32),
            ("Custom", Self::CUSTOM as u32),
            ("Composition", Self::COMPOSITION as u32),
            ("KeyDown", Self::KEY_DOWN as u32),
            ("KeyUp", Self::KEY_UP as u32),
            ("Submit", Self::SUBMIT as u32),
            ("PointerDown", Self::POINTER_DOWN as u32),
            ("PointerMove", Self::POINTER_MOVE as u32),
            ("PointerUp", Self::POINTER_UP as u32),
            ("PointerEnter", Self::POINTER_ENTER as u32),
            ("PointerLeave", Self::POINTER_LEAVE as u32),
            ("Wheel", Self::WHEEL as u32),
            ("TableEvent", Self::TABLE_EVENT as u32),
        ],
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventPayload {
    Empty,
    Point {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        content_width: f32,
        content_height: f32,
    },
    Text(String),
    Layout {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Custom {
        name: String,
        data: Vec<u8>,
    },
    Key {
        key: String,
        code: String,
        modifiers: u8,
        key_code: u32,
        is_composing: bool,
    },
    Pointer {
        x: f32,
        y: f32,
        modifiers: u8,
        flags: u8,
    },
    Wheel {
        delta_x: f32,
        delta_y: f32,
        modifiers: u8,
    },
    TableEvent {
        table_key: u64,
        event_id: u32,
        payload: Vec<u8>,
    },
}

/// Packs keyboard modifier flags into the `NveDirectEvent::modifiers` bitmask.
#[inline]
pub const fn pack_key_modifiers(
    meta: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
    repeat: bool,
) -> u8 {
    (if meta { NVE_KEY_MOD_META } else { 0 })
        | (if ctrl { NVE_KEY_MOD_CTRL } else { 0 })
        | (if shift { NVE_KEY_MOD_SHIFT } else { 0 })
        | (if alt { NVE_KEY_MOD_ALT } else { 0 })
        | (if repeat { NVE_KEY_MOD_REPEAT } else { 0 })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeEvent {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub timestamp_us: u64,
    pub kind: NativeEventKind,
    pub payload: EventPayload,
}

impl NativeEvent {
    /// Borrows this owned `NativeEvent` as a zero-copy `NveDirectEvent` whose string/byte
    /// pointers remain valid for as long as `self` is borrowed and unmodified.
    #[inline]
    pub fn as_direct_event(&self) -> NveDirectEvent {
        match &self.payload {
            EventPayload::Empty => {
                NveDirectEvent::empty(self.surface_id, self.node_id, self.timestamp_us, self.kind)
            }
            EventPayload::Point { x, y } => NveDirectEvent {
                kind: self.kind as u8,
                payload_tag: NveEventPayloadTag::Point as u8,
                x: *x,
                y: *y,
                ..NveDirectEvent::empty(self.surface_id, self.node_id, self.timestamp_us, self.kind)
            },
            EventPayload::Scroll {
                x,
                y,
                content_width,
                content_height,
            } => NveDirectEvent::scroll(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                *x,
                *y,
                *content_width,
                *content_height,
            ),
            EventPayload::Text(text) => NveDirectEvent {
                kind: self.kind as u8,
                payload_tag: NveEventPayloadTag::Text as u8,
                text: crate::NveStrSlice::from_str(text),
                ..NveDirectEvent::empty(self.surface_id, self.node_id, self.timestamp_us, self.kind)
            },
            EventPayload::Layout {
                x,
                y,
                width,
                height,
            } => NveDirectEvent::layout(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                *x,
                *y,
                *width,
                *height,
            ),
            EventPayload::Custom { name, data } => NveDirectEvent::custom(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                name,
                NveValue::from_bytes(data),
            ),
            EventPayload::Key {
                key,
                code,
                modifiers,
                key_code,
                is_composing,
            } => NveDirectEvent {
                kind: self.kind as u8,
                payload_tag: NveEventPayloadTag::Key as u8,
                modifiers: *modifiers,
                event_flags: if *is_composing {
                    NVE_EVENT_FLAG_COMPOSING
                } else {
                    0
                },
                key_code: *key_code,
                name: crate::NveStrSlice::from_str(code),
                text: crate::NveStrSlice::from_str(key),
                ..NveDirectEvent::empty(self.surface_id, self.node_id, self.timestamp_us, self.kind)
            },
            EventPayload::Pointer {
                x,
                y,
                modifiers,
                flags,
            } => NveDirectEvent::pointer(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                self.kind,
                *x,
                *y,
                *modifiers,
                *flags,
            ),
            EventPayload::Wheel {
                delta_x,
                delta_y,
                modifiers,
            } => NveDirectEvent::wheel(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                *delta_x,
                *delta_y,
                *modifiers,
            ),
            EventPayload::TableEvent {
                table_key,
                event_id,
                payload,
            } => NveDirectEvent::table_event(
                self.surface_id,
                self.node_id,
                self.timestamp_us,
                crate::Key64(*table_key),
                *event_id,
                payload,
            ),
        }
    }
}

/// Converts a slice of owned `NativeEvent`s into zero-copy `NveDirectEvent` records borrowing
/// directly from `events`.
#[inline]
pub fn native_events_to_direct(events: &[NativeEvent], out: &mut Vec<NveDirectEvent>) {
    out.clear();
    out.reserve(events.len());
    for ev in events {
        out.push(ev.as_direct_event());
    }
}

/// Converts a borrowed `NveDirectEventRef` into an owned `NativeEvent` (used at network wire
/// boundaries and in-memory test harnesses).
pub fn direct_event_to_native(ev: NveDirectEventRef<'_>) -> Option<NativeEvent> {
    let kind = ev.kind()?;
    let payload = match ev.payload_tag() {
        0 => EventPayload::Empty,
        1 => EventPayload::Point {
            x: ev.x(),
            y: ev.y(),
        },
        2 => EventPayload::Scroll {
            x: ev.x(),
            y: ev.y(),
            content_width: ev.width(),
            content_height: ev.height(),
        },
        3 => EventPayload::Text(ev.text().to_owned()),
        4 => EventPayload::Layout {
            x: ev.x(),
            y: ev.y(),
            width: ev.width(),
            height: ev.height(),
        },
        5 => {
            let data = if let Some(b) = ev.value().as_bytes() {
                b.to_vec()
            } else if let Some(s) = ev.value().as_str() {
                s.as_bytes().to_vec()
            } else if !ev.value().is_null() {
                crate::encode_bin_value_to_vec(ev.value()).unwrap_or_default()
            } else if !ev.text().is_empty() {
                ev.text().as_bytes().to_vec()
            } else {
                Vec::new()
            };
            EventPayload::Custom {
                name: ev.name().to_owned(),
                data,
            }
        }
        6 => EventPayload::Key {
            key: ev.key().to_owned(),
            code: ev.code().to_owned(),
            modifiers: ev.modifiers(),
            key_code: ev.key_code(),
            is_composing: ev.is_composing(),
        },
        7 => EventPayload::Pointer {
            x: ev.x(),
            y: ev.y(),
            modifiers: ev.modifiers(),
            flags: ev.event_flags() & (NVE_EVENT_FLAG_BUTTON_MASK | (1 << 3) | (1 << 4)),
        },
        8 => EventPayload::Wheel {
            delta_x: ev.delta_x(),
            delta_y: ev.delta_y(),
            modifiers: ev.modifiers(),
        },
        9 => EventPayload::TableEvent {
            table_key: ev.table_key().0,
            event_id: ev.table_event_id(),
            payload: ev.table_payload().to_vec(),
        },
        _ => return None,
    };
    Some(NativeEvent {
        surface_id: ev.surface_id(),
        node_id: ev.node_id(),
        timestamp_us: ev.timestamp_us(),
        kind,
        payload,
    })
}

pub fn encode_events(events: &[NativeEvent]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + events.len() * 32);
    out.extend_from_slice(&(events.len() as u32).to_le_bytes());
    for ev in events {
        out.extend_from_slice(&ev.surface_id.to_le_bytes());
        out.extend_from_slice(&ev.node_id.to_le_bytes());
        out.extend_from_slice(&ev.timestamp_us.to_le_bytes());
        out.push(ev.kind as u8);
        match &ev.payload {
            EventPayload::Empty => out.push(0),
            EventPayload::Point { x, y } => {
                out.push(1);
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
            }
            EventPayload::Scroll {
                x,
                y,
                content_width,
                content_height,
            } => {
                out.push(2);
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
                out.extend_from_slice(&content_width.to_le_bytes());
                out.extend_from_slice(&content_height.to_le_bytes());
            }
            EventPayload::Text(t) => {
                out.push(3);
                let b = t.as_bytes();
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b);
            }
            EventPayload::Layout {
                x,
                y,
                width,
                height,
            } => {
                out.push(4);
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
                out.extend_from_slice(&width.to_le_bytes());
                out.extend_from_slice(&height.to_le_bytes());
            }
            EventPayload::Custom { name, data } => {
                out.push(5);
                let nb = name.as_bytes();
                out.extend_from_slice(&(nb.len() as u16).to_le_bytes());
                out.extend_from_slice(nb);
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(data);
            }
            EventPayload::Key {
                key,
                code,
                modifiers,
                key_code,
                is_composing,
            } => {
                out.push(6);
                out.push(*modifiers);
                out.push(u8::from(*is_composing));
                out.extend_from_slice(&key_code.to_le_bytes());
                let kb = key.as_bytes();
                out.extend_from_slice(&(kb.len() as u16).to_le_bytes());
                out.extend_from_slice(kb);
                let cb = code.as_bytes();
                out.extend_from_slice(&(cb.len() as u16).to_le_bytes());
                out.extend_from_slice(cb);
            }
            EventPayload::Pointer {
                x,
                y,
                modifiers,
                flags,
            } => {
                out.push(7);
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
                out.push(*modifiers);
                out.push(*flags);
            }
            EventPayload::Wheel {
                delta_x,
                delta_y,
                modifiers,
            } => {
                out.push(8);
                out.extend_from_slice(&delta_x.to_le_bytes());
                out.extend_from_slice(&delta_y.to_le_bytes());
                out.push(*modifiers);
            }
            EventPayload::TableEvent {
                table_key,
                event_id,
                payload,
            } => {
                out.push(9);
                out.extend_from_slice(&table_key.to_le_bytes());
                out.extend_from_slice(&event_id.to_le_bytes());
                out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                out.extend_from_slice(payload);
            }
        }
    }
    out
}

pub fn decode_events(bytes: &[u8]) -> Result<Vec<NativeEvent>, DecodeError> {
    let mut pos = 0usize;
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], DecodeError> {
        if bytes.len().saturating_sub(*pos) < n {
            return Err(DecodeError::Truncated);
        }
        let s = &bytes[*pos..*pos + n];
        *pos += n;
        Ok(s)
    };
    let read_u16 = |pos: &mut usize| -> Result<u16, DecodeError> {
        let b = take(pos, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    };
    let read_u32 = |pos: &mut usize| -> Result<u32, DecodeError> {
        let b = take(pos, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let read_u64 = |pos: &mut usize| -> Result<u64, DecodeError> {
        let b = take(pos, 8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    };
    let read_f32 = |pos: &mut usize| -> Result<f32, DecodeError> {
        let b = take(pos, 4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };

    let count = read_u32(&mut pos)? as usize;
    let mut out = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let surface_id = read_u32(&mut pos)?;
        let node_id = read_u32(&mut pos)?;
        let timestamp_us = read_u64(&mut pos)?;
        let kind = NativeEventKind::from_u8(take(&mut pos, 1)?[0])?;
        let ptag = take(&mut pos, 1)?[0];
        let payload = match ptag {
            0 => EventPayload::Empty,
            1 => EventPayload::Point {
                x: read_f32(&mut pos)?,
                y: read_f32(&mut pos)?,
            },
            2 => EventPayload::Scroll {
                x: read_f32(&mut pos)?,
                y: read_f32(&mut pos)?,
                content_width: read_f32(&mut pos)?,
                content_height: read_f32(&mut pos)?,
            },
            3 => {
                let len = read_u32(&mut pos)? as usize;
                let s = std::str::from_utf8(take(&mut pos, len)?)
                    .map_err(|_| DecodeError::InvalidUtf8)?
                    .to_owned();
                EventPayload::Text(s)
            }
            4 => EventPayload::Layout {
                x: read_f32(&mut pos)?,
                y: read_f32(&mut pos)?,
                width: read_f32(&mut pos)?,
                height: read_f32(&mut pos)?,
            },
            5 => {
                let nlen = read_u16(&mut pos)? as usize;
                let name = std::str::from_utf8(take(&mut pos, nlen)?)
                    .map_err(|_| DecodeError::InvalidUtf8)?
                    .to_owned();
                let dlen = read_u32(&mut pos)? as usize;
                let data = take(&mut pos, dlen)?.to_vec();
                EventPayload::Custom { name, data }
            }
            6 => {
                let modifiers = take(&mut pos, 1)?[0];
                let is_composing = take(&mut pos, 1)?[0] != 0;
                let key_code = read_u32(&mut pos)?;
                let klen = read_u16(&mut pos)? as usize;
                let key = std::str::from_utf8(take(&mut pos, klen)?)
                    .map_err(|_| DecodeError::InvalidUtf8)?
                    .to_owned();
                let clen = read_u16(&mut pos)? as usize;
                let code = std::str::from_utf8(take(&mut pos, clen)?)
                    .map_err(|_| DecodeError::InvalidUtf8)?
                    .to_owned();
                EventPayload::Key {
                    key,
                    code,
                    modifiers,
                    key_code,
                    is_composing,
                }
            }
            7 => EventPayload::Pointer {
                x: read_f32(&mut pos)?,
                y: read_f32(&mut pos)?,
                modifiers: take(&mut pos, 1)?[0],
                flags: take(&mut pos, 1)?[0],
            },
            8 => EventPayload::Wheel {
                delta_x: read_f32(&mut pos)?,
                delta_y: read_f32(&mut pos)?,
                modifiers: take(&mut pos, 1)?[0],
            },
            9 => {
                let table_key = read_u64(&mut pos)?;
                let event_id = read_u32(&mut pos)?;
                let len = read_u32(&mut pos)? as usize;
                let payload = take(&mut pos, len)?.to_vec();
                EventPayload::TableEvent {
                    table_key,
                    event_id,
                    payload,
                }
            }
            other => return Err(DecodeError::InvalidEnum("EventPayload", other as u32)),
        };
        out.push(NativeEvent {
            surface_id,
            node_id,
            timestamp_us,
            kind,
            payload,
        });
    }
    if pos != bytes.len() {
        return Err(DecodeError::TrailingBytes(bytes.len() - pos));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
