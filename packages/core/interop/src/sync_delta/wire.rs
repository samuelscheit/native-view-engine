use std::fmt;

use crate::error::InteropError;

/// 4-byte wire magic identifying NVE Delta Sync v1 packets.
pub const SYNC_DELTA_MAGIC: [u8; 4] = *b"NVD1";

/// Current wire protocol version.
pub const SYNC_DELTA_VERSION: u8 = 1;

/// Fixed packet header size in bytes (8-byte aligned).
pub const DELTA_HEADER_SIZE: usize = 32;

/// Hard cap on decoded frame size (16 MiB) preventing allocation bombs from corrupt/fuzzed packets.
pub const MAX_DECODED_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Default bounded retransmit / reorder window size.
pub const DEFAULT_SYNC_WINDOW: usize = 64;

/// Flag bit indicating this packet is a retransmission.
pub const FLAG_RETRANSMIT: u16 = 1 << 0;

/// FNV-1a 64-bit hash over a single byte slice.
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    fnv1a_64_pair(bytes, &[])
}

/// Streaming FNV-1a 64-bit hash over two contiguous slices (`header` + `payload`).
#[inline]
pub fn fnv1a_64_pair(a: &[u8], b: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET_BASIS;
    for &byte in a.iter().chain(b.iter()) {
        h ^= u64::from(byte);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Wire packet kind discriminator.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketKind {
    /// Complete frame bytes (sent on initial sync or when smaller than delta).
    Keyframe = 1,
    /// Byte-level XOR + zero-run RLE delta against `base_seq`.
    XorRleDelta = 2,
    /// `NVE1` CommandBuffer op-level copy-run / literal-op delta against `base_seq`.
    CommandOpDelta = 3,
    /// Cumulative ACK + 64-bit selective ACK bitmap (`ack_seq`, `sack_bitmap`).
    Ack = 4,
    /// Explicit negative acknowledgement requesting retransmission of `missing_seq`.
    Nack = 5,
}

impl PacketKind {
    #[inline]
    pub const fn from_u8(v: u8) -> Result<Self, SyncDeltaError> {
        match v {
            1 => Ok(Self::Keyframe),
            2 => Ok(Self::XorRleDelta),
            3 => Ok(Self::CommandOpDelta),
            4 => Ok(Self::Ack),
            5 => Ok(Self::Nack),
            other => Err(SyncDeltaError::InvalidPacketKind(other)),
        }
    }
}

/// Typed error returned by binary delta encoding, decoding, and session framing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncDeltaError {
    Truncated,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u8),
    InvalidPacketKind(u8),
    ChecksumMismatch { expected: u64, actual: u64 },
    LengthMismatch { declared: usize, actual: usize },
    FrameTooLarge(usize),
    BaseSequenceMismatch { expected: u32, actual: u32 },
    CorruptPayload(&'static str),
}

impl fmt::Display for SyncDeltaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated delta packet or payload"),
            Self::InvalidMagic(m) => write!(f, "invalid delta magic: {m:?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported delta version: {v}"),
            Self::InvalidPacketKind(k) => write!(f, "invalid delta packet kind: {k}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "FNV-1a checksum mismatch: expected 0x{expected:016x}, got 0x{actual:016x}"
            ),
            Self::LengthMismatch { declared, actual } => write!(
                f,
                "payload length mismatch: declared {declared} bytes, actual {actual} bytes"
            ),
            Self::FrameTooLarge(n) => {
                write!(f, "decoded frame size {n} exceeds limit {MAX_DECODED_FRAME_BYTES}")
            }
            Self::BaseSequenceMismatch { expected, actual } => write!(
                f,
                "base sequence mismatch: expected seq {expected}, packet references {actual}"
            ),
            Self::CorruptPayload(msg) => write!(f, "corrupt delta payload: {msg}"),
        }
    }
}

impl std::error::Error for SyncDeltaError {}

impl From<SyncDeltaError> for InteropError {
    fn from(err: SyncDeltaError) -> Self {
        Self::SyncDelta(err.to_string())
    }
}

/// Decoded fixed 32-byte packet header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaPacketHeader {
    pub version: u8,
    pub kind: PacketKind,
    pub flags: u16,
    pub seq: u32,
    pub base_seq: u32,
    pub ack_seq: u32,
    pub payload_len: u32,
    pub checksum: u64,
}

/// Zero-copy validated view borrowing directly from an incoming wire packet slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaPacketRef<'a> {
    header: DeltaPacketHeader,
    payload: &'a [u8],
}

impl<'a> DeltaPacketRef<'a> {
    /// Parses and validates a packet in zero-copy fashion (checking magic, version, kind,
    /// exact length, and 64-bit FNV-1a checksum).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, SyncDeltaError> {
        if bytes.len() < DELTA_HEADER_SIZE {
            return Err(SyncDeltaError::Truncated);
        }
        let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
        if magic != SYNC_DELTA_MAGIC {
            return Err(SyncDeltaError::InvalidMagic(magic));
        }
        let version = bytes[4];
        if version != SYNC_DELTA_VERSION {
            return Err(SyncDeltaError::UnsupportedVersion(version));
        }
        let kind = PacketKind::from_u8(bytes[5])?;
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        let seq = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let base_seq = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let ack_seq = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let payload_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let checksum = u64::from_le_bytes(bytes[24..32].try_into().unwrap());

        let declared = payload_len as usize;
        let actual = bytes.len() - DELTA_HEADER_SIZE;
        if actual != declared {
            return Err(SyncDeltaError::LengthMismatch { declared, actual });
        }
        let payload = &bytes[DELTA_HEADER_SIZE..];
        let computed = fnv1a_64_pair(&bytes[0..24], payload);
        if computed != checksum {
            return Err(SyncDeltaError::ChecksumMismatch {
                expected: checksum,
                actual: computed,
            });
        }

        Ok(Self {
            header: DeltaPacketHeader {
                version,
                kind,
                flags,
                seq,
                base_seq,
                ack_seq,
                payload_len,
                checksum,
            },
            payload,
        })
    }

    #[inline]
    pub const fn header(&self) -> DeltaPacketHeader {
        self.header
    }

    #[inline]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

/// Encodes a framed packet with its 32-byte header and FNV-1a checksum into `out`.
pub fn write_packet_into(
    kind: PacketKind,
    flags: u16,
    seq: u32,
    base_seq: u32,
    ack_seq: u32,
    payload: &[u8],
    out: &mut Vec<u8>,
) {
    out.clear();
    out.reserve(DELTA_HEADER_SIZE + payload.len());
    out.extend_from_slice(&SYNC_DELTA_MAGIC);
    out.push(SYNC_DELTA_VERSION);
    out.push(kind as u8);
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&base_seq.to_le_bytes());
    out.extend_from_slice(&ack_seq.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let checksum = fnv1a_64_pair(&out[0..24], payload);
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(payload);
}

/// Builds a framed packet as a newly allocated `Vec<u8>`.
pub fn encode_packet(
    kind: PacketKind,
    flags: u16,
    seq: u32,
    base_seq: u32,
    ack_seq: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(DELTA_HEADER_SIZE + payload.len());
    write_packet_into(kind, flags, seq, base_seq, ack_seq, payload, &mut out);
    out
}

/// Re-stamps `FLAG_RETRANSMIT` on an existing valid packet and recomputes its header checksum.
pub(super) fn mark_packet_retransmit(packet: &[u8]) -> Vec<u8> {
    let mut out = packet.to_vec();
    if out.len() >= DELTA_HEADER_SIZE {
        let flags = u16::from_le_bytes(out[6..8].try_into().unwrap()) | FLAG_RETRANSMIT;
        out[6..8].copy_from_slice(&flags.to_le_bytes());
        let checksum = fnv1a_64_pair(&out[0..24], &out[DELTA_HEADER_SIZE..]);
        out[24..32].copy_from_slice(&checksum.to_le_bytes());
    }
    out
}

// =============================================================================
// LEB128 varint helpers
// =============================================================================

#[inline]
pub(super) fn write_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

#[inline]
pub(super) fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, SyncDeltaError> {
    let mut shift = 0u32;
    let mut result = 0u64;
    for _ in 0..10 {
        let Some(&b) = bytes.get(*pos) else {
            return Err(SyncDeltaError::Truncated);
        };
        *pos += 1;
        let low = u64::from(b & 0x7F);
        if shift == 63 && low > 1 {
            return Err(SyncDeltaError::CorruptPayload("varint u64 overflow"));
        }
        result |= low << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
    Err(SyncDeltaError::CorruptPayload("unterminated varint"))
}
