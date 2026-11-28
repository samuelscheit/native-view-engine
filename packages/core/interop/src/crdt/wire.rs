//! Binary CRDT delta wire format (`NveCrdtDelta`) with `XXH64` integrity verification.
//!
//! Layout (little-endian): `magic("NVE_CRDT")` | `version(u16=2)` | `sender(u64)` |
//! `vv_len(u32)` + `(peer:u64, seq:u64)*` | `record_count(u32)` + `(rec_len:u32, tag:u8, body)*` |
//! `checksum(u64 XXH64 over all preceding bytes)`.

use std::fmt;

use nve_abi::xxh64;
use nve_codec::{ByteReader, ByteWriter, CodecError, NveDecode, NveEncode};

use super::hlc::{HlcTimestamp, OpId, PeerId, VersionVector};

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

impl From<CodecError> for CrdtWireError {
    #[inline]
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::Truncated { needed, remaining } => Self::Truncated {
                expected: needed,
                actual: remaining,
            },
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::TrailingBytes(n) => Self::TrailingBytes(n),
            CodecError::BadBool(_) => Self::InvalidPayload("boolean byte must be 0 or 1"),
            CodecError::LenOverflow { len, max } => Self::LimitExceeded {
                kind: "field_bytes",
                count: len,
                max,
            },
            CodecError::DepthExceeded { max } => Self::LimitExceeded {
                kind: "nesting_depth",
                count: max.saturating_add(1),
                max,
            },
            CodecError::BadMagic
            | CodecError::BadVersion(_)
            | CodecError::BadEnum { .. }
            | CodecError::NonZeroReserved(_) => Self::InvalidPayload("corrupt CRDT wire field"),
        }
    }
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

        ByteWriter::with_buf(out, |w| {
            w.write_raw(&NVE_CRDT_DELTA_MAGIC);
            w.write_u16(NVE_CRDT_DELTA_VERSION);
            self.sender.encode(w)?;

            w.write_u32(self.version_vector.len() as u32);
            for (peer, seq) in self.version_vector.iter() {
                peer.encode(w)?;
                w.write_u64(seq);
            }

            w.write_u32(self.records.len() as u32);
            for rec in &self.records {
                let len_pos = w.len();
                w.write_u32(0);
                let body_start = w.len();
                encode_record(rec, w)?;
                let body_len = w.len() - body_start;
                check_limit("record_bytes", body_len, MAX_FIELD_BYTES)?;
                w.patch_u32(len_pos, body_len as u32)?;
            }

            check_limit("wire_bytes", w.len() + 8, MAX_WIRE_BYTES)?;
            let checksum = crdt_xxh64(w.as_slice());
            w.write_u64(checksum);
            Ok(())
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CrdtWireError> {
        check_limit("wire_bytes", bytes.len(), MAX_WIRE_BYTES)?;
        if bytes.len() < NVE_CRDT_DELTA_MIN_LEN {
            return Err(CrdtWireError::Truncated {
                expected: NVE_CRDT_DELTA_MIN_LEN,
                actual: bytes.len(),
            });
        }

        let payload_end = bytes.len() - 8;
        let mut r = ByteReader::new(&bytes[..payload_end]);
        let magic = r.read_array::<8>()?;
        if magic != NVE_CRDT_DELTA_MAGIC {
            return Err(CrdtWireError::InvalidMagic(magic));
        }
        let version = r.read_u16()?;
        if version != NVE_CRDT_DELTA_VERSION {
            return Err(CrdtWireError::UnsupportedVersion(version));
        }

        let expected = ByteReader::new(&bytes[payload_end..]).read_u64()?;
        let actual = crdt_xxh64(&bytes[..payload_end]);
        if actual != expected {
            return Err(CrdtWireError::ChecksumMismatch { expected, actual });
        }

        let sender = PeerId::decode(&mut r)?;

        let vv_len = r.read_u32()? as usize;
        check_limit("version_vector", vv_len, MAX_VV_ENTRIES)?;
        r.ensure_remaining(vv_len.saturating_mul(16))?;
        let mut version_vector = VersionVector::new();
        for _ in 0..vv_len {
            let peer = PeerId::decode(&mut r)?;
            let seq = r.read_u64()?;
            version_vector.set_max(peer, seq);
        }

        let record_count = r.read_u32()? as usize;
        check_limit("records", record_count, MAX_RECORDS)?;
        r.ensure_remaining(record_count.saturating_mul(5))?;
        let mut records = Vec::with_capacity(record_count);
        for _ in 0..record_count {
            let rec_len = r.read_u32()? as usize;
            check_limit("record_bytes", rec_len, MAX_FIELD_BYTES)?;
            let mut rec_r = ByteReader::new(r.read_exact(rec_len)?);
            let rec = decode_record(&mut rec_r)?;
            rec_r.finish()?;
            records.push(rec);
        }

        r.finish()?;

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

fn encode_bytes(bytes: &[u8], w: &mut ByteWriter) -> Result<(), CrdtWireError> {
    check_limit("field_bytes", bytes.len(), MAX_FIELD_BYTES)?;
    w.write_bytes(bytes)?;
    Ok(())
}

fn encode_value(val: &CrdtValue, w: &mut ByteWriter) -> Result<(), CrdtWireError> {
    match val {
        CrdtValue::Unit => w.write_u8(VAL_UNIT),
        CrdtValue::Bool(b) => {
            w.write_u8(VAL_BOOL);
            w.write_bool(*b);
        }
        CrdtValue::U32(v) => {
            w.write_u8(VAL_U32);
            w.write_u32(*v);
        }
        CrdtValue::U64(v) => {
            w.write_u8(VAL_U64);
            w.write_u64(*v);
        }
        CrdtValue::I64(v) => {
            w.write_u8(VAL_I64);
            w.write_i64(*v);
        }
        CrdtValue::Utf8(s) => {
            w.write_u8(VAL_UTF8);
            encode_bytes(s.as_bytes(), w)?;
        }
        CrdtValue::Bytes(b) => {
            w.write_u8(VAL_BYTES);
            encode_bytes(b, w)?;
        }
    }
    Ok(())
}

fn decode_value(r: &mut ByteReader<'_>) -> Result<CrdtValue, CrdtWireError> {
    match r.read_u8()? {
        VAL_UNIT => Ok(CrdtValue::Unit),
        VAL_BOOL => Ok(CrdtValue::Bool(r.read_bool()?)),
        VAL_U32 => Ok(CrdtValue::U32(r.read_u32()?)),
        VAL_U64 => Ok(CrdtValue::U64(r.read_u64()?)),
        VAL_I64 => Ok(CrdtValue::I64(r.read_i64()?)),
        VAL_UTF8 => Ok(CrdtValue::Utf8(read_bounded_string(r)?)),
        VAL_BYTES => Ok(CrdtValue::Bytes(r.read_bytes(MAX_FIELD_BYTES)?.to_vec())),
        other => Err(CrdtWireError::UnknownValueTag(other)),
    }
}

fn read_bounded_string(r: &mut ByteReader<'_>) -> Result<String, CrdtWireError> {
    let len = r.read_u32()? as usize;
    check_limit("string_bytes", len, MAX_FIELD_BYTES)?;
    let slice = r.read_exact(len)?;
    std::str::from_utf8(slice)
        .map(str::to_owned)
        .map_err(|_| CrdtWireError::InvalidUtf8)
}

fn encode_record(rec: &CrdtRecord, w: &mut ByteWriter) -> Result<(), CrdtWireError> {
    match rec {
        CrdtRecord::LwwSet {
            namespace,
            entity_id,
            field_id,
            op_id,
            timestamp,
            value,
        } => {
            w.write_u8(TAG_LWW_SET);
            w.write_u16(*namespace);
            w.write_u64(*entity_id);
            w.write_u16(*field_id);
            op_id.encode(w)?;
            timestamp.encode(w)?;
            encode_value(value, w)?;
        }
        CrdtRecord::GCounterSet {
            namespace,
            entity_id,
            peer,
            op_id,
            count,
        } => {
            w.write_u8(TAG_GCOUNTER_SET);
            w.write_u16(*namespace);
            w.write_u64(*entity_id);
            peer.encode(w)?;
            op_id.encode(w)?;
            w.write_u64(*count);
        }
        CrdtRecord::PnCounterSet {
            namespace,
            entity_id,
            peer,
            op_id,
            pos,
            neg,
        } => {
            w.write_u8(TAG_PNCOUNTER_SET);
            w.write_u16(*namespace);
            w.write_u64(*entity_id);
            peer.encode(w)?;
            op_id.encode(w)?;
            w.write_u64(*pos);
            w.write_u64(*neg);
        }
        CrdtRecord::OrSetAddU64 {
            namespace,
            key,
            op_id,
        } => {
            w.write_u8(TAG_ORSET_ADD_U64);
            w.write_u16(*namespace);
            w.write_u64(*key);
            op_id.encode(w)?;
        }
        CrdtRecord::OrSetRemove {
            namespace,
            removed_tag,
            remove_op,
        } => {
            w.write_u8(TAG_ORSET_REMOVE);
            w.write_u16(*namespace);
            removed_tag.encode(w)?;
            remove_op.encode(w)?;
        }
        CrdtRecord::RgaInsert {
            namespace,
            list_id,
            id,
            after_id,
            value,
        } => {
            w.write_u8(TAG_RGA_INSERT);
            w.write_u16(*namespace);
            w.write_u64(*list_id);
            id.encode(w)?;
            after_id.encode(w)?;
            encode_value(value, w)?;
        }
        CrdtRecord::RgaDelete {
            namespace,
            list_id,
            target_id,
            remove_op,
        } => {
            w.write_u8(TAG_RGA_DELETE);
            w.write_u16(*namespace);
            w.write_u64(*list_id);
            target_id.encode(w)?;
            remove_op.encode(w)?;
        }
        CrdtRecord::OrSetAddString {
            namespace,
            key,
            op_id,
        } => {
            w.write_u8(TAG_ORSET_ADD_STR);
            w.write_u16(*namespace);
            encode_bytes(key.as_bytes(), w)?;
            op_id.encode(w)?;
        }
    }
    Ok(())
}

fn decode_record(r: &mut ByteReader<'_>) -> Result<CrdtRecord, CrdtWireError> {
    match r.read_u8()? {
        TAG_LWW_SET => Ok(CrdtRecord::LwwSet {
            namespace: r.read_u16()?,
            entity_id: r.read_u64()?,
            field_id: r.read_u16()?,
            op_id: OpId::decode(r)?,
            timestamp: HlcTimestamp::decode(r)?,
            value: decode_value(r)?,
        }),
        TAG_GCOUNTER_SET => Ok(CrdtRecord::GCounterSet {
            namespace: r.read_u16()?,
            entity_id: r.read_u64()?,
            peer: PeerId::decode(r)?,
            op_id: OpId::decode(r)?,
            count: r.read_u64()?,
        }),
        TAG_PNCOUNTER_SET => Ok(CrdtRecord::PnCounterSet {
            namespace: r.read_u16()?,
            entity_id: r.read_u64()?,
            peer: PeerId::decode(r)?,
            op_id: OpId::decode(r)?,
            pos: r.read_u64()?,
            neg: r.read_u64()?,
        }),
        TAG_ORSET_ADD_U64 => Ok(CrdtRecord::OrSetAddU64 {
            namespace: r.read_u16()?,
            key: r.read_u64()?,
            op_id: OpId::decode(r)?,
        }),
        TAG_ORSET_REMOVE => Ok(CrdtRecord::OrSetRemove {
            namespace: r.read_u16()?,
            removed_tag: OpId::decode(r)?,
            remove_op: OpId::decode(r)?,
        }),
        TAG_RGA_INSERT => {
            let namespace = r.read_u16()?;
            let list_id = r.read_u64()?;
            let id = OpId::decode(r)?;
            let after_id = r
                .read_opt(OpId::decode)
                .map_err(|e| match e {
                    CodecError::BadBool(_) => {
                        CrdtWireError::InvalidPayload("RGA anchor flag must be 0 or 1")
                    }
                    other => CrdtWireError::from(other),
                })?;
            Ok(CrdtRecord::RgaInsert {
                namespace,
                list_id,
                id,
                after_id,
                value: decode_value(r)?,
            })
        }
        TAG_RGA_DELETE => Ok(CrdtRecord::RgaDelete {
            namespace: r.read_u16()?,
            list_id: r.read_u64()?,
            target_id: OpId::decode(r)?,
            remove_op: OpId::decode(r)?,
        }),
        TAG_ORSET_ADD_STR => Ok(CrdtRecord::OrSetAddString {
            namespace: r.read_u16()?,
            key: read_bounded_string(r)?,
            op_id: OpId::decode(r)?,
        }),
        other => Err(CrdtWireError::UnknownOpTag(other)),
    }
}
