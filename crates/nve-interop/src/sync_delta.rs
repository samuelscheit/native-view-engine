//! Zero-copy binary frame-delta wire protocol for live NVE Studio pairing (`< 10 KB/s`).
//!
//! Designed for QUIC datagrams, WebRTC DataChannels, and WebSocket transports:
//! - Fixed 32-byte little-endian header (`b"NVD1"`) with 64-bit FNV-1a integrity checksum over
//!   header + payload and zero-copy packet validation (`DeltaPacketRef<'a>`).
//! - Dual compact encodings automatically selecting the smallest wire representation per frame:
//!   1. **XOR + RLE** (`PacketKind::XorRleDelta`): zero-run RLE over `prev ^ next` with LEB128
//!      varints for byte-level edits to state or frame buffers.
//!   2. **`CommandBuffer` Changed-Op Delta** (`PacketKind::CommandOpDelta`): scans `NVE1` command
//!      boundaries in zero-copy slices, collapses unchanged op runs into 3-byte `CopyRun` tokens,
//!      and ships ONLY the raw bytes of changed/inserted commands — immune to mid-buffer byte shifts!
//! - Reliable sequence numbers + cumulative/selective ACK (`SACK` 64-bit bitmap) + NACK +
//!   bounded retransmit window + out-of-order receiver reorder buffer.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;

use nve_abi::{NVE_ABI_MAGIC, NVE_ABI_VERSION};

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

/// NVE CommandBuffer fixed header length (`nve_abi::codec::HEADER_SIZE`).
const NVE_CMD_HEADER_SIZE: usize = 24;

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
fn mark_packet_retransmit(packet: &[u8]) -> Vec<u8> {
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
fn write_varint(mut v: u64, out: &mut Vec<u8>) {
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
fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, SyncDeltaError> {
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

// =============================================================================
// Codec A: XOR + Zero-Run RLE Delta
// =============================================================================

/// Encodes `next` as a compact XOR + zero-run RLE delta relative to `base`.
///
/// Token encoding:
/// - `0x01..=0x7F`: `tag` unchanged bytes (`base[i] ^ next[i] == 0`)
/// - `0x00` + varint `n`: extended run of `n` unchanged bytes
/// - `0x81..=0xFF`: `tag - 0x80` XOR literal bytes
/// - `0x80` + varint `n` + `n` bytes: extended run of `n` XOR literal bytes
pub fn encode_xor_rle_delta(base: &[u8], next: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    encode_xor_rle_delta_into(base, next, &mut out);
    out
}

/// Zero-allocation-reuse version of [`encode_xor_rle_delta`].
pub fn encode_xor_rle_delta_into(base: &[u8], next: &[u8], out: &mut Vec<u8>) {
    out.clear();
    write_varint(next.len() as u64, out);

    let len = next.len();
    let mut i = 0usize;

    while i < len {
        // Count unchanged run where (base_byte ^ next[i]) == 0.
        let zero_start = i;
        while i < len {
            let b = base.get(i).copied().unwrap_or(0);
            if (b ^ next[i]) != 0 {
                break;
            }
            i += 1;
        }
        let zero_run = i - zero_start;
        if zero_run > 0 {
            if zero_run <= 127 {
                out.push(zero_run as u8);
            } else {
                out.push(0x00);
                write_varint(zero_run as u64, out);
            }
        }

        if i >= len {
            break;
        }

        // Count changed run; stop early if we encounter >= 2 consecutive zero-XOR bytes so
        // RLE stays maximally compact.
        let lit_start = i;
        while i < len {
            let b0 = base.get(i).copied().unwrap_or(0);
            if (b0 ^ next[i]) == 0 {
                let next_also_zero = i + 1 < len
                    && (base.get(i + 1).copied().unwrap_or(0) ^ next[i + 1]) == 0;
                if next_also_zero || i + 1 == len {
                    break;
                }
            }
            i += 1;
        }
        let lit_len = i - lit_start;
        if lit_len <= 127 {
            out.push(0x80 | (lit_len as u8));
        } else {
            out.push(0x80);
            write_varint(lit_len as u64, out);
        }
        for idx in lit_start..i {
            let b = base.get(idx).copied().unwrap_or(0);
            out.push(b ^ next[idx]);
        }
    }
}

/// Applies an XOR+RLE delta payload onto `base`, writing the reconstructed frame into `out`.
pub fn decode_xor_rle_delta_into(
    base: &[u8],
    delta_payload: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), SyncDeltaError> {
    let mut pos = 0usize;
    let target_len_u64 = read_varint(delta_payload, &mut pos)?;
    let target_len = usize::try_from(target_len_u64)
        .map_err(|_| SyncDeltaError::FrameTooLarge(usize::MAX))?;
    if target_len > MAX_DECODED_FRAME_BYTES {
        return Err(SyncDeltaError::FrameTooLarge(target_len));
    }

    out.clear();
    out.reserve(target_len);

    while out.len() < target_len {
        let Some(&tag) = delta_payload.get(pos) else {
            return Err(SyncDeltaError::Truncated);
        };
        pos += 1;

        if (tag & 0x80) == 0 {
            let run_len = if tag == 0 {
                usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("zero-run length overflow"))?
            } else {
                usize::from(tag)
            };
            if run_len == 0 || out.len().saturating_add(run_len) > target_len {
                return Err(SyncDeltaError::CorruptPayload("zero-run exceeds target length"));
            }
            let start = out.len();
            let end = start + run_len;
            let from_base_end = end.min(base.len());
            if start < from_base_end {
                out.extend_from_slice(&base[start..from_base_end]);
            }
            let pad_zeros = end.saturating_sub(from_base_end.max(start));
            if pad_zeros > 0 {
                out.resize(out.len() + pad_zeros, 0);
            }
        } else {
            let lit_len = if tag == 0x80 {
                usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("literal length overflow"))?
            } else {
                usize::from(tag & 0x7F)
            };
            if lit_len == 0 || out.len().saturating_add(lit_len) > target_len {
                return Err(SyncDeltaError::CorruptPayload("literal run exceeds target length"));
            }
            if delta_payload.len().saturating_sub(pos) < lit_len {
                return Err(SyncDeltaError::Truncated);
            }
            let base_offset = out.len();
            let xor_bytes = &delta_payload[pos..pos + lit_len];
            pos += lit_len;
            for (k, &xb) in xor_bytes.iter().enumerate() {
                let b = base.get(base_offset + k).copied().unwrap_or(0);
                out.push(b ^ xb);
            }
        }
    }

    if pos != delta_payload.len() {
        return Err(SyncDeltaError::CorruptPayload("trailing bytes after xor-rle delta"));
    }
    Ok(())
}

/// Decodes an XOR+RLE delta payload onto `base`, returning the reconstructed bytes.
pub fn decode_xor_rle_delta(base: &[u8], delta_payload: &[u8]) -> Result<Vec<u8>, SyncDeltaError> {
    let mut out = Vec::new();
    decode_xor_rle_delta_into(base, delta_payload, &mut out)?;
    Ok(out)
}

// =============================================================================
// Codec B: NVE1 CommandBuffer Changed-Op Delta (Zero-Copy Boundary Scanner)
// =============================================================================

#[derive(Debug, Clone, Copy)]
struct RawCmdHeader {
    flags: u16,
    surface_id: u32,
    frame_id: u64,
    op_count: u32,
}

/// Scans an `NVE1` `CommandBuffer` in a single zero-copy pass and returns its header plus the
/// byte range `(start, end)` of every encoded op. Returns `None` if `bytes` is not a valid `NVE1`
/// command buffer.
fn scan_nve_command_slices(bytes: &[u8]) -> Option<(RawCmdHeader, Vec<(usize, usize)>)> {
    if bytes.len() < NVE_CMD_HEADER_SIZE {
        return None;
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    if magic != NVE_ABI_MAGIC {
        return None;
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().ok()?);
    if version != NVE_ABI_VERSION {
        return None;
    }
    let flags = u16::from_le_bytes(bytes[6..8].try_into().ok()?);
    let surface_id = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let frame_id = u64::from_le_bytes(bytes[12..20].try_into().ok()?);
    let op_count = u32::from_le_bytes(bytes[20..24].try_into().ok()?);

    let mut pos = NVE_CMD_HEADER_SIZE;
    let mut ranges = Vec::with_capacity((op_count as usize).min(4096));

    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let end = pos.checked_add(n)?;
        if end > bytes.len() {
            return None;
        }
        let s = &bytes[*pos..end];
        *pos = end;
        Some(s)
    };
    let read_u16 = |pos: &mut usize| -> Option<u16> {
        Some(u16::from_le_bytes(take(pos, 2)?.try_into().ok()?))
    };
    let read_u32 = |pos: &mut usize| -> Option<u32> {
        Some(u32::from_le_bytes(take(pos, 4)?.try_into().ok()?))
    };

    for _ in 0..op_count {
        let start = pos;
        let op = take(&mut pos, 1)?[0];
        match op {
            // CreateNode: u32 node_id + u16 kind + u16 name_len + name_bytes
            1 => {
                take(&mut pos, 6)?;
                let name_len = usize::from(read_u16(&mut pos)?);
                take(&mut pos, name_len)?;
            }
            // RemoveNode: u32 node_id
            2 => {
                take(&mut pos, 4)?;
            }
            // SetChildren: u32 parent_id + u32 count + count*u32
            3 => {
                take(&mut pos, 4)?;
                let count = read_u32(&mut pos)? as usize;
                take(&mut pos, count.checked_mul(4)?)?;
            }
            // SetText: u32 node_id + u32 len + text_bytes
            4 => {
                take(&mut pos, 4)?;
                let len = read_u32(&mut pos)? as usize;
                take(&mut pos, len)?;
            }
            // SetLayout: u32 node_id + 6*(u8+f32=5) + 3*u8 + 3*f32 + 2*(4*f32) = 4+30+3+12+32 = 81
            5 => {
                take(&mut pos, 81)?;
            }
            // SetStyle: u32 node_id + 3*u32 + 4*f32 + i32 = 4+12+16+4 = 36
            6 => {
                take(&mut pos, 36)?;
            }
            // SetRawProp: u32 node_id + u16 key_len + key + u8 val_tag + val
            7 => {
                take(&mut pos, 4)?;
                let key_len = usize::from(read_u16(&mut pos)?);
                take(&mut pos, key_len)?;
                let vtag = take(&mut pos, 1)?[0];
                match vtag {
                    0 => {}
                    1 => {
                        take(&mut pos, 1)?;
                    }
                    2 | 3 => {
                        take(&mut pos, 8)?;
                    }
                    4 | 5 => {
                        let len = read_u32(&mut pos)? as usize;
                        take(&mut pos, len)?;
                    }
                    _ => return None,
                }
            }
            // SetEventMask: u32 node_id + u64 mask
            8 => {
                take(&mut pos, 12)?;
            }
            // DispatchCommand: u32 node_id + u16 name_len + name + u32 payload_len + payload
            9 => {
                take(&mut pos, 4)?;
                let nlen = usize::from(read_u16(&mut pos)?);
                take(&mut pos, nlen)?;
                let plen = read_u32(&mut pos)? as usize;
                take(&mut pos, plen)?;
            }
            // Commit: u8 sync_mount
            10 => {
                take(&mut pos, 1)?;
            }
            _ => return None,
        }
        ranges.push((start, pos));
    }

    if pos != bytes.len() {
        return None;
    }

    Some((
        RawCmdHeader {
            flags,
            surface_id,
            frame_id,
            op_count,
        },
        ranges,
    ))
}

const CMD_DELTA_COPY_RUN: u8 = 1;
const CMD_DELTA_LITERAL_OP: u8 = 2;
const CMD_DELTA_XOR_OP: u8 = 3;

/// Extracts a stable `(opcode, node_id)` identity key from a raw encoded `NVE1` command slice.
///
/// Used by [`encode_command_op_delta`] so a modified `SetStyle`, `SetLayout`, or `SetText` on the
/// same node can be encoded as a tiny per-op XOR+RLE patch instead of re-sending the entire op.
#[inline]
fn raw_op_identity(op_slice: &[u8]) -> Option<(u8, u32)> {
    let opcode = *op_slice.first()?;
    if opcode == 10 {
        // Commit has no node_id.
        return Some((opcode, 0));
    }
    if op_slice.len() < 5 {
        return None;
    }
    let node_id = u32::from_le_bytes(op_slice[1..5].try_into().ok()?);
    Some((opcode, node_id))
}

/// Encodes an op-level delta between two valid `NVE1` `CommandBuffer`s.
///
/// - Identical op runs collapse into compact `CopyRun(prev_idx, count)` tokens (3–5 bytes).
/// - Modified ops targeting the same `(opcode, node_id)` emit a per-op `XorOp(prev_idx, xor_rle)`
///   whenever smaller than the raw op (shrinking `SetStyle`/`SetLayout`/`SetText` edits by 3–5x!).
/// - New ops emit only their raw slice via `LiteralOp`.
///
/// Returns `None` if either buffer is not a valid `NVE1` `CommandBuffer`.
pub fn encode_command_op_delta(base: &[u8], next: &[u8]) -> Option<Vec<u8>> {
    let (_, prev_ranges) = scan_nve_command_slices(base)?;
    let (next_hdr, next_ranges) = scan_nve_command_slices(next)?;

    // Index previous op slices by exact FNV-1a hash AND by `(opcode, node_id)` for per-op XOR patching.
    let mut prev_exact: HashMap<u64, Vec<usize>> = HashMap::with_capacity(prev_ranges.len());
    let mut prev_by_node: HashMap<(u8, u32), usize> = HashMap::with_capacity(prev_ranges.len());
    for (idx, &(s, e)) in prev_ranges.iter().enumerate() {
        let slice = &base[s..e];
        let h = fnv1a_64(slice);
        let entry = prev_exact.entry(h).or_default();
        if entry.len() < 4 {
            entry.push(idx);
        }
        if let Some(key) = raw_op_identity(slice) {
            prev_by_node.insert(key, idx);
        }
    }

    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&next_hdr.flags.to_le_bytes());
    write_varint(u64::from(next_hdr.surface_id), &mut out);
    write_varint(next_hdr.frame_id, &mut out);
    write_varint(u64::from(next_hdr.op_count), &mut out);

    let mut scratch_xor = Vec::with_capacity(64);
    let mut scratch_token = Vec::with_capacity(64);

    let mut ni = 0usize;
    let mut expected_prev = 0usize;
    let n_next = next_ranges.len();
    let n_prev = prev_ranges.len();

    while ni < n_next {
        let (ns, ne) = next_ranges[ni];
        let nslice = &next[ns..ne];

        // Fast path: check `expected_prev` first for sequential runs (O(1) no hash lookup).
        let matched_prev = if expected_prev < n_prev {
            let (ps, pe) = prev_ranges[expected_prev];
            if &base[ps..pe] == nslice {
                Some(expected_prev)
            } else {
                None
            }
        } else {
            None
        }
        .or_else(|| {
            let h = fnv1a_64(nslice);
            prev_exact.get(&h).and_then(|cands| {
                cands.iter().copied().find(|&pidx| {
                    let (ps, pe) = prev_ranges[pidx];
                    &base[ps..pe] == nslice
                })
            })
        });

        if let Some(pstart) = matched_prev {
            let mut run = 1usize;
            while ni + run < n_next && pstart + run < n_prev {
                let (ns2, ne2) = next_ranges[ni + run];
                let (ps2, pe2) = prev_ranges[pstart + run];
                if next[ns2..ne2] != base[ps2..pe2] {
                    break;
                }
                run += 1;
            }
            out.push(CMD_DELTA_COPY_RUN);
            write_varint(pstart as u64, &mut out);
            write_varint(run as u64, &mut out);
            ni += run;
            expected_prev = pstart + run;
        } else {
            // Try per-op XOR+RLE against the previous op on the same `(opcode, node_id)`.
            let mut emitted_xor = false;
            if let Some(key) = raw_op_identity(nslice) {
                if let Some(&pidx) = prev_by_node.get(&key) {
                    let (ps, pe) = prev_ranges[pidx];
                    encode_xor_rle_delta_into(&base[ps..pe], nslice, &mut scratch_xor);
                    scratch_token.clear();
                    scratch_token.push(CMD_DELTA_XOR_OP);
                    write_varint(pidx as u64, &mut scratch_token);
                    write_varint(scratch_xor.len() as u64, &mut scratch_token);
                    scratch_token.extend_from_slice(&scratch_xor);

                    // Compare against LiteralOp size: 1 + varint(len) + len
                    let lit_overhead = 1 + if nslice.len() < 128 { 1 } else { 2 } + nslice.len();
                    if scratch_token.len() < lit_overhead {
                        out.extend_from_slice(&scratch_token);
                        emitted_xor = true;
                        expected_prev = pidx + 1;
                    }
                }
            }
            if !emitted_xor {
                out.push(CMD_DELTA_LITERAL_OP);
                write_varint(nslice.len() as u64, &mut out);
                out.extend_from_slice(nslice);
                expected_prev = expected_prev.saturating_add(1);
            }
            ni += 1;
        }
    }

    Some(out)
}

/// Applies a `CommandOpDelta` payload onto a base `NVE1` `CommandBuffer`, reconstructing the exact
/// target `CommandBuffer` byte-for-byte.
pub fn decode_command_op_delta(base: &[u8], delta_payload: &[u8]) -> Result<Vec<u8>, SyncDeltaError> {
    let (_, prev_ranges) = scan_nve_command_slices(base)
        .ok_or(SyncDeltaError::CorruptPayload("base frame is not a valid NVE1 CommandBuffer"))?;

    let mut pos = 0usize;
    if delta_payload.len() < 2 {
        return Err(SyncDeltaError::Truncated);
    }
    let flags = u16::from_le_bytes(delta_payload[0..2].try_into().unwrap());
    pos += 2;

    let surface_id = u32::try_from(read_varint(delta_payload, &mut pos)?)
        .map_err(|_| SyncDeltaError::CorruptPayload("surface_id overflow"))?;
    let frame_id = read_varint(delta_payload, &mut pos)?;
    let op_count = u32::try_from(read_varint(delta_payload, &mut pos)?)
        .map_err(|_| SyncDeltaError::CorruptPayload("op_count overflow"))?;

    let mut out = Vec::with_capacity(base.len().min(64 * 1024).max(NVE_CMD_HEADER_SIZE));
    out.extend_from_slice(&NVE_ABI_MAGIC.to_le_bytes());
    out.extend_from_slice(&NVE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&surface_id.to_le_bytes());
    out.extend_from_slice(&frame_id.to_le_bytes());
    out.extend_from_slice(&op_count.to_le_bytes());

    let mut xor_scratch = Vec::with_capacity(64);
    let mut emitted_ops = 0u32;
    while pos < delta_payload.len() {
        let tag = delta_payload[pos];
        pos += 1;
        match tag {
            CMD_DELTA_COPY_RUN => {
                let start_idx = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("copy_run start overflow"))?;
                let count = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("copy_run count overflow"))?;
                if count == 0 || start_idx.saturating_add(count) > prev_ranges.len() {
                    return Err(SyncDeltaError::CorruptPayload("copy_run out of bounds"));
                }
                for &(ps, pe) in &prev_ranges[start_idx..start_idx + count] {
                    if out.len().saturating_add(pe - ps) > MAX_DECODED_FRAME_BYTES {
                        return Err(SyncDeltaError::FrameTooLarge(out.len() + (pe - ps)));
                    }
                    out.extend_from_slice(&base[ps..pe]);
                    emitted_ops = emitted_ops.saturating_add(1);
                }
            }
            CMD_DELTA_LITERAL_OP => {
                let len = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("literal op len overflow"))?;
                if len == 0 || delta_payload.len().saturating_sub(pos) < len {
                    return Err(SyncDeltaError::Truncated);
                }
                if out.len().saturating_add(len) > MAX_DECODED_FRAME_BYTES {
                    return Err(SyncDeltaError::FrameTooLarge(out.len() + len));
                }
                out.extend_from_slice(&delta_payload[pos..pos + len]);
                pos += len;
                emitted_ops = emitted_ops.saturating_add(1);
            }
            CMD_DELTA_XOR_OP => {
                let prev_idx = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("xor_op prev_idx overflow"))?;
                let &(ps, pe) = prev_ranges
                    .get(prev_idx)
                    .ok_or(SyncDeltaError::CorruptPayload("xor_op prev_idx out of bounds"))?;
                let xor_len = usize::try_from(read_varint(delta_payload, &mut pos)?)
                    .map_err(|_| SyncDeltaError::CorruptPayload("xor_op len overflow"))?;
                if xor_len == 0 || delta_payload.len().saturating_sub(pos) < xor_len {
                    return Err(SyncDeltaError::Truncated);
                }
                decode_xor_rle_delta_into(
                    &base[ps..pe],
                    &delta_payload[pos..pos + xor_len],
                    &mut xor_scratch,
                )?;
                pos += xor_len;
                if out.len().saturating_add(xor_scratch.len()) > MAX_DECODED_FRAME_BYTES {
                    return Err(SyncDeltaError::FrameTooLarge(out.len() + xor_scratch.len()));
                }
                out.extend_from_slice(&xor_scratch);
                emitted_ops = emitted_ops.saturating_add(1);
            }
            _ => {
                return Err(SyncDeltaError::CorruptPayload("unknown command delta op tag"));
            }
        }
    }

    if emitted_ops != op_count {
        return Err(SyncDeltaError::CorruptPayload("reconstructed op_count mismatch"));
    }
    Ok(out)
}

// =============================================================================
// Reliable Live Pairing Session (Sequence / ACK / SACK / NACK / Retransmit)
// =============================================================================

/// Live bandwidth & compression telemetry for a [`SyncDeltaSession`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncMetrics {
    pub frames_sent: u64,
    pub keyframes_sent: u64,
    pub xor_rle_deltas_sent: u64,
    pub command_op_deltas_sent: u64,
    pub retransmits_sent: u64,
    pub wire_bytes_sent: u64,
    pub raw_bytes_sent: u64,
    pub last_packet_bytes: usize,
}

impl SyncMetrics {
    /// Average wire bytes per delta packet (excluding initial keyframes).
    pub fn avg_delta_bytes(&self) -> f64 {
        let delta_count = self.xor_rle_deltas_sent + self.command_op_deltas_sent;
        if delta_count == 0 {
            return self.last_packet_bytes as f64;
        }
        // Conservative average over all non-retransmit packets if needed, or total wire / frames.
        self.wire_bytes_sent as f64 / self.frames_sent.max(1) as f64
    }

    /// Estimated steady-state bandwidth in KB/s at `edits_per_sec` based on `bytes_per_frame`.
    pub fn estimated_kbps(bytes_per_frame: usize, edits_per_sec: f64) -> f64 {
        (bytes_per_frame as f64 * edits_per_sec) / 1024.0
    }
}

#[derive(Debug, Clone)]
struct SentEntry {
    seq: u32,
    packet: Vec<u8>,
    last_sent_ms: u64,
}

/// Result of feeding one incoming wire packet into [`SyncDeltaSession::receive_packet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOutcome {
    /// Zero or more frames applied in strict sequence order `(seq, reconstructed_frame_bytes)`.
    pub applied_frames: Vec<(u32, Vec<u8>)>,
    /// Compact ACK packet to send back to the peer (carrying cumulative `ack_seq` + 64-bit SACK).
    pub ack_packet: Option<Vec<u8>>,
    /// Any NACK packets requesting missing sequence numbers detected by a gap.
    pub nack_packets: Vec<Vec<u8>>,
}

/// Stateful bidirectional binary delta sync session for live Studio pairing over QUIC,
/// WebRTC DataChannels, or WebSockets.
#[derive(Debug, Clone)]
pub struct SyncDeltaSession {
    max_window: usize,
    // Sender state
    next_seq: u32,
    last_sent_seq: u32,
    last_sent_frame: Vec<u8>,
    sent_window: VecDeque<SentEntry>,
    metrics: SyncMetrics,
    // Receiver state
    last_applied_seq: u32,
    last_applied_frame: Vec<u8>,
    reorder_buf: BTreeMap<u32, Vec<u8>>,
}

impl Default for SyncDeltaSession {
    fn default() -> Self {
        Self::new(DEFAULT_SYNC_WINDOW)
    }
}

impl SyncDeltaSession {
    pub fn new(max_window: usize) -> Self {
        Self {
            max_window: max_window.clamp(4, 1024),
            next_seq: 1,
            last_sent_seq: 0,
            last_sent_frame: Vec::new(),
            sent_window: VecDeque::new(),
            metrics: SyncMetrics::default(),
            last_applied_seq: 0,
            last_applied_frame: Vec::new(),
            reorder_buf: BTreeMap::new(),
        }
    }

    pub fn metrics(&self) -> SyncMetrics {
        self.metrics
    }

    pub fn in_flight_count(&self) -> usize {
        self.sent_window.len()
    }

    pub fn last_applied_seq(&self) -> u32 {
        self.last_applied_seq
    }

    pub fn last_applied_frame(&self) -> &[u8] {
        &self.last_applied_frame
    }

    pub fn max_window(&self) -> usize {
        self.max_window
    }

    pub fn last_sent_seq(&self) -> u32 {
        self.last_sent_seq
    }

    pub fn last_sent_frame(&self) -> &[u8] {
        &self.last_sent_frame
    }

    pub fn reorder_buffer_len(&self) -> usize {
        self.reorder_buf.len()
    }

    /// Returns the earliest missing sequence number when out-of-order packets are buffered.
    pub fn next_missing_seq(&self) -> Option<u32> {
        if self.reorder_buf.is_empty() {
            None
        } else {
            Some(self.last_applied_seq.wrapping_add(1))
        }
    }

    /// Resets receiver sequence tracking (used when re-pairing to a fresh server epoch).
    pub fn reset_receiver(&mut self) {
        self.last_applied_seq = 0;
        self.last_applied_frame.clear();
        self.reorder_buf.clear();
    }

    /// Builds an explicit NACK packet requesting `missing_seq`.
    pub fn build_nack_packet(&self, missing_seq: u32) -> Vec<u8> {
        encode_packet(
            PacketKind::Nack,
            0,
            0,
            0,
            self.last_applied_seq,
            &missing_seq.to_le_bytes(),
        )
    }

    /// Builds an up-to-date cumulative ACK + 64-bit SACK packet.
    pub fn build_current_ack_packet(&self) -> Vec<u8> {
        self.build_ack_packet()
    }

    /// Encodes `frame_bytes`, automatically choosing the smallest of `Keyframe`, `XorRleDelta`,
    /// or `CommandOpDelta`, and inserts the packet into the bounded retransmit window.
    pub fn encode_frame(&mut self, frame_bytes: &[u8], now_ms: u64) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);

        let (kind, base_seq, payload) = if self.last_sent_seq == 0 || self.last_sent_frame.is_empty()
        {
            (PacketKind::Keyframe, 0, frame_bytes.to_vec())
        } else {
            let xor_delta = encode_xor_rle_delta(&self.last_sent_frame, frame_bytes);
            let cmd_delta = encode_command_op_delta(&self.last_sent_frame, frame_bytes);

            let mut best_kind = PacketKind::XorRleDelta;
            let mut best_payload = xor_delta;

            if let Some(cd) = cmd_delta {
                if cd.len() < best_payload.len() {
                    best_kind = PacketKind::CommandOpDelta;
                    best_payload = cd;
                }
            }

            if frame_bytes.len() <= best_payload.len() {
                (PacketKind::Keyframe, 0, frame_bytes.to_vec())
            } else {
                (best_kind, self.last_sent_seq, best_payload)
            }
        };

        let packet = encode_packet(kind, 0, seq, base_seq, self.last_applied_seq, &payload);

        self.last_sent_seq = seq;
        self.last_sent_frame.clear();
        self.last_sent_frame.extend_from_slice(frame_bytes);

        match kind {
            PacketKind::Keyframe => self.metrics.keyframes_sent += 1,
            PacketKind::XorRleDelta => self.metrics.xor_rle_deltas_sent += 1,
            PacketKind::CommandOpDelta => self.metrics.command_op_deltas_sent += 1,
            _ => {}
        }
        self.metrics.frames_sent += 1;
        self.metrics.raw_bytes_sent += frame_bytes.len() as u64;
        self.metrics.wire_bytes_sent += packet.len() as u64;
        self.metrics.last_packet_bytes = packet.len();

        self.push_sent_window(seq, packet.clone(), now_ms);
        packet
    }

    /// Forces a standalone keyframe packet (useful after prolonged packet loss or new peer join).
    pub fn encode_keyframe(&mut self, frame_bytes: &[u8], now_ms: u64) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
        let packet = encode_packet(
            PacketKind::Keyframe,
            0,
            seq,
            0,
            self.last_applied_seq,
            frame_bytes,
        );
        self.last_sent_seq = seq;
        self.last_sent_frame.clear();
        self.last_sent_frame.extend_from_slice(frame_bytes);
        self.metrics.frames_sent += 1;
        self.metrics.keyframes_sent += 1;
        self.metrics.raw_bytes_sent += frame_bytes.len() as u64;
        self.metrics.wire_bytes_sent += packet.len() as u64;
        self.metrics.last_packet_bytes = packet.len();
        self.push_sent_window(seq, packet.clone(), now_ms);
        packet
    }

    fn push_sent_window(&mut self, seq: u32, packet: Vec<u8>, now_ms: u64) {
        if self.sent_window.len() >= self.max_window {
            self.sent_window.pop_front();
        }
        self.sent_window.push_back(SentEntry {
            seq,
            packet,
            last_sent_ms: now_ms,
        });
    }

    /// Processes an incoming ACK packet (cumulative `ack_seq` + 64-bit SACK bitmap), evicting
    /// acknowledged packets from the retransmit window. Returns the number of evicted packets.
    pub fn handle_ack(&mut self, packet_bytes: &[u8]) -> Result<usize, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();
        if hdr.kind != PacketKind::Ack {
            return Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8));
        }
        if parsed.payload().len() != 8 {
            return Err(SyncDeltaError::CorruptPayload("ACK payload must be 8-byte SACK bitmap"));
        }
        let sack_bitmap = u64::from_le_bytes(parsed.payload().try_into().unwrap());
        let cum_ack = hdr.ack_seq;

        let before = self.sent_window.len();
        self.sent_window.retain(|entry| {
            if entry.seq <= cum_ack {
                return false;
            }
            let diff = entry.seq - cum_ack;
            if (1..=64).contains(&diff) && (sack_bitmap & (1u64 << (diff - 1))) != 0 {
                return false;
            }
            true
        });
        Ok(before - self.sent_window.len())
    }

    /// Processes an incoming NACK packet requesting `missing_seq`.
    ///
    /// Returns the retransmitted packet (with `FLAG_RETRANSMIT`) if still in the window, or a
    /// fresh `Keyframe` of the latest state if the missing packet aged out of the window.
    pub fn handle_nack(
        &mut self,
        packet_bytes: &[u8],
        now_ms: u64,
    ) -> Result<Option<Vec<u8>>, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();
        if hdr.kind != PacketKind::Nack {
            return Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8));
        }
        if parsed.payload().len() != 4 {
            return Err(SyncDeltaError::CorruptPayload("NACK payload must be 4-byte missing_seq"));
        }
        let missing_seq = u32::from_le_bytes(parsed.payload().try_into().unwrap());

        if let Some(entry) = self.sent_window.iter_mut().find(|e| e.seq == missing_seq) {
            entry.last_sent_ms = now_ms;
            self.metrics.retransmits_sent += 1;
            let retx = mark_packet_retransmit(&entry.packet);
            self.metrics.wire_bytes_sent += retx.len() as u64;
            return Ok(Some(retx));
        }

        // Missing packet fell out of the retransmit window -> recover cleanly with a fresh Keyframe.
        if !self.last_sent_frame.is_empty() {
            let latest = self.last_sent_frame.clone();
            let kf = self.encode_keyframe(&latest, now_ms);
            self.metrics.retransmits_sent += 1;
            return Ok(Some(kf));
        }
        Ok(None)
    }

    /// Returns any unacknowledged packets whose retransmission timer (`rto_ms`) has elapsed.
    pub fn poll_retransmits(&mut self, now_ms: u64, rto_ms: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for entry in &mut self.sent_window {
            if now_ms.saturating_sub(entry.last_sent_ms) >= rto_ms {
                entry.last_sent_ms = now_ms;
                self.metrics.retransmits_sent += 1;
                let retx = mark_packet_retransmit(&entry.packet);
                self.metrics.wire_bytes_sent += retx.len() as u64;
                out.push(retx);
            }
        }
        out
    }

    /// Receives and validates one wire packet on the peer side, applying it immediately if in
    /// order or buffering it in the bounded reorder window if out of order.
    pub fn receive_packet(&mut self, packet_bytes: &[u8]) -> Result<ReceiveOutcome, SyncDeltaError> {
        let parsed = DeltaPacketRef::parse(packet_bytes)?;
        let hdr = parsed.header();

        match hdr.kind {
            PacketKind::Ack => {
                let _ = self.handle_ack(packet_bytes)?;
                return Ok(ReceiveOutcome {
                    applied_frames: Vec::new(),
                    ack_packet: None,
                    nack_packets: Vec::new(),
                });
            }
            PacketKind::Nack => {
                return Ok(ReceiveOutcome {
                    applied_frames: Vec::new(),
                    ack_packet: None,
                    nack_packets: Vec::new(),
                });
            }
            PacketKind::Keyframe | PacketKind::XorRleDelta | PacketKind::CommandOpDelta => {}
        }

        // Ignore duplicate / already-applied sequence numbers, still returning an up-to-date ACK.
        if hdr.seq <= self.last_applied_seq {
            return Ok(ReceiveOutcome {
                applied_frames: Vec::new(),
                ack_packet: Some(self.build_ack_packet()),
                nack_packets: Vec::new(),
            });
        }

        let mut applied_frames = Vec::new();
        let mut nack_packets = Vec::new();

        let expected_next = self.last_applied_seq + 1;
        if hdr.kind == PacketKind::Keyframe && hdr.seq >= expected_next {
            // A fresh keyframe resets the baseline immediately even after a gap.
            let frame = parsed.payload().to_vec();
            self.last_applied_seq = hdr.seq;
            self.last_applied_frame = frame.clone();
            applied_frames.push((hdr.seq, frame));
            // Drop stale buffered packets older than the keyframe.
            self.reorder_buf.retain(|&s, _| s > self.last_applied_seq);
            self.drain_reorder_buffer(&mut applied_frames)?;
        } else if hdr.seq == expected_next {
            let frame = self.apply_single_packet(parsed)?;
            self.last_applied_seq = hdr.seq;
            self.last_applied_frame = frame.clone();
            applied_frames.push((hdr.seq, frame));
            self.drain_reorder_buffer(&mut applied_frames)?;
        } else {
            // Out-of-order delta packet: buffer up to `max_window` and emit NACKs for the gap.
            if self.reorder_buf.len() < self.max_window {
                self.reorder_buf.insert(hdr.seq, packet_bytes.to_vec());
            }
            let gap_end = hdr.seq.min(expected_next.saturating_add(8));
            for missing in expected_next..gap_end {
                if !self.reorder_buf.contains_key(&missing) {
                    nack_packets.push(encode_packet(
                        PacketKind::Nack,
                        0,
                        0,
                        0,
                        self.last_applied_seq,
                        &missing.to_le_bytes(),
                    ));
                }
            }
        }

        Ok(ReceiveOutcome {
            applied_frames,
            ack_packet: Some(self.build_ack_packet()),
            nack_packets,
        })
    }

    fn apply_single_packet(&self, pkt: DeltaPacketRef<'_>) -> Result<Vec<u8>, SyncDeltaError> {
        let hdr = pkt.header();
        match hdr.kind {
            PacketKind::Keyframe => Ok(pkt.payload().to_vec()),
            PacketKind::XorRleDelta => {
                if hdr.base_seq != self.last_applied_seq {
                    return Err(SyncDeltaError::BaseSequenceMismatch {
                        expected: self.last_applied_seq,
                        actual: hdr.base_seq,
                    });
                }
                decode_xor_rle_delta(&self.last_applied_frame, pkt.payload())
            }
            PacketKind::CommandOpDelta => {
                if hdr.base_seq != self.last_applied_seq {
                    return Err(SyncDeltaError::BaseSequenceMismatch {
                        expected: self.last_applied_seq,
                        actual: hdr.base_seq,
                    });
                }
                decode_command_op_delta(&self.last_applied_frame, pkt.payload())
            }
            PacketKind::Ack | PacketKind::Nack => Err(SyncDeltaError::InvalidPacketKind(hdr.kind as u8)),
        }
    }

    fn drain_reorder_buffer(
        &mut self,
        applied: &mut Vec<(u32, Vec<u8>)>,
    ) -> Result<(), SyncDeltaError> {
        while let Some(raw) = self.reorder_buf.remove(&(self.last_applied_seq + 1)) {
            let pkt = DeltaPacketRef::parse(&raw)?;
            let frame = self.apply_single_packet(pkt)?;
            let seq = pkt.header().seq;
            self.last_applied_seq = seq;
            self.last_applied_frame = frame.clone();
            applied.push((seq, frame));
        }
        Ok(())
    }

    fn build_ack_packet(&self) -> Vec<u8> {
        let mut sack = 0u64;
        for &buffered_seq in self.reorder_buf.keys() {
            if buffered_seq > self.last_applied_seq {
                let diff = buffered_seq - self.last_applied_seq;
                if (1..=64).contains(&diff) {
                    sack |= 1u64 << (diff - 1);
                }
            }
        }
        encode_packet(
            PacketKind::Ack,
            0,
            0,
            0,
            self.last_applied_seq,
            &sack.to_le_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::{
        ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexDirection,
        LayoutProps, StyleProps, ROOT_NODE_ID,
    };

    fn make_studio_like_command_buffer(frame_id: u64, accent: ColorRgba, status_text: &str) -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, frame_id);
        b.create_node(2, ComponentKind::View, None);
        b.set_layout(
            2,
            &LayoutProps {
                width: Dimension::Percent(100.0),
                height: Dimension::Percent(100.0),
                flex_direction: FlexDirection::Row,
                padding: EdgeInsets::all(12.0),
                gap: 8.0,
                ..Default::default()
            },
        );
        let mut children = Vec::new();
        for i in 0..40u32 {
            let nid = 10 + i;
            children.push(nid);
            b.create_node(nid, ComponentKind::View, None);
            b.set_layout(
                nid,
                &LayoutProps {
                    width: Dimension::Percent(100.0),
                    height: Dimension::Points(44.0),
                    ..Default::default()
                },
            );
            b.set_style(
                nid,
                &StyleProps {
                    background: if i == 0 {
                        accent
                    } else {
                        ColorRgba::rgb(30, 41, 59)
                    },
                    border_radius: 12.0,
                    font_size: 12.0,
                    ..Default::default()
                },
            );
        }
        b.create_node(99, ComponentKind::Text, None);
        b.set_text(99, status_text);
        children.push(99);
        b.set_children(2, &children);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        b.into_bytes()
    }

    #[test]
    fn incremental_studio_edits_stay_under_256_bytes_and_under_10_kbps() {
        let mut sender = SyncDeltaSession::default();
        let mut receiver = SyncDeltaSession::default();

        // Frame 1: Full Studio tree keyframe (~6 KB raw).
        let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "Todo");
        assert!(f1.len() > 4_000, "expected realistic multi-KB Studio frame");

        let p1 = sender.encode_frame(&f1, 0);
        let r1 = receiver.receive_packet(&p1).unwrap();
        assert_eq!(r1.applied_frames.len(), 1);
        assert_eq!(r1.applied_frames[0].1, f1);
        sender.handle_ack(r1.ack_packet.as_ref().unwrap()).unwrap();
        assert_eq!(sender.in_flight_count(), 0);

        // Frame 2: Incremental Studio edit (task status text changed + accent toggled!).
        // Even with variable-length string shift ("Todo" -> "In Progress" shifts all subsequent bytes!),
        // CommandOpDelta encodes ONLY the 2 changed ops + CopyRun tokens!
        let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(167, 139, 250), "In Progress");
        let p2 = sender.encode_frame(&f2, 16);
        let parsed_p2 = DeltaPacketRef::parse(&p2).unwrap();
        assert_eq!(parsed_p2.header().kind, PacketKind::CommandOpDelta);
        assert!(
            p2.len() < 256,
            "incremental Studio edit wire packet must be < 256B, got {} B (raw {} B)",
            p2.len(),
            f2.len()
        );

        let kbps_30 = SyncMetrics::estimated_kbps(p2.len(), 30.0);
        let kbps_60 = SyncMetrics::estimated_kbps(p2.len(), 60.0);
        assert!(
            kbps_60 < 10.0,
            "expected < 10 KB/s at 60 edits/sec, got {kbps_60:.2} KB/s ({} B/frame)",
            p2.len()
        );

        let r2 = receiver.receive_packet(&p2).unwrap();
        assert_eq!(r2.applied_frames.len(), 1);
        assert_eq!(r2.applied_frames[0].1, f2);

        // Frame 3: Single-field status cycle ("In Progress" -> "Done").
        let f3 = make_studio_like_command_buffer(3, ColorRgba::rgb(167, 139, 250), "Done");
        let p3 = sender.encode_frame(&f3, 33);
        assert!(
            p3.len() < 160,
            "single-field edit should be < 160B, got {} B",
            p3.len()
        );
        let r3 = receiver.receive_packet(&p3).unwrap();
        assert_eq!(r3.applied_frames[0].1, f3);

        eprintln!(
            "[sync_delta] raw={} B | accent+status delta={} B ({:.2} KB/s @30fps, {:.2} KB/s @60fps) | status-only delta={} B",
            f2.len(),
            p2.len(),
            kbps_30,
            kbps_60,
            p3.len()
        );
    }

    #[test]
    fn xor_rle_handles_growth_shrink_identical_and_arbitrary_binary_blobs() {
        let base = vec![0xABu8; 8192];

        // 1. Identical payload -> tiny delta (target_len varint + 1 extended zero-run)
        let d_same = encode_xor_rle_delta(&base, &base);
        assert!(d_same.len() <= 8, "identical 8KB delta was {}B", d_same.len());
        assert_eq!(decode_xor_rle_delta(&base, &d_same).unwrap(), base);

        // 2. Mutate 4 bytes in the middle of 8KB
        let mut modified = base.clone();
        modified[4000] = 0x11;
        modified[4001] = 0x22;
        modified[4002] = 0x33;
        modified[4003] = 0x44;
        let d_mod = encode_xor_rle_delta(&base, &modified);
        assert!(d_mod.len() < 24, "4-byte delta in 8KB was {}B", d_mod.len());
        assert_eq!(decode_xor_rle_delta(&base, &d_mod).unwrap(), modified);

        // 3. Growing buffer
        let mut grown = base.clone();
        grown.extend_from_slice(b"appended-tail-data");
        let d_grow = encode_xor_rle_delta(&base, &grown);
        assert_eq!(decode_xor_rle_delta(&base, &d_grow).unwrap(), grown);

        // 4. Shrinking buffer to 0 bytes
        let d_empty = encode_xor_rle_delta(&base, &[]);
        assert_eq!(decode_xor_rle_delta(&base, &d_empty).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn out_of_order_delivery_nack_retransmit_and_rto_window_recovery() {
        let mut sender = SyncDeltaSession::new(8);
        let mut receiver = SyncDeltaSession::new(8);

        let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "S1");
        let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(56, 189, 248), "S2");
        let f3 = make_studio_like_command_buffer(3, ColorRgba::rgb(56, 189, 248), "S3");

        let p1 = sender.encode_frame(&f1, 100);
        let _p2 = sender.encode_frame(&f2, 116);
        let p3 = sender.encode_frame(&f3, 132);
        assert_eq!(sender.in_flight_count(), 3);

        // Deliver p1, drop p2, deliver p3 out of order!
        let r1 = receiver.receive_packet(&p1).unwrap();
        assert_eq!(r1.applied_frames.len(), 1);
        sender.handle_ack(r1.ack_packet.as_ref().unwrap()).unwrap();
        assert_eq!(sender.in_flight_count(), 2);

        let r3 = receiver.receive_packet(&p3).unwrap();
        assert!(r3.applied_frames.is_empty(), "p3 must wait for missing p2");
        assert_eq!(r3.nack_packets.len(), 1, "expected NACK for seq 2");

        // Selective ACK for p3 evicts p3 from sender's window while keeping p2!
        sender.handle_ack(r3.ack_packet.as_ref().unwrap()).unwrap();
        assert_eq!(sender.in_flight_count(), 1);

        // Sender handles NACK for seq 2 -> retransmits with FLAG_RETRANSMIT set.
        let retx2 = sender
            .handle_nack(&r3.nack_packets[0], 150)
            .unwrap()
            .expect("expected retransmit for seq 2");
        let parsed_retx = DeltaPacketRef::parse(&retx2).unwrap();
        assert_eq!(parsed_retx.header().seq, 2);
        assert_ne!(parsed_retx.header().flags & FLAG_RETRANSMIT, 0);

        // Delivering retx2 applies BOTH seq 2 AND buffered seq 3 in strict order!
        let r2 = receiver.receive_packet(&retx2).unwrap();
        assert_eq!(r2.applied_frames.len(), 2);
        assert_eq!(r2.applied_frames[0], (2, f2));
        assert_eq!(r2.applied_frames[1], (3, f3));
        assert_eq!(receiver.last_applied_seq(), 3);
        sender.handle_ack(r2.ack_packet.as_ref().unwrap()).unwrap();
        assert_eq!(sender.in_flight_count(), 0);

        // RTO poll triggers retransmit when unacked packet ages past RTO.
        let f4 = make_studio_like_command_buffer(4, ColorRgba::rgb(34, 197, 94), "S4");
        let _p4 = sender.encode_frame(&f4, 200);
        assert!(sender.poll_retransmits(220, 50).is_empty());
        let timed_out = sender.poll_retransmits(260, 50);
        assert!(!timed_out.is_empty());
        assert!(sender.metrics().retransmits_sent >= 2);
    }

    #[test]
    fn fuzz_and_corruption_guards_reject_bitflips_truncation_and_oom_bombs() {
        let mut sender = SyncDeltaSession::default();
        let f1 = make_studio_like_command_buffer(1, ColorRgba::rgb(56, 189, 248), "Alpha");
        let f2 = make_studio_like_command_buffer(2, ColorRgba::rgb(245, 158, 11), "Beta");
        let _ = sender.encode_frame(&f1, 0);
        let p2 = sender.encode_frame(&f2, 16);

        // 1. Every single bit flip in the packet must fail validation cleanly.
        for byte_idx in 0..p2.len() {
            let mut mutated = p2.clone();
            mutated[byte_idx] ^= 0x01;
            assert!(
                DeltaPacketRef::parse(&mutated).is_err(),
                "bit flip at byte {byte_idx} was not detected!"
            );
        }

        // 2. Every prefix truncation must fail cleanly.
        for cut in 0..p2.len() {
            assert!(DeltaPacketRef::parse(&p2[..cut]).is_err());
        }

        // 3. Trailing byte appended must fail cleanly.
        let mut extended = p2.clone();
        extended.push(0x00);
        assert!(matches!(
            DeltaPacketRef::parse(&extended),
            Err(SyncDeltaError::LengthMismatch { .. })
        ));

        // 4. Forged valid-checksum OOM-bomb XOR+RLE payload declaring 1 GiB target_len.
        let mut bomb_payload = Vec::new();
        write_varint(1024 * 1024 * 1024, &mut bomb_payload);
        let bomb_pkt = encode_packet(PacketKind::XorRleDelta, 0, 2, 1, 0, &bomb_payload);
        let parsed = DeltaPacketRef::parse(&bomb_pkt).unwrap();
        assert!(matches!(
            decode_xor_rle_delta(&f1, parsed.payload()),
            Err(SyncDeltaError::FrameTooLarge(_))
        ));

        // 5. Deterministic pseudo-random fuzz corpus against decoders (zero panics!).
        let mut seed: u64 = 0xDEAD_BEEF_CAFE_BABEu64;
        for _ in 0..256 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = ((seed >> 48) as usize) % 96;
            let mut buf = vec![0u8; len];
            for b in &mut buf {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                *b = (seed >> 56) as u8;
            }
            let _ = DeltaPacketRef::parse(&buf);
            let _ = decode_xor_rle_delta(&f1, &buf);
            let _ = decode_command_op_delta(&f1, &buf);
        }
    }
}
