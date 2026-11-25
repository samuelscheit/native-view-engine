//! Binary CRDT delta wire format (`NveCrdtDelta`) with `XXH64` integrity verification.
//!
//! Layout (little-endian): `magic("NVE_CRDT")` | `version(u16=2)` | `sender(u64)` |
//! `vv_len(u32)` + `(peer:u64, seq:u64)*` | `record_count(u32)` + `(rec_len:u32, tag:u8, body)*` |
//! `checksum(u64 XXH64 over all preceding bytes)`.

use std::fmt;

use nve_engine::xxh64;

use super::hlc::{HLC_ENCODED_LEN, HlcTimestamp, OP_ID_ENCODED_LEN, OpId, PeerId, VersionVector};

/// 8-byte self-documenting ASCII magic identifying `NveCrdtDelta` binary frames.
pub const NVE_CRDT_DELTA_MAGIC: [u8; 8] = *b"NVE_CRDT";
/// Current `NveCrdtDelta` wire format version (`2`, with `XXH64` checksum).
pub const NVE_CRDT_DELTA_VERSION: u16 = 2;
/// Domain-separated `XXH64` seed for `NveCrdtDelta` integrity checksums.
pub const NVE_CRDT_XXH64_SEED: u64 = 0x4E56_455F_4352_4454; // "NVE_CRDT"
/// Minimum valid frame size: magic(8) + ver(2) + sender(8) + vv_len(4) + rec_count(4) + cksum(8).
pub const NVE_CRDT_DELTA_MIN_LEN: usize = 34;
pub const MAX_WIRE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_VV_ENTRIES: usize = 65_536;
pub const MAX_RECORDS: usize = 1_000_000;
pub const MAX_FIELD_BYTES: usize = 4 * 1024 * 1024;

const TAG_LWW_SET: u8 = 1;
const TAG_GCOUNTER_SET: u8 = 2;
const TAG_PNCOUNTER_SET: u8 = 3;
const TAG_ORSET_ADD_U64: u8 = 4;
const TAG_ORSET_REMOVE: u8 = 5;
const TAG_RGA_INSERT: u8 = 6;
const TAG_RGA_DELETE: u8 = 7;
const TAG_ORSET_ADD_STR: u8 = 8;

const VAL_UNIT: u8 = 0;
const VAL_BOOL: u8 = 1;
const VAL_U32: u8 = 2;
const VAL_U64: u8 = 3;
const VAL_I64: u8 = 4;
const VAL_UTF8: u8 = 5;
const VAL_BYTES: u8 = 6;

/// Computes the canonical `XXH64` checksum over CRDT wire bytes.
#[inline]
pub fn crdt_xxh64(bytes: &[u8]) -> u64 {
    xxh64(bytes, NVE_CRDT_XXH64_SEED)
}

/// Typed decoding/encoding errors for `NveCrdtDelta` binary frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdtWireError {
    Truncated { expected: usize, actual: usize },
    InvalidMagic([u8; 8]),
    UnsupportedVersion(u16),
    ChecksumMismatch { expected: u64, actual: u64 },
    UnknownOpTag(u8),
    UnknownValueTag(u8),
    InvalidUtf8,
    LimitExceeded { kind: &'static str, count: usize, max: usize },
    TrailingBytes(usize),
    InvalidPayload(&'static str),
}

impl fmt::Display for CrdtWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected, actual } => {
                write!(f, "NveCrdtDelta truncated: need {expected}B, have {actual}B")
            }
            Self::InvalidMagic(m) => write!(f, "NveCrdtDelta invalid magic: {m:02x?}"),
            Self::UnsupportedVersion(v) => write!(f, "NveCrdtDelta unsupported version: {v}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "NveCrdtDelta XXH64 checksum mismatch: expected {expected:#018x}, got {actual:#018x}"
                )
            }
            Self::UnknownOpTag(t) => write!(f, "NveCrdtDelta unknown op tag: {t}"),
            Self::UnknownValueTag(t) => write!(f, "NveCrdtDelta unknown value tag: {t}"),
            Self::InvalidUtf8 => write!(f, "NveCrdtDelta invalid UTF-8"),
            Self::LimitExceeded { kind, count, max } => {
                write!(f, "NveCrdtDelta limit exceeded ({kind}): {count} > {max}")
            }
            Self::TrailingBytes(n) => write!(f, "NveCrdtDelta trailing bytes: {n}"),
            Self::InvalidPayload(msg) => write!(f, "NveCrdtDelta invalid payload: {msg}"),
        }
    }
}

impl std::error::Error for CrdtWireError {}

/// Binary-encoded primitive payload for LWW registers and RGA list elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdtValue {
    Unit,
    Bool(bool),
    U32(u32),
    U64(u64),
    I64(i64),
    Utf8(String),
    Bytes(Vec<u8>),
}

/// Tagged CRDT delta operation record inside an [`NveCrdtDelta`] packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdtRecord {
    LwwSet {
        namespace: u16,
        entity_id: u64,
        field_id: u16,
        op_id: OpId,
        timestamp: HlcTimestamp,
        value: CrdtValue,
    },
    GCounterSet {
        namespace: u16,
        entity_id: u64,
        peer: PeerId,
        op_id: OpId,
        count: u64,
    },
    PnCounterSet {
        namespace: u16,
        entity_id: u64,
        peer: PeerId,
        op_id: OpId,
        pos: u64,
        neg: u64,
    },
    OrSetAddU64 {
        namespace: u16,
        key: u64,
        op_id: OpId,
    },
    OrSetRemove {
        namespace: u16,
        removed_tag: OpId,
        remove_op: OpId,
    },
    RgaInsert {
        namespace: u16,
        list_id: u64,
        id: OpId,
        after_id: Option<OpId>,
        value: CrdtValue,
    },
    RgaDelete {
        namespace: u16,
        list_id: u64,
        target_id: OpId,
        remove_op: OpId,
    },
    OrSetAddString {
        namespace: u16,
        key: String,
        op_id: OpId,
    },
}

/// Self-contained binary CRDT delta packet (`NveCrdtDelta`, `XXH64`-verified).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveCrdtDelta {
    pub sender: PeerId,
    pub version_vector: VersionVector,
    pub records: Vec<CrdtRecord>,
}

impl NveCrdtDelta {
    pub fn new(sender: PeerId, version_vector: VersionVector, records: Vec<CrdtRecord>) -> Self {
        Self {
            sender,
            version_vector,
            records,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, CrdtWireError> {
        let mut out = Vec::with_capacity(
            NVE_CRDT_DELTA_MIN_LEN + self.version_vector.len() * 16 + self.records.len() * 32,
        );
        self.encode_into(&mut out)?;
        Ok(out)
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), CrdtWireError> {
        out.clear();
        check_limit("version_vector", self.version_vector.len(), MAX_VV_ENTRIES)?;
        check_limit("records", self.records.len(), MAX_RECORDS)?;

        out.extend_from_slice(&NVE_CRDT_DELTA_MAGIC);
        out.extend_from_slice(&NVE_CRDT_DELTA_VERSION.to_le_bytes());
        out.extend_from_slice(&self.sender.to_le_bytes());

        out.extend_from_slice(&(self.version_vector.len() as u32).to_le_bytes());
        for (peer, seq) in self.version_vector.iter() {
            out.extend_from_slice(&peer.to_le_bytes());
            out.extend_from_slice(&seq.to_le_bytes());
        }

        out.extend_from_slice(&(self.records.len() as u32).to_le_bytes());
        for rec in &self.records {
            let len_pos = out.len();
            out.extend_from_slice(&0u32.to_le_bytes());
            let body_start = out.len();
            encode_record(rec, out)?;
            let body_len = out.len() - body_start;
            check_limit("record_bytes", body_len, MAX_FIELD_BYTES)?;
            out[len_pos..len_pos + 4].copy_from_slice(&(body_len as u32).to_le_bytes());
        }

        check_limit("wire_bytes", out.len() + 8, MAX_WIRE_BYTES)?;
        let checksum = crdt_xxh64(out);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CrdtWireError> {
        check_limit("wire_bytes", bytes.len(), MAX_WIRE_BYTES)?;
        if bytes.len() < NVE_CRDT_DELTA_MIN_LEN {
            return Err(CrdtWireError::Truncated {
                expected: NVE_CRDT_DELTA_MIN_LEN,
                actual: bytes.len(),
            });
        }

        let magic: [u8; 8] = bytes[0..8].try_into().expect("checked min len");
        if magic != NVE_CRDT_DELTA_MAGIC {
            return Err(CrdtWireError::InvalidMagic(magic));
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != NVE_CRDT_DELTA_VERSION {
            return Err(CrdtWireError::UnsupportedVersion(version));
        }

        let payload_end = bytes.len() - 8;
        let expected = u64::from_le_bytes(bytes[payload_end..].try_into().expect("8B checksum"));
        let actual = crdt_xxh64(&bytes[..payload_end]);
        if actual != expected {
            return Err(CrdtWireError::ChecksumMismatch { expected, actual });
        }

        let mut cur = Cursor::new(&bytes[10..payload_end]);
        let sender = PeerId(cur.read_u64()?);

        let vv_len = cur.read_u32()? as usize;
        check_limit("version_vector", vv_len, MAX_VV_ENTRIES)?;
        cur.ensure_remaining(vv_len.saturating_mul(16))?;
        let mut version_vector = VersionVector::new();
        for _ in 0..vv_len {
            let peer = PeerId(cur.read_u64()?);
            let seq = cur.read_u64()?;
            version_vector.set_max(peer, seq);
        }

        let record_count = cur.read_u32()? as usize;
        check_limit("records", record_count, MAX_RECORDS)?;
        cur.ensure_remaining(record_count.saturating_mul(5))?;
        let mut records = Vec::with_capacity(record_count);
        for _ in 0..record_count {
            let rec_len = cur.read_u32()? as usize;
            check_limit("record_bytes", rec_len, MAX_FIELD_BYTES)?;
            let mut rec_cur = Cursor::new(cur.read_slice(rec_len)?);
            let rec = decode_record(&mut rec_cur)?;
            if rec_cur.remaining() != 0 {
                return Err(CrdtWireError::TrailingBytes(rec_cur.remaining()));
            }
            records.push(rec);
        }

        if cur.remaining() != 0 {
            return Err(CrdtWireError::TrailingBytes(cur.remaining()));
        }

        Ok(Self {
            sender,
            version_vector,
            records,
        })
    }
}

#[inline]
fn check_limit(kind: &'static str, count: usize, max: usize) -> Result<(), CrdtWireError> {
    if count > max {
        Err(CrdtWireError::LimitExceeded { kind, count, max })
    } else {
        Ok(())
    }
}

fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) -> Result<(), CrdtWireError> {
    check_limit("field_bytes", bytes.len(), MAX_FIELD_BYTES)?;
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn encode_value(val: &CrdtValue, out: &mut Vec<u8>) -> Result<(), CrdtWireError> {
    match val {
        CrdtValue::Unit => out.push(VAL_UNIT),
        CrdtValue::Bool(b) => out.extend_from_slice(&[VAL_BOOL, u8::from(*b)]),
        CrdtValue::U32(v) => {
            out.push(VAL_U32);
            out.extend_from_slice(&v.to_le_bytes());
        }
        CrdtValue::U64(v) => {
            out.push(VAL_U64);
            out.extend_from_slice(&v.to_le_bytes());
        }
        CrdtValue::I64(v) => {
            out.push(VAL_I64);
            out.extend_from_slice(&v.to_le_bytes());
        }
        CrdtValue::Utf8(s) => {
            out.push(VAL_UTF8);
            encode_bytes(s.as_bytes(), out)?;
        }
        CrdtValue::Bytes(b) => {
            out.push(VAL_BYTES);
            encode_bytes(b, out)?;
        }
    }
    Ok(())
}

fn decode_value(cur: &mut Cursor<'_>) -> Result<CrdtValue, CrdtWireError> {
    match cur.read_u8()? {
        VAL_UNIT => Ok(CrdtValue::Unit),
        VAL_BOOL => match cur.read_u8()? {
            0 => Ok(CrdtValue::Bool(false)),
            1 => Ok(CrdtValue::Bool(true)),
            _ => Err(CrdtWireError::InvalidPayload("boolean byte must be 0 or 1")),
        },
        VAL_U32 => Ok(CrdtValue::U32(cur.read_u32()?)),
        VAL_U64 => Ok(CrdtValue::U64(cur.read_u64()?)),
        VAL_I64 => Ok(CrdtValue::I64(cur.read_i64()?)),
        VAL_UTF8 => Ok(CrdtValue::Utf8(cur.read_string()?)),
        VAL_BYTES => Ok(CrdtValue::Bytes(cur.read_bytes_vec()?)),
        other => Err(CrdtWireError::UnknownValueTag(other)),
    }
}

fn encode_record(rec: &CrdtRecord, out: &mut Vec<u8>) -> Result<(), CrdtWireError> {
    match rec {
        CrdtRecord::LwwSet {
            namespace,
            entity_id,
            field_id,
            op_id,
            timestamp,
            value,
        } => {
            out.push(TAG_LWW_SET);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&entity_id.to_le_bytes());
            out.extend_from_slice(&field_id.to_le_bytes());
            out.extend_from_slice(&op_id.to_le_bytes());
            out.extend_from_slice(&timestamp.encode_le());
            encode_value(value, out)?;
        }
        CrdtRecord::GCounterSet {
            namespace,
            entity_id,
            peer,
            op_id,
            count,
        } => {
            out.push(TAG_GCOUNTER_SET);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&entity_id.to_le_bytes());
            out.extend_from_slice(&peer.to_le_bytes());
            out.extend_from_slice(&op_id.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
        }
        CrdtRecord::PnCounterSet {
            namespace,
            entity_id,
            peer,
            op_id,
            pos,
            neg,
        } => {
            out.push(TAG_PNCOUNTER_SET);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&entity_id.to_le_bytes());
            out.extend_from_slice(&peer.to_le_bytes());
            out.extend_from_slice(&op_id.to_le_bytes());
            out.extend_from_slice(&pos.to_le_bytes());
            out.extend_from_slice(&neg.to_le_bytes());
        }
        CrdtRecord::OrSetAddU64 {
            namespace,
            key,
            op_id,
        } => {
            out.push(TAG_ORSET_ADD_U64);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&key.to_le_bytes());
            out.extend_from_slice(&op_id.to_le_bytes());
        }
        CrdtRecord::OrSetRemove {
            namespace,
            removed_tag,
            remove_op,
        } => {
            out.push(TAG_ORSET_REMOVE);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&removed_tag.to_le_bytes());
            out.extend_from_slice(&remove_op.to_le_bytes());
        }
        CrdtRecord::RgaInsert {
            namespace,
            list_id,
            id,
            after_id,
            value,
        } => {
            out.push(TAG_RGA_INSERT);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&list_id.to_le_bytes());
            out.extend_from_slice(&id.to_le_bytes());
            match after_id {
                None => out.push(0),
                Some(anchor) => {
                    out.push(1);
                    out.extend_from_slice(&anchor.to_le_bytes());
                }
            }
            encode_value(value, out)?;
        }
        CrdtRecord::RgaDelete {
            namespace,
            list_id,
            target_id,
            remove_op,
        } => {
            out.push(TAG_RGA_DELETE);
            out.extend_from_slice(&namespace.to_le_bytes());
            out.extend_from_slice(&list_id.to_le_bytes());
            out.extend_from_slice(&target_id.to_le_bytes());
            out.extend_from_slice(&remove_op.to_le_bytes());
        }
        CrdtRecord::OrSetAddString {
            namespace,
            key,
            op_id,
        } => {
            out.push(TAG_ORSET_ADD_STR);
            out.extend_from_slice(&namespace.to_le_bytes());
            encode_bytes(key.as_bytes(), out)?;
            out.extend_from_slice(&op_id.to_le_bytes());
        }
    }
    Ok(())
}

fn decode_record(cur: &mut Cursor<'_>) -> Result<CrdtRecord, CrdtWireError> {
    match cur.read_u8()? {
        TAG_LWW_SET => Ok(CrdtRecord::LwwSet {
            namespace: cur.read_u16()?,
            entity_id: cur.read_u64()?,
            field_id: cur.read_u16()?,
            op_id: cur.read_op_id()?,
            timestamp: cur.read_hlc()?,
            value: decode_value(cur)?,
        }),
        TAG_GCOUNTER_SET => Ok(CrdtRecord::GCounterSet {
            namespace: cur.read_u16()?,
            entity_id: cur.read_u64()?,
            peer: PeerId(cur.read_u64()?),
            op_id: cur.read_op_id()?,
            count: cur.read_u64()?,
        }),
        TAG_PNCOUNTER_SET => Ok(CrdtRecord::PnCounterSet {
            namespace: cur.read_u16()?,
            entity_id: cur.read_u64()?,
            peer: PeerId(cur.read_u64()?),
            op_id: cur.read_op_id()?,
            pos: cur.read_u64()?,
            neg: cur.read_u64()?,
        }),
        TAG_ORSET_ADD_U64 => Ok(CrdtRecord::OrSetAddU64 {
            namespace: cur.read_u16()?,
            key: cur.read_u64()?,
            op_id: cur.read_op_id()?,
        }),
        TAG_ORSET_REMOVE => Ok(CrdtRecord::OrSetRemove {
            namespace: cur.read_u16()?,
            removed_tag: cur.read_op_id()?,
            remove_op: cur.read_op_id()?,
        }),
        TAG_RGA_INSERT => {
            let namespace = cur.read_u16()?;
            let list_id = cur.read_u64()?;
            let id = cur.read_op_id()?;
            let after_id = match cur.read_u8()? {
                0 => None,
                1 => Some(cur.read_op_id()?),
                _ => return Err(CrdtWireError::InvalidPayload("RGA anchor flag must be 0 or 1")),
            };
            Ok(CrdtRecord::RgaInsert {
                namespace,
                list_id,
                id,
                after_id,
                value: decode_value(cur)?,
            })
        }
        TAG_RGA_DELETE => Ok(CrdtRecord::RgaDelete {
            namespace: cur.read_u16()?,
            list_id: cur.read_u64()?,
            target_id: cur.read_op_id()?,
            remove_op: cur.read_op_id()?,
        }),
        TAG_ORSET_ADD_STR => Ok(CrdtRecord::OrSetAddString {
            namespace: cur.read_u16()?,
            key: cur.read_string()?,
            op_id: cur.read_op_id()?,
        }),
        other => Err(CrdtWireError::UnknownOpTag(other)),
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    const fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn ensure_remaining(&self, need: usize) -> Result<(), CrdtWireError> {
        if need > self.remaining() {
            Err(CrdtWireError::Truncated {
                expected: need,
                actual: self.remaining(),
            })
        } else {
            Ok(())
        }
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8], CrdtWireError> {
        self.ensure_remaining(len)?;
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..self.pos])
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CrdtWireError> {
        Ok(self.read_slice(N)?.try_into().expect("checked N bytes"))
    }

    fn read_u8(&mut self) -> Result<u8, CrdtWireError> {
        Ok(self.read_array::<1>()?[0])
    }

    fn read_u16(&mut self) -> Result<u16, CrdtWireError> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    fn read_u32(&mut self) -> Result<u32, CrdtWireError> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    fn read_u64(&mut self) -> Result<u64, CrdtWireError> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_i64(&mut self) -> Result<i64, CrdtWireError> {
        Ok(i64::from_le_bytes(self.read_array::<8>()?))
    }

    fn read_op_id(&mut self) -> Result<OpId, CrdtWireError> {
        Ok(OpId::from_le_bytes(self.read_array::<OP_ID_ENCODED_LEN>()?))
    }

    fn read_hlc(&mut self) -> Result<HlcTimestamp, CrdtWireError> {
        Ok(HlcTimestamp::decode_le(self.read_array::<HLC_ENCODED_LEN>()?))
    }

    fn read_bytes_vec(&mut self) -> Result<Vec<u8>, CrdtWireError> {
        let len = self.read_u32()? as usize;
        check_limit("field_bytes", len, MAX_FIELD_BYTES)?;
        Ok(self.read_slice(len)?.to_vec())
    }

    fn read_string(&mut self) -> Result<String, CrdtWireError> {
        let len = self.read_u32()? as usize;
        check_limit("string_bytes", len, MAX_FIELD_BYTES)?;
        std::str::from_utf8(self.read_slice(len)?)
            .map(str::to_owned)
            .map_err(|_| CrdtWireError::InvalidUtf8)
    }
}
