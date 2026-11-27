//! Binary `NVEQ` v2 wire codec for SSR dehydration and out-of-order streaming hydration.
//!
//! Wire format (`NVEQ`, version `2`, zero `serde_json` or extra crate dependencies):
//! - `[0..4]` magic `"NVEQ"`
//! - `[4]`    version `2u8`
//! - `[5..9]` `entry_count: u32` (little-endian)
//! - For each entry:
//!   - `proc_id: u64` (LE)
//!   - `age_ms: u32` (LE)
//!   - `flags: u8`:
//!     - `bit 0` (`0b0000_0001`): `subscribed_all`
//!     - `bits 1..=2` (`(flags >> 1) & 0b11`): `entry_kind`
//!       - `0` = `Ok`: `body_len: u32` + raw CBOR `P::Output`
//!       - `1` = `DomainErr`: `status: u8` + `body_len: u32` + raw CBOR `P::Error`
//!       - `2` = `ProtocolErr`: `status: u8` + `msg_len: u32` (`<= 256`) + valid UTF-8 message
//!       - `3` = `StreamPending`: 0 payload bytes
//!     - `bits 3..=7` (`0b1111_1000`): reserved — rejected with [`DehydrateError::InvalidFlags`]
//!   - `tag_count: u16` (LE), followed by `tag_count` `u64` tag values (LE)
//!   - `args_len: u32` (LE), followed by `encoded_args` bytes
//!   - Entry-kind-specific payload bytes

use core::fmt;

use bytes::Bytes;
use nve_rpc::{ProcId, Tag64};

use crate::slab::{rpc_status_from_u8, MAX_PROTOCOL_ERR_MSG_BYTES};

const MAGIC: [u8; 4] = *b"NVEQ";
pub const NVEQ_VERSION: u8 = 2;

/// Reads the encoded entry count from an `NVEQ` v2 header in O(1) without decoding payloads.
#[inline]
pub fn nveq_entry_count(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 9 || bytes[0..4] != MAGIC || bytes[4] != NVEQ_VERSION {
        return None;
    }
    Some(u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]))
}
const MAX_HYDRATE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HYDRATE_ENTRIES: usize = 4096;
const MAX_HYDRATE_TAGS_PER_ENTRY: usize = 128;

const HYDRATE_FLAG_SUBSCRIBED_ALL: u8 = 1 << 0;
const HYDRATE_KIND_SHIFT: u8 = 1;
const HYDRATE_KIND_MASK: u8 = 0b0000_0011;
const HYDRATE_RESERVED_FLAGS_MASK: u8 = 0b1111_1000;

const ENTRY_KIND_OK: u8 = 0;
const ENTRY_KIND_DOMAIN_ERR: u8 = 1;
const ENTRY_KIND_PROTOCOL_ERR: u8 = 2;
const ENTRY_KIND_STREAM_PENDING: u8 = 3;

/// Failure returned when encoding or decoding an `NVEQ` hydration snapshot buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DehydrateError {
    TooLarge(usize),
    InvalidMagic,
    UnsupportedVersion(u8),
    Truncated,
    TooManyEntries(usize),
    TooManyTags(usize),
    TrailingBytes(usize),
    InvalidFlags(u8),
    InvalidStatus(u8),
    InvalidUtf8,
}

impl fmt::Display for DehydrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "hydration payload exceeds limit ({n} bytes)"),
            Self::InvalidMagic => f.write_str("invalid NVEQ hydration magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVEQ hydration version {v}"),
            Self::Truncated => f.write_str("truncated NVEQ hydration payload"),
            Self::TooManyEntries(n) => write!(f, "hydration payload has too many entries ({n})"),
            Self::TooManyTags(n) => write!(f, "hydration entry has too many tags ({n})"),
            Self::TrailingBytes(n) => write!(f, "hydration payload has {n} trailing byte(s)"),
            Self::InvalidFlags(bits) => {
                write!(f, "hydration entry has reserved flag bits set ({bits:#04x})")
            }
            Self::InvalidStatus(code) => {
                write!(f, "hydration error entry has invalid RpcStatus ({code})")
            }
            Self::InvalidUtf8 => f.write_str("hydration protocol error message is not valid UTF-8"),
        }
    }
}

impl std::error::Error for DehydrateError {}

pub(crate) enum DehydratePayloadRef<'a> {
    Ok(&'a [u8]),
    DomainErr { status: u8, body: &'a [u8] },
    ProtocolErr { status: u8, message: &'a str },
    StreamPending,
}

pub(crate) struct DehydrateEntryRef<'a> {
    pub proc_id: ProcId,
    pub age_ms: u32,
    pub subscribed_all: bool,
    pub tags: &'a [Tag64],
    pub encoded_args: &'a [u8],
    pub payload: DehydratePayloadRef<'a>,
}

pub(crate) enum HydratedPayload {
    Ok(Bytes),
    DomainErr { status: u8, body: Bytes },
    ProtocolErr { status: u8, message: String },
    StreamPending,
}

pub(crate) struct HydratedEntry {
    pub proc_id: ProcId,
    pub age_ms: u32,
    pub subscribed_all: bool,
    pub tags: Vec<Tag64>,
    pub encoded_args: Bytes,
    pub payload: HydratedPayload,
}

pub(crate) fn encode_hydration_entries(
    entries: &[DehydrateEntryRef<'_>],
) -> Result<Vec<u8>, DehydrateError> {
    if entries.len() > MAX_HYDRATE_ENTRIES {
        return Err(DehydrateError::TooManyEntries(entries.len()));
    }
    let mut cap = 9usize;
    for e in entries {
        if e.tags.len() > MAX_HYDRATE_TAGS_PER_ENTRY {
            return Err(DehydrateError::TooManyTags(e.tags.len()));
        }
        if u32::try_from(e.encoded_args.len()).is_err() {
            return Err(DehydrateError::TooLarge(usize::MAX));
        }
        let payload_bytes = match e.payload {
            DehydratePayloadRef::Ok(body) => {
                if u32::try_from(body.len()).is_err() {
                    return Err(DehydrateError::TooLarge(usize::MAX));
                }
                4usize.saturating_add(body.len())
            }
            DehydratePayloadRef::DomainErr { status, body } => {
                if rpc_status_from_u8(status).is_none() {
                    return Err(DehydrateError::InvalidStatus(status));
                }
                if u32::try_from(body.len()).is_err() {
                    return Err(DehydrateError::TooLarge(usize::MAX));
                }
                5usize.saturating_add(body.len())
            }
            DehydratePayloadRef::ProtocolErr { status, message } => {
                if rpc_status_from_u8(status).is_none() {
                    return Err(DehydrateError::InvalidStatus(status));
                }
                if message.len() > MAX_PROTOCOL_ERR_MSG_BYTES {
                    return Err(DehydrateError::TooLarge(message.len()));
                }
                5usize.saturating_add(message.len())
            }
            DehydratePayloadRef::StreamPending => 0,
        };
        // Fixed per-entry header: proc_id (8) + age_ms (4) + flags (1) + tag_count (2) + args_len (4) = 19B.
        cap = cap
            .saturating_add(19)
            .saturating_add(e.tags.len().saturating_mul(8))
            .saturating_add(e.encoded_args.len())
            .saturating_add(payload_bytes);
        if cap > MAX_HYDRATE_BYTES {
            return Err(DehydrateError::TooLarge(cap));
        }
    }

    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(&MAGIC);
    out.push(NVEQ_VERSION);
    let count =
        u32::try_from(entries.len()).map_err(|_| DehydrateError::TooManyEntries(entries.len()))?;
    out.extend_from_slice(&count.to_le_bytes());

    for e in entries {
        out.extend_from_slice(&e.proc_id.raw().to_le_bytes());
        out.extend_from_slice(&e.age_ms.to_le_bytes());
        let kind_bits = match e.payload {
            DehydratePayloadRef::Ok(_) => ENTRY_KIND_OK,
            DehydratePayloadRef::DomainErr { .. } => ENTRY_KIND_DOMAIN_ERR,
            DehydratePayloadRef::ProtocolErr { .. } => ENTRY_KIND_PROTOCOL_ERR,
            DehydratePayloadRef::StreamPending => ENTRY_KIND_STREAM_PENDING,
        };
        let flags = (kind_bits << HYDRATE_KIND_SHIFT)
            | if e.subscribed_all {
                HYDRATE_FLAG_SUBSCRIBED_ALL
            } else {
                0
            };
        out.push(flags);
        out.extend_from_slice(&(e.tags.len() as u16).to_le_bytes());
        for tag in e.tags {
            out.extend_from_slice(&tag.raw().to_le_bytes());
        }
        out.extend_from_slice(&(e.encoded_args.len() as u32).to_le_bytes());
        out.extend_from_slice(e.encoded_args);
        match e.payload {
            DehydratePayloadRef::Ok(body) => {
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                out.extend_from_slice(body);
            }
            DehydratePayloadRef::DomainErr { status, body } => {
                out.push(status);
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                out.extend_from_slice(body);
            }
            DehydratePayloadRef::ProtocolErr { status, message } => {
                out.push(status);
                out.extend_from_slice(&(message.len() as u32).to_le_bytes());
                out.extend_from_slice(message.as_bytes());
            }
            DehydratePayloadRef::StreamPending => {}
        }
    }
    debug_assert_eq!(out.len(), cap);
    debug_assert!(out.len() <= MAX_HYDRATE_BYTES);
    Ok(out)
}

pub(crate) fn decode_hydration_entries(bytes: &[u8]) -> Result<Vec<HydratedEntry>, DehydrateError> {
    if bytes.len() > MAX_HYDRATE_BYTES {
        return Err(DehydrateError::TooLarge(bytes.len()));
    }
    if bytes.len() < 9 {
        return Err(DehydrateError::Truncated);
    }
    if bytes[..4] != MAGIC {
        return Err(DehydrateError::InvalidMagic);
    }
    if bytes[4] != NVEQ_VERSION {
        return Err(DehydrateError::UnsupportedVersion(bytes[4]));
    }
    let entry_count = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
    if entry_count > MAX_HYDRATE_ENTRIES {
        return Err(DehydrateError::TooManyEntries(entry_count));
    }

    let mut pos = 9usize;
    let mut out = Vec::with_capacity(entry_count.min(64));

    for _ in 0..entry_count {
        let proc_id = ProcId::from_raw(read_u64(bytes, &mut pos)?);
        let age_ms = read_u32(bytes, &mut pos)?;
        let flags = read_u8(bytes, &mut pos)?;
        if (flags & HYDRATE_RESERVED_FLAGS_MASK) != 0 {
            return Err(DehydrateError::InvalidFlags(flags));
        }
        let subscribed_all = (flags & HYDRATE_FLAG_SUBSCRIBED_ALL) != 0;
        let entry_kind = (flags >> HYDRATE_KIND_SHIFT) & HYDRATE_KIND_MASK;

        let tag_count = read_u16(bytes, &mut pos)? as usize;
        if tag_count > MAX_HYDRATE_TAGS_PER_ENTRY {
            return Err(DehydrateError::TooManyTags(tag_count));
        }
        let mut tags = Vec::with_capacity(tag_count);
        for _ in 0..tag_count {
            tags.push(Tag64::from_raw(read_u64(bytes, &mut pos)?));
        }
        let args_len = read_u32(bytes, &mut pos)? as usize;
        let encoded_args = Bytes::copy_from_slice(read_slice(bytes, &mut pos, args_len)?);

        let payload = match entry_kind {
            ENTRY_KIND_OK => {
                let body_len = read_u32(bytes, &mut pos)? as usize;
                let raw_body = Bytes::copy_from_slice(read_slice(bytes, &mut pos, body_len)?);
                HydratedPayload::Ok(raw_body)
            }
            ENTRY_KIND_DOMAIN_ERR => {
                let status = read_u8(bytes, &mut pos)?;
                if rpc_status_from_u8(status).is_none() {
                    return Err(DehydrateError::InvalidStatus(status));
                }
                let body_len = read_u32(bytes, &mut pos)? as usize;
                let body = Bytes::copy_from_slice(read_slice(bytes, &mut pos, body_len)?);
                HydratedPayload::DomainErr { status, body }
            }
            ENTRY_KIND_PROTOCOL_ERR => {
                let status = read_u8(bytes, &mut pos)?;
                if rpc_status_from_u8(status).is_none() {
                    return Err(DehydrateError::InvalidStatus(status));
                }
                let msg_len = read_u32(bytes, &mut pos)? as usize;
                if msg_len > MAX_PROTOCOL_ERR_MSG_BYTES {
                    return Err(DehydrateError::TooLarge(msg_len));
                }
                let msg_bytes = read_slice(bytes, &mut pos, msg_len)?;
                let message = core::str::from_utf8(msg_bytes)
                    .map_err(|_| DehydrateError::InvalidUtf8)?
                    .to_owned();
                HydratedPayload::ProtocolErr { status, message }
            }
            ENTRY_KIND_STREAM_PENDING => HydratedPayload::StreamPending,
            _ => unreachable!("2-bit entry_kind covers 0..=3"),
        };

        out.push(HydratedEntry {
            proc_id,
            age_ms,
            subscribed_all,
            tags,
            encoded_args,
            payload,
        });
    }

    if pos != bytes.len() {
        return Err(DehydrateError::TrailingBytes(bytes.len() - pos));
    }
    Ok(out)
}

#[inline]
fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, DehydrateError> {
    Ok(read_slice(bytes, pos, 1)?[0])
}

#[inline]
fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, DehydrateError> {
    let slice = read_slice(bytes, pos, 2)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

#[inline]
fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, DehydrateError> {
    let slice = read_slice(bytes, pos, 4)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[inline]
fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, DehydrateError> {
    let slice = read_slice(bytes, pos, 8)?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

#[inline]
fn read_slice<'a>(bytes: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], DehydrateError> {
    let end = pos.checked_add(len).ok_or(DehydrateError::Truncated)?;
    let slice = bytes.get(*pos..end).ok_or(DehydrateError::Truncated)?;
    *pos = end;
    Ok(slice)
}
