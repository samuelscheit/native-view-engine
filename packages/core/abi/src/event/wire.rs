//! Compact little-endian `NVEV` v2 binary wire codec for [`EventBatchBuf`].

use crate::ffi::DecodeError;
use crate::id::{NodeId, SurfaceId};

use super::validated::{validate_one_event, RawEventRef};
use super::{EventBatchBuf, EventValidationError, NVE_EVENT_SIZE};

/// Wire magic bytes for a binary [`EventBatchBuf`] frame (`"NVEV"`).
pub const NVE_EVENT_WIRE_MAGIC: [u8; 4] = *b"NVEV";
/// Wire format version for 64-byte [`crate::NveEvent`] batches.
pub const NVE_EVENT_WIRE_VERSION: u16 = 2;
/// Fixed header size (`magic[4] + version:u16 + count:u32 + bytes_len:u32`).
pub const NVE_EVENT_WIRE_HEADER_SIZE: usize = 14;

impl EventBatchBuf {
    /// Fallibly appends this batch as a little-endian `NVEV` v2 wire frame into `out`.
    pub fn try_encode_wire(&self, out: &mut Vec<u8>) -> Result<(), EventValidationError> {
        let count = u32::try_from(self.events.len())
            .map_err(|_| EventValidationError::EventsLengthOverflow)?;
        let blen = u32::try_from(self.bytes.len())
            .map_err(|_| EventValidationError::BytesLengthOverflow)?;
        let ev_bytes = self
            .events
            .len()
            .checked_mul(NVE_EVENT_SIZE)
            .ok_or(EventValidationError::EventsLengthOverflow)?;
        let total = NVE_EVENT_WIRE_HEADER_SIZE
            .checked_add(ev_bytes)
            .and_then(|n| n.checked_add(self.bytes.len()))
            .ok_or(EventValidationError::BytesLengthOverflow)?;
        out.reserve(total);
        out.extend_from_slice(&NVE_EVENT_WIRE_MAGIC);
        out.extend_from_slice(&NVE_EVENT_WIRE_VERSION.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&blen.to_le_bytes());
        for ev in &self.events {
            out.extend_from_slice(ev.as_bytes());
        }
        out.extend_from_slice(&self.bytes);
        Ok(())
    }

    /// Appends this batch as a little-endian `NVEV` v2 wire frame into `out`.
    pub fn encode_wire(&self, out: &mut Vec<u8>) {
        let _ = self.try_encode_wire(out);
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
                other => DecodeError::InvalidEvent(other),
            })?;
            events.push(ev);
        }

        Ok(Self {
            events,
            bytes: arena_slice.to_vec(),
        })
    }
}
