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
}
