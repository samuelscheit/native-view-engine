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
//!   - `flags: u8` (`bit 0` = `subscribed_all`)
//!   - `tag_count: u16`, followed by `tag_count` `u64` tag values
//!   - `args_len: u32`, followed by `encoded_args` bytes
//!   - `body_len: u32`, followed by `raw_response` bytes

use core::fmt;

use bytes::Bytes;
use nve_hash::xxh64;
use nve_rpc::{MaybeSend, MaybeSync, ProcId, Tag64, Transport};

use crate::engine::{query_key_hash, run_side_effects, QueryClient, BODY_HASH_SEED};
use crate::slab::{
    FLAG_HAS_DATA, FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_FETCHING, FLAG_IS_INVALIDATED,
    FLAG_SUBSCRIBED_ALL,
};
use crate::state::WakeCallback;

const MAGIC: [u8; 4] = *b"NVEQ";
const VERSION: u8 = 1;
const MAX_HYDRATE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HYDRATE_ENTRIES: usize = 4096;
const MAX_HYDRATE_TAGS_PER_ENTRY: usize = 128;
const HYDRATE_FLAG_SUBSCRIBED_ALL: u8 = 1 << 0;

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
    pub subscribed_all: bool,
    pub tags: &'a [Tag64],
    pub encoded_args: &'a [u8],
    pub raw_body: &'a [u8],
}

pub(crate) struct HydratedEntry {
    pub proc_id: ProcId,
    pub age_ms: u32,
    pub subscribed_all: bool,
    pub tags: Vec<Tag64>,
    pub encoded_args: Bytes,
    pub raw_body: Bytes,
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
        if u32::try_from(e.encoded_args.len()).is_err() || u32::try_from(e.raw_body.len()).is_err()
        {
            return Err(DehydrateError::TooLarge(usize::MAX));
        }
        cap = cap
            .saturating_add(23)
            .saturating_add(e.tags.len().saturating_mul(8))
            .saturating_add(e.encoded_args.len())
            .saturating_add(e.raw_body.len());
        if cap > MAX_HYDRATE_BYTES {
            return Err(DehydrateError::TooLarge(cap));
        }
    }
    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    let count = u32::try_from(entries.len()).map_err(|_| DehydrateError::TooManyEntries(entries.len()))?;
    out.extend_from_slice(&count.to_le_bytes());

    for e in entries {
        out.extend_from_slice(&e.proc_id.raw().to_le_bytes());
        out.extend_from_slice(&e.age_ms.to_le_bytes());
        let flags = if e.subscribed_all {
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
        out.extend_from_slice(&(e.raw_body.len() as u32).to_le_bytes());
        out.extend_from_slice(e.raw_body);
    }
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
        let flags = read_slice(bytes, &mut pos, 1)?[0];
        let subscribed_all = (flags & HYDRATE_FLAG_SUBSCRIBED_ALL) != 0;
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
            subscribed_all,
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

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    /// Dehydrates all live entries with raw CBOR response bytes into a compact `NVEQ` binary blob
    /// for SSR -> client hydration (enforcing the same entry, tag, and byte caps as decode).
    pub fn dehydrate_bytes(&self) -> Result<Vec<u8>, DehydrateError> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut tag_bufs: Vec<Vec<Tag64>> = Vec::new();
            let mut slot_indices: Vec<usize> = Vec::new();
            for (idx, meta) in s.metas.iter().enumerate() {
                if meta.is_free() || !meta.has_data() {
                    continue;
                }
                if s.payloads[idx].raw_response.is_some() {
                    let mut tags = Vec::new();
                    s.tags.collect_tags_for_slot(meta.first_tag_edge, &mut tags);
                    tag_bufs.push(tags);
                    slot_indices.push(idx);
                }
            }
            let refs: Vec<DehydrateEntryRef<'_>> = slot_indices
                .iter()
                .zip(tag_bufs.iter())
                .map(|(&idx, tags)| {
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    let age_ms = u32::try_from(now_ms.saturating_sub(meta.updated_at_ms))
                        .unwrap_or(u32::MAX);
                    DehydrateEntryRef {
                        proc_id: meta.proc_id,
                        age_ms,
                        subscribed_all: (meta.flags & FLAG_SUBSCRIBED_ALL) != 0,
                        tags: tags.as_slice(),
                        encoded_args: &payload.encoded_args,
                        raw_body: payload.raw_response.as_deref().unwrap_or(&[]),
                    }
                })
                .collect();
            encode_hydration_entries(&refs)
        })
    }

    /// Seeds cache slots from a dehydrated `NVEQ` binary blob without decoding typed values yet
    /// (decoding happens lazily on the first typed [`Self::read`] call, tags and `subscribed_all`
    /// flags are indexed immediately, existing slots invalidate in-flight fetches and notify
    /// observers cleanly, and `max_entries` GC is enforced).
    pub fn hydrate_bytes(&self, bytes: &[u8]) -> Result<usize, DehydrateError> {
        let entries = decode_hydration_entries(bytes)?;
        let now_ms = self.inner.clock.now_ms();
        let count = entries.len();
        let (wake, waiters) = self.inner.state.with(|s| {
            let mut waiters = Vec::new();
            let mut wake_cb: Option<WakeCallback> = None;

            for entry in entries {
                let key_hash = query_key_hash(entry.proc_id, &entry.encoded_args);
                let updated_at = now_ms.saturating_sub(u64::from(entry.age_ms));
                let body_hash = xxh64(&entry.raw_body, BODY_HASH_SEED);

                let slot = if let Some(existing) =
                    s.lookup_slot(entry.proc_id, key_hash, &entry.encoded_args)
                {
                    let idx = existing.index();
                    if entry.subscribed_all && (s.metas[idx].flags & FLAG_SUBSCRIBED_ALL) == 0 {
                        s.metas[idx].flags |= FLAG_SUBSCRIBED_ALL;
                        s.subscribed_all_count += 1;
                    } else if !entry.subscribed_all && !entry.tags.is_empty() {
                        let mut first_edge = s.metas[idx].first_tag_edge;
                        s.tags.attach_tags(existing, &mut first_edge, &entry.tags);
                        s.metas[idx].first_tag_edge = first_edge;
                    }
                    existing
                } else {
                    let target_cap = self.inner.config.max_entries.saturating_sub(1);
                    s.gc_sweep(now_ms, 16, target_cap, &mut waiters);
                    // Pass `now_ms` as `last_inactive_ms` so SSR entries with `age_ms >= gc_time_ms`
                    // are not immediately evicted before the client mounts and reads them.
                    s.insert_slot(
                        entry.proc_id,
                        "",
                        key_hash,
                        entry.encoded_args,
                        self.inner.config.default_options,
                        &entry.tags,
                        entry.subscribed_all,
                        now_ms,
                    )
                    .slot
                };

                let idx = slot.index();
                let meta = &mut s.metas[idx];
                meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
                meta.updated_at_ms = updated_at;
                meta.invalidated_epoch = s.wildcard_epoch;
                meta.flags |= FLAG_HAS_DATA | FLAG_HYDRATED;
                meta.flags &= !(FLAG_HAS_ERROR | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING);

                let payload = &mut s.payloads[idx];
                waiters.append(&mut payload.in_flight_waiters);
                payload.raw_response = Some(entry.raw_body);
                payload.raw_body_hash = body_hash;
                payload.committed = None;
                payload.committed_error = None;
                payload.effective_optimistic = None;
                wake_cb = s.mark_dirty(slot);
            }
            (wake_cb, waiters)
        });
        run_side_effects(wake, waiters);
        Ok(count)
    }
}
