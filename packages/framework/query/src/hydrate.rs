//! Compact zero-`serde_json` binary dehydration/hydration codec for [`crate::QueryClient`]
//! (Slice 1c SSR hydration seam).
//!
//! Wire format (`NVEQ`, version `1`):
//! - `[0..4]` magic `"NVEQ"`
//! - `[4]`    version `1u8`
//! - `[5..9]` `entry_count: u32` (little-endian)
//! - For each entry:
//!   - `proc_id: u64`
//!   - `age_ms: u32`
//!   - `tag_count: u16`, followed by `tag_count` `u64` tag values
//!   - `args_len: u32`, followed by `encoded_args` bytes
//!   - `body_len: u32`, followed by `raw_response` bytes

use core::fmt;

use bytes::Bytes;
use nve_rpc::{ProcId, Tag64};

const MAGIC: [u8; 4] = *b"NVEQ";
const VERSION: u8 = 1;
const MAX_HYDRATE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HYDRATE_ENTRIES: usize = 4096;
const MAX_HYDRATE_TAGS_PER_ENTRY: usize = 128;

/// Failure returned when decoding a hydration snapshot buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DehydrateError {
    TooLarge(usize),
    InvalidMagic,
    UnsupportedVersion(u8),
    Truncated,
    TooManyEntries(usize),
    TooManyTags(usize),
    TrailingBytes(usize),
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
        }
    }
}

impl std::error::Error for DehydrateError {}

pub(crate) struct DehydrateEntryRef<'a> {
    pub proc_id: ProcId,
    pub age_ms: u32,
    pub tags: &'a [Tag64],
    pub encoded_args: &'a [u8],
    pub raw_body: &'a [u8],
}

pub(crate) struct HydratedEntry {
    pub proc_id: ProcId,
    pub age_ms: u32,
    pub tags: Vec<Tag64>,
    pub encoded_args: Bytes,
    pub raw_body: Bytes,
}

pub(crate) fn encode_hydration_entries(entries: &[DehydrateEntryRef<'_>]) -> Vec<u8> {
    let mut cap = 9usize;
    for e in entries {
        cap = cap
            .saturating_add(22)
            .saturating_add(e.tags.len().saturating_mul(8))
            .saturating_add(e.encoded_args.len())
            .saturating_add(e.raw_body.len());
    }
    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());

    for e in entries {
        out.extend_from_slice(&e.proc_id.raw().to_le_bytes());
        out.extend_from_slice(&e.age_ms.to_le_bytes());
        let tag_count = e.tags.len().min(MAX_HYDRATE_TAGS_PER_ENTRY);
        out.extend_from_slice(&(tag_count as u16).to_le_bytes());
        for tag in &e.tags[..tag_count] {
            out.extend_from_slice(&tag.raw().to_le_bytes());
        }
        let args_len = u32::try_from(e.encoded_args.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&args_len.to_le_bytes());
        out.extend_from_slice(e.encoded_args);
        let body_len = u32::try_from(e.raw_body.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&body_len.to_le_bytes());
        out.extend_from_slice(e.raw_body);
    }
    out
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
    if bytes[4] != VERSION {
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
        let body_len = read_u32(bytes, &mut pos)? as usize;
        let raw_body = Bytes::copy_from_slice(read_slice(bytes, &mut pos, body_len)?);

        out.push(HydratedEntry {
            proc_id,
            age_ms,
            tags,
            encoded_args,
            raw_body,
        });
    }

    if pos != bytes.len() {
        return Err(DehydrateError::TrailingBytes(bytes.len() - pos));
    }
    Ok(out)
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
