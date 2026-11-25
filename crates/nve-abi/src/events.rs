use serde::{Deserialize, Serialize};

use crate::{DecodeError, NodeId, SurfaceId};

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
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeEventKind {
    Press = 1,
    LongPress = 2,
    Scroll = 3,
    TextChange = 4,
    Focus = 5,
    Blur = 6,
    Layout = 7,
    Custom = 8,
}

impl NativeEventKind {
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(Self::Press),
            2 => Ok(Self::LongPress),
            3 => Ok(Self::Scroll),
            4 => Ok(Self::TextChange),
            5 => Ok(Self::Focus),
            6 => Ok(Self::Blur),
            7 => Ok(Self::Layout),
            8 => Ok(Self::Custom),
            other => Err(DecodeError::InvalidEnum("NativeEventKind", other as u32)),
        }
    }
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeEvent {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub timestamp_us: u64,
    pub kind: NativeEventKind,
    pub payload: EventPayload,
}

/// 3-byte ASCII wire magic identifying a compact zero-JSON binary keydown payload (`NK1`).
///
/// Layout: `[b'N', b'K', b'1', flags_u8, utf8_key...]`
/// where `flags_u8` packs modifier bits:
/// - bit 0 (`0x01`): `meta`
/// - bit 1 (`0x02`): `ctrl`
/// - bit 2 (`0x04`): `shift`
/// - bit 3 (`0x08`): `alt`
pub const KEYDOWN_WIRE_MAGIC: [u8; 3] = *b"NK1";
pub const KEYDOWN_FLAG_META: u8 = 1 << 0;
pub const KEYDOWN_FLAG_CTRL: u8 = 1 << 1;
pub const KEYDOWN_FLAG_SHIFT: u8 = 1 << 2;
pub const KEYDOWN_FLAG_ALT: u8 = 1 << 3;

/// Decoded zero-JSON keyboard event from `NativeEventKind::Custom` (`name: "keydown"`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedKeyDown {
    pub key: String,
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl DecodedKeyDown {
    #[inline]
    pub fn has_primary_modifier(&self) -> bool {
        self.meta || self.ctrl
    }
}

/// Encodes a keyboard event into the compact zero-JSON `NK1` binary payload:
/// `[b'N', b'K', b'1', flags_u8, utf8_key...]`.
pub fn encode_keydown_bytes(
    key: &str,
    meta: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Vec<u8> {
    let flags = (u8::from(meta) * KEYDOWN_FLAG_META)
        | (u8::from(ctrl) * KEYDOWN_FLAG_CTRL)
        | (u8::from(shift) * KEYDOWN_FLAG_SHIFT)
        | (u8::from(alt) * KEYDOWN_FLAG_ALT);
    let mut out = Vec::with_capacity(4 + key.len());
    out.extend_from_slice(&KEYDOWN_WIRE_MAGIC);
    out.push(flags);
    out.extend_from_slice(key.as_bytes());
    out
}

/// Decodes a `keydown` payload from either:
/// 1. Compact binary `NK1` framing (`[b'N', b'K', b'1', flags_u8, utf8_key...]`), or
/// 2. Compact UTF-8 shorthand (`"Cmd+k"`, `"Ctrl+Shift+Enter"`, `"Escape"`, `"Enter"`, `"+"`).
pub fn decode_keydown_bytes(data: &[u8]) -> Option<DecodedKeyDown> {
    if data.starts_with(&KEYDOWN_WIRE_MAGIC) {
        if data.len() < 5 {
            return None;
        }
        let flags = data[3];
        let key = std::str::from_utf8(&data[4..]).ok()?;
        if key.is_empty() {
            return None;
        }
        return Some(DecodedKeyDown {
            key: key.to_owned(),
            meta: (flags & KEYDOWN_FLAG_META) != 0,
            ctrl: (flags & KEYDOWN_FLAG_CTRL) != 0,
            shift: (flags & KEYDOWN_FLAG_SHIFT) != 0,
            alt: (flags & KEYDOWN_FLAG_ALT) != 0,
        });
    }

    let raw_text = std::str::from_utf8(data).ok()?;
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed == "+" {
        return Some(DecodedKeyDown {
            key: "+".to_owned(),
            ..DecodedKeyDown::default()
        });
    }

    if trimmed.contains('+') {
        let mut out = DecodedKeyDown::default();
        // Preserve trailing literal `+` key in combinations like `"Cmd++"`.
        let (mod_section, key_part) = if let Some(prefix) = trimmed.strip_suffix("++") {
            (prefix, "+")
        } else {
            let split_at = trimmed.rfind('+')?;
            (&trimmed[..split_at], trimmed[split_at + 1..].trim())
        };
        if key_part.is_empty() {
            return None;
        }
        for tok in mod_section.split('+').map(str::trim).filter(|s| !s.is_empty()) {
            match tok.to_ascii_lowercase().as_str() {
                "cmd" | "meta" | "super" => out.meta = true,
                "ctrl" | "control" => out.ctrl = true,
                "shift" => out.shift = true,
                "alt" | "opt" | "option" => out.alt = true,
                _ => {}
            }
        }
        out.key = key_part.to_owned();
        return Some(out);
    }

    Some(DecodedKeyDown {
        key: trimmed.to_owned(),
        ..DecodedKeyDown::default()
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
    let read_u32 = |pos: &mut usize| -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(take(pos, 4)?.try_into().unwrap()))
    };
    let read_u64 = |pos: &mut usize| -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(take(pos, 8)?.try_into().unwrap()))
    };
    let read_f32 = |pos: &mut usize| -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(take(pos, 4)?.try_into().unwrap()))
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
                let nlen = u16::from_le_bytes(take(&mut pos, 2)?.try_into().unwrap()) as usize;
                let name = std::str::from_utf8(take(&mut pos, nlen)?)
                    .map_err(|_| DecodeError::InvalidUtf8)?
                    .to_owned();
                let dlen = read_u32(&mut pos)? as usize;
                let data = take(&mut pos, dlen)?.to_vec();
                EventPayload::Custom { name, data }
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
mod tests {
    use super::*;

    #[test]
    fn roundtrip_event_ring() {
        let events = vec![
            NativeEvent {
                surface_id: 1,
                node_id: 10,
                timestamp_us: 123_456,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 42.5, y: 18.0 },
            },
            NativeEvent {
                surface_id: 1,
                node_id: 11,
                timestamp_us: 123_500,
                kind: NativeEventKind::TextChange,
                payload: EventPayload::Text("hello".into()),
            },
            NativeEvent {
                surface_id: 1,
                node_id: 12,
                timestamp_us: 123_900,
                kind: NativeEventKind::Custom,
                payload: EventPayload::Custom {
                    name: "onSvgLoad".into(),
                    data: vec![9, 8, 7],
                },
            },
        ];
        let encoded = encode_events(&events);
        let decoded = decode_events(&encoded).unwrap();
        assert_eq!(events, decoded);
    }

    #[test]
    fn keydown_binary_nk1_and_shorthand_roundtrip_and_edge_cases() {
        // Every single modifier flag combination roundtrips through NK1 binary framing.
        for mask in 0u8..16 {
            let meta = (mask & KEYDOWN_FLAG_META) != 0;
            let ctrl = (mask & KEYDOWN_FLAG_CTRL) != 0;
            let shift = (mask & KEYDOWN_FLAG_SHIFT) != 0;
            let alt = (mask & KEYDOWN_FLAG_ALT) != 0;
            let encoded = encode_keydown_bytes("k", meta, ctrl, shift, alt);
            assert_eq!(&encoded[..3], &KEYDOWN_WIRE_MAGIC);
            assert_eq!(encoded[3], mask);
            assert_eq!(&encoded[4..], b"k");
            let decoded = decode_keydown_bytes(&encoded).expect("valid NK1");
            assert_eq!(
                decoded,
                DecodedKeyDown {
                    key: "k".into(),
                    meta,
                    ctrl,
                    shift,
                    alt,
                }
            );
            assert_eq!(decoded.has_primary_modifier(), meta || ctrl);
        }

        // Multi-byte UTF-8 and space keys survive NK1 verbatim without trimming.
        let space = encode_keydown_bytes(" ", false, false, true, false);
        assert_eq!(
            decode_keydown_bytes(&space),
            Some(DecodedKeyDown {
                key: " ".into(),
                shift: true,
                ..Default::default()
            })
        );
        let cjk = encode_keydown_bytes("漢", true, false, false, true);
        assert_eq!(
            decode_keydown_bytes(&cjk),
            Some(DecodedKeyDown {
                key: "漢".into(),
                meta: true,
                alt: true,
                ..Default::default()
            })
        );

        // Compact UTF-8 shorthand works seamlessly.
        assert_eq!(
            decode_keydown_bytes(b"Cmd+k"),
            Some(DecodedKeyDown {
                key: "k".into(),
                meta: true,
                ..Default::default()
            })
        );
        assert_eq!(
            decode_keydown_bytes(b"Ctrl+Shift+Alt+Enter"),
            Some(DecodedKeyDown {
                key: "Enter".into(),
                ctrl: true,
                shift: true,
                alt: true,
                ..Default::default()
            })
        );
        assert_eq!(
            decode_keydown_bytes(b"Escape"),
            Some(DecodedKeyDown {
                key: "Escape".into(),
                ..Default::default()
            })
        );
        assert_eq!(
            decode_keydown_bytes(b"+"),
            Some(DecodedKeyDown {
                key: "+".into(),
                ..Default::default()
            })
        );
        assert_eq!(
            decode_keydown_bytes(b"Cmd++"),
            Some(DecodedKeyDown {
                key: "+".into(),
                meta: true,
                ..Default::default()
            })
        );

        // Truncated / empty / invalid UTF-8 inputs return None safely.
        assert_eq!(decode_keydown_bytes(&[]), None);
        assert_eq!(decode_keydown_bytes(b"   "), None);
        assert_eq!(decode_keydown_bytes(b"NK1"), None);
        assert_eq!(decode_keydown_bytes(b"NK1\x01"), None);
        assert_eq!(decode_keydown_bytes(&[b'N', b'K', b'1', 0, 0xFF, 0xFE]), None);
        assert_eq!(decode_keydown_bytes(&[0xFF, 0xFE]), None);
        assert_eq!(decode_keydown_bytes(b"Cmd+"), None);
    }
}
