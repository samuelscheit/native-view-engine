//! Bearer-token TLS 1.3 handshake messages (`Hello`, `HelloAck`, `Reject`) and out-of-order
//! `AsyncSlot` chunk payloads for `NveWireFrame` v2.

use nve_codec::{write_fixed, ByteReader, ByteSink, ByteWriter, CodecError, NveCodec, SliceWriter};

use crate::frame::NveWireError;

/// Default wire schema version negotiated during `Hello` / `HelloAck`.
pub const DEFAULT_WIRE_SCHEMA_VERSION: u16 = 2;

/// Capability flag: peer supports `NveSceneArchive` (`SceneKeyframe`) frames.
pub const NVE_WIRE_CAP_SCENE_ARCHIVE: u32 = 1 << 0;
/// Capability flag: peer supports compact `NodePatchDelta` frames.
pub const NVE_WIRE_CAP_NODE_PATCHES: u32 = 1 << 1;
/// Capability flag: peer supports RFC 9221 unreliable datagrams.
pub const NVE_WIRE_CAP_DATAGRAMS: u32 = 1 << 2;
/// Capability flag: peer supports out-of-order `SlotChunk` async/GenUI streaming.
pub const NVE_WIRE_CAP_SLOT_CHUNKS: u32 = 1 << 3;

const MAX_TOKEN_BYTES: usize = 512;
const MAX_CLIENT_NAME_BYTES: usize = 128;

/// Constant-time byte-slice equality check (prevents timing leaks on Bearer tokens).
#[inline]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len_diff = (a.len() ^ b.len()) as u64;
    let min_len = a.len().min(b.len());
    let mut diff: u8 = 0;
    for i in 0..min_len {
        diff |= a[i] ^ b[i];
    }
    len_diff == 0 && diff == 0
}

/// Handshake rejection reason carried in `NveWireKind::Reject` payloads.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
pub enum NveWireRejectReason {
    Unauthorized = 1,
    SchemaVersionMismatch = 2,
    UnsupportedVersion = 3,
    ServerBusy = 4,
    FrameTooLarge = 5,
}

impl NveWireRejectReason {
    pub fn encode(self, detail: &str) -> Vec<u8> {
        let mut clipped_len = detail.len().min(256);
        while clipped_len > 0 && !detail.is_char_boundary(clipped_len) {
            clipped_len -= 1;
        }
        let detail_bytes = &detail.as_bytes()[..clipped_len];
        let mut w = ByteWriter::with_capacity(4 + clipped_len);
        w.write_u8(self as u8);
        w.write_u8(0);
        w.write_u16(clipped_len as u16);
        w.write_raw(detail_bytes);
        w.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<(Self, String), NveWireError> {
        if bytes.len() < 4 {
            return Err(NveWireError::Truncated {
                expected: 4,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let reason = Self::from_repr(r.read_u8()?)
            .ok_or(NveWireError::InvalidPayload("unknown reject reason"))?;
        r.expect_zero(1)?;
        let detail_len = usize::from(r.read_u16()?);
        if r.remaining() != detail_len {
            return Err(NveWireError::InvalidPayload("reject detail length mismatch"));
        }
        let detail = std::str::from_utf8(r.read_exact(detail_len)?)
            .map_err(|_| NveWireError::InvalidUtf8)?
            .to_owned();
        Ok((reason, detail))
    }
}

/// Client `Hello` payload carrying the real Bearer token over TLS 1.3 (never derived from a hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveWireHello {
    pub schema_version: u16,
    pub capabilities: u32,
    pub bearer_token: String,
    pub client_name: String,
}

impl NveWireHello {
    pub fn new(bearer_token: impl Into<String>, client_name: impl Into<String>) -> Self {
        Self {
            schema_version: DEFAULT_WIRE_SCHEMA_VERSION,
            capabilities: NVE_WIRE_CAP_SCENE_ARCHIVE
                | NVE_WIRE_CAP_NODE_PATCHES
                | NVE_WIRE_CAP_DATAGRAMS
                | NVE_WIRE_CAP_SLOT_CHUNKS,
            bearer_token: bearer_token.into(),
            client_name: client_name.into(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, NveWireError> {
        let token = self.bearer_token.as_bytes();
        let name = self.client_name.as_bytes();
        if token.len() > MAX_TOKEN_BYTES {
            return Err(NveWireError::InvalidPayload("bearer token exceeds 512 bytes"));
        }
        if name.len() > MAX_CLIENT_NAME_BYTES {
            return Err(NveWireError::InvalidPayload("client name exceeds 128 bytes"));
        }
        let mut w = ByteWriter::with_capacity(10 + token.len() + name.len());
        w.write_u16(self.schema_version);
        w.write_u32(self.capabilities);
        w.write_u16(token.len() as u16);
        w.write_u16(name.len() as u16);
        w.write_raw(token);
        w.write_raw(name);
        Ok(w.into_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NveWireError> {
        if bytes.len() < 10 {
            return Err(NveWireError::Truncated {
                expected: 10,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let schema_version = r.read_u16()?;
        let capabilities = r.read_u32()?;
        let token_len = usize::from(r.read_u16()?);
        let name_len = usize::from(r.read_u16()?);
        if token_len > MAX_TOKEN_BYTES || name_len > MAX_CLIENT_NAME_BYTES {
            return Err(NveWireError::InvalidPayload("hello string exceeds limit"));
        }
        if r.remaining() != token_len + name_len {
            return Err(NveWireError::InvalidPayload("hello payload length mismatch"));
        }
        let bearer_token = std::str::from_utf8(r.read_exact(token_len)?)
            .map_err(|_| NveWireError::InvalidUtf8)?
            .to_owned();
        let client_name = std::str::from_utf8(r.read_exact(name_len)?)
            .map_err(|_| NveWireError::InvalidUtf8)?
            .to_owned();
        Ok(Self {
            schema_version,
            capabilities,
            bearer_token,
            client_name,
        })
    }
}

/// Server `HelloAck` payload confirming session ID and negotiated capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveWireHelloAck {
    pub session_id: u32,
    pub schema_version: u16,
    pub server_capabilities: u32,
}

impl NveWireHelloAck {
    pub const ENCODED_LEN: usize = 12;

    #[inline]
    fn write_to_sink<S: ByteSink>(&self, w: &mut S) -> Result<(), CodecError> {
        w.put_u32(self.session_id)?;
        w.put_u16(self.schema_version)?;
        w.put_zeroes(2)?;
        w.put_u32(self.server_capabilities)
    }

    #[inline]
    pub fn write_into(&self, dst: &mut [u8; Self::ENCODED_LEN]) -> Result<(), CodecError> {
        let mut w = SliceWriter::new(dst);
        self.write_to_sink(&mut w)?;
        w.finish()
    }

    #[inline]
    pub fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        write_fixed::<{ Self::ENCODED_LEN }>(|w| self.write_to_sink(w))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NveWireError> {
        if bytes.len() != Self::ENCODED_LEN {
            return Err(NveWireError::Truncated {
                expected: Self::ENCODED_LEN,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let session_id = r.read_u32()?;
        let schema_version = r.read_u16()?;
        r.expect_zero(2)?;
        let server_capabilities = r.read_u32()?;
        r.finish()?;
        Ok(Self {
            session_id,
            schema_version,
            server_capabilities,
        })
    }
}

/// Out-of-order async / GenUI slot chunk payload carried in `NveWireKind::SlotChunk`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveWireSlotChunk<'a> {
    pub slot_id: u32,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub data: &'a [u8],
}

impl<'a> NveWireSlotChunk<'a> {
    pub const HEADER_LEN: usize = 12;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = ByteWriter::with_capacity(Self::HEADER_LEN + self.data.len());
        w.write_u32(self.slot_id);
        w.write_u32(self.chunk_index);
        w.write_u32(self.total_chunks);
        w.write_raw(self.data);
        w.into_bytes()
    }

    pub fn decode(bytes: &'a [u8]) -> Result<Self, NveWireError> {
        if bytes.len() < Self::HEADER_LEN {
            return Err(NveWireError::Truncated {
                expected: Self::HEADER_LEN,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let slot_id = r.read_u32()?;
        let chunk_index = r.read_u32()?;
        let total_chunks = r.read_u32()?;
        let data = r.remaining_slice();
        Ok(Self {
            slot_id,
            chunk_index,
            total_chunks,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_detail_clips_on_utf8_char_boundary() {
        let detail = "€".repeat(100);
        assert_eq!(detail.len(), 300);
        let bytes = NveWireRejectReason::Unauthorized.encode(&detail);
        let (reason, decoded) = NveWireRejectReason::decode(&bytes).expect("must decode valid UTF-8");
        assert_eq!(reason, NveWireRejectReason::Unauthorized);
        assert_eq!(decoded.len(), 255);
        assert_eq!(decoded, "€".repeat(85));
    }
}
