//! `NVK2` append-only WAL wire framing, streaming compaction writer, and crash-safe replay
//! distinguishing genuine torn tails at EOF from mid-log corruption.

use std::collections::BTreeMap;
use std::io::Write;

use nve_codec::{ByteReader, ByteWriter};
use nve_hash::xxh64;

use crate::{validate_key, validate_value, StorageError, MAX_BATCH_OPS, MAX_KEY_LEN, MAX_VALUE_LEN};

/// File magic for the crash-safe append-only KV log.
pub const NVK2_MAGIC: [u8; 4] = *b"NVK2";
/// Current binary log wire version.
pub const NVK2_VERSION: u16 = 1;
/// Atomic record commit marker placed after the per-record XXH64 checksum.
pub const NVK2_COMMIT_MARKER: [u8; 4] = *b"NKCM";

pub(crate) const HEADER_LEN: usize = 16;
pub(crate) const RECORD_HEADER_LEN: usize = 16;
pub(crate) const RECORD_TRAILER_LEN: usize = 12;
const MIN_RECORD_LEN: usize = RECORD_HEADER_LEN + RECORD_TRAILER_LEN;

pub(crate) const KIND_PUT: u8 = 1;
pub(crate) const KIND_DELETE: u8 = 2;
pub(crate) const KIND_BATCH: u8 = 3;

pub(crate) fn encode_file_header() -> [u8; HEADER_LEN] {
    let mut hdr = [0u8; HEADER_LEN];
    hdr[0..4].copy_from_slice(&NVK2_MAGIC);
    hdr[4..6].copy_from_slice(&NVK2_VERSION.to_le_bytes());
    let checksum = xxh64(&hdr[0..8], 0);
    hdr[8..16].copy_from_slice(&checksum.to_le_bytes());
    hdr
}

pub(crate) fn encode_record_frame_into(
    out: &mut Vec<u8>,
    kind: u8,
    seq_no: u64,
    payload: &[u8],
) -> Result<(), StorageError> {
    out.clear();
    let needed = RECORD_HEADER_LEN + payload.len() + RECORD_TRAILER_LEN;
    out.reserve(needed);
    ByteWriter::with_buf(out, |w| {
        w.write_u8(kind);
        w.write_raw(&[0u8; 3]);
        w.write_u64(seq_no);
        w.write_bytes(payload)
            .map_err(|_| StorageError::InvalidArg("NVK2 record payload exceeds u32::MAX".into()))?;
        let checksum = xxh64(w.as_slice(), 0);
        w.write_u64(checksum);
        w.write_raw(&NVK2_COMMIT_MARKER);
        Ok(())
    })
}

/// Streams a freshly compacted `NVK2` image directly into `writer` using a single reused scratch
/// buffer (zero full-database `Vec<u8>` amplification), returning `(next_seq, bytes_written)`.
pub(crate) fn stream_compacted_entries<W: Write>(
    entries: &BTreeMap<Vec<u8>, Vec<u8>>,
    writer: &mut W,
) -> Result<(u64, u64), StorageError> {
    let header = encode_file_header();
    writer.write_all(&header)?;
    let mut bytes_written = header.len() as u64;
    let mut seq = 1u64;
    let mut payload = ByteWriter::with_capacity(256);
    let mut frame = Vec::with_capacity(256 + MIN_RECORD_LEN);
    for (k, v) in entries {
        payload.clear();
        payload.write_bytes(k)?;
        payload.write_bytes(v)?;
        encode_record_frame_into(&mut frame, KIND_PUT, seq, payload.as_slice())?;
        writer.write_all(&frame)?;
        bytes_written = bytes_written.saturating_add(frame.len() as u64);
        seq = seq.saturating_add(1);
    }
    Ok((seq, bytes_written))
}

pub(crate) struct ReplayResult {
    pub(crate) entries: BTreeMap<Vec<u8>, Vec<u8>>,
    pub(crate) last_seq: u64,
    pub(crate) valid_len: usize,
    pub(crate) recovered_torn_tail: bool,
}

/// Scans `bytes[search_start..]` for any genuinely framed and XXH64-authenticated `NVK2` record
/// (valid `kind`, zero reserved bytes, non-zero `seq_no`, in-bounds `payload_len`, `NKCM` trailer,
/// and matching XXH64 checksum).
///
/// Unlike a raw `b"NKCM"` substring search, arbitrary user binary payloads containing `b"NKCM"`
/// inside a torn final record will NOT trigger a false positive. Bounded by a cumulative hash byte
/// budget so crafted inputs cannot cause quadratic work.
fn has_valid_committed_frame_after(bytes: &[u8], search_start: usize) -> bool {
    if bytes.len().saturating_sub(search_start) < MIN_RECORD_LEN {
        return false;
    }
    let last_candidate = bytes.len() - MIN_RECORD_LEN;
    let mut hash_budget = bytes.len().saturating_mul(2).max(65_536);

    for pos in search_start..=last_candidate {
        let kind = bytes[pos];
        if !matches!(kind, KIND_PUT | KIND_DELETE | KIND_BATCH) {
            continue;
        }
        if bytes[pos + 1..pos + 4] != [0u8; 3] {
            continue;
        }
        let mut r = ByteReader::new(&bytes[pos + 4..]);
        let Ok(seq_no) = r.read_u64() else {
            continue;
        };
        if seq_no == 0 {
            continue;
        }
        let Ok(payload_len) = r.read_u32().map(|v| v as usize) else {
            continue;
        };
        if payload_len
            .checked_add(RECORD_TRAILER_LEN)
            .is_none_or(|n| n > r.remaining())
        {
            continue;
        }
        let payload_end = pos + RECORD_HEADER_LEN + payload_len;
        let frame_end = payload_end + RECORD_TRAILER_LEN;
        if bytes[frame_end - 4..frame_end] != NVK2_COMMIT_MARKER {
            continue;
        }
        let hashed_len = payload_end - pos;
        if hashed_len > hash_budget {
            break;
        }
        hash_budget -= hashed_len;
        let mut trailer = ByteReader::new(&bytes[payload_end..frame_end]);
        let Ok(stored_crc) = trailer.read_u64() else {
            continue;
        };
        if xxh64(&bytes[pos..payload_end], 0) == stored_crc {
            return true;
        }
    }
    false
}

#[inline]
fn map_replay_corrupt(err: StorageError) -> StorageError {
    match err {
        StorageError::Corrupt(_) => err,
        other => StorageError::Corrupt(other.to_string()),
    }
}

pub(crate) fn replay_nvk2_buffer(bytes: &[u8]) -> Result<ReplayResult, StorageError> {
    if bytes.len() < HEADER_LEN {
        return Err(StorageError::Corrupt("NVK2 header truncated".into()));
    }
    let mut r = ByteReader::new(bytes);
    r.expect_magic(&NVK2_MAGIC)
        .map_err(|_| StorageError::Corrupt("invalid NVK2 file magic".into()))?;
    let version = r.read_u16()?;
    if version != NVK2_VERSION {
        return Err(StorageError::Corrupt(format!(
            "unsupported NVK2 version {version}"
        )));
    }
    let reserved = r.read_u16()?;
    if reserved != 0 {
        return Err(StorageError::Corrupt(format!(
            "non-zero NVK2 file header reserved field {reserved:#x}"
        )));
    }
    let stored_hdr_crc = r.read_u64()?;
    if xxh64(&bytes[0..8], 0) != stored_hdr_crc {
        return Err(StorageError::Corrupt(
            "corrupt NVK2 file header checksum".into(),
        ));
    }

    let mut entries = BTreeMap::new();
    let mut valid_len = HEADER_LEN;
    let mut last_seq = 0u64;
    let mut recovered_torn_tail = false;

    while r.remaining() > 0 {
        let frame_start = r.pos();
        // Fewer than MIN_RECORD_LEN (28) bytes remaining at EOF can never hold a committed record,
        // so this is unconditionally a torn tail even if the fragment contains `b"NKCM"`.
        if r.remaining() < MIN_RECORD_LEN {
            recovered_torn_tail = true;
            break;
        }
        let kind = r.read_u8()?;
        let rec_reserved = r.read_array::<3>()?;
        let seq_no = r.read_u64()?;
        let payload_len = r.read_u32()? as usize;

        if payload_len
            .checked_add(RECORD_TRAILER_LEN)
            .is_none_or(|n| n > r.remaining())
        {
            if has_valid_committed_frame_after(bytes, frame_start + 1) {
                return Err(StorageError::Corrupt(
                    "mid-log NVK2 record length corruption before committed tail".into(),
                ));
            }
            recovered_torn_tail = true;
            break;
        }

        let payload = r.read_exact(payload_len)?;
        let payload_end = r.pos();
        let stored_crc = r.read_u64()?;
        let marker = r.read_array::<4>()?;
        let has_trailing = r.remaining() > 0;
        let computed_crc = xxh64(&bytes[frame_start..payload_end], 0);

        if marker != NVK2_COMMIT_MARKER || stored_crc != computed_crc {
            if has_trailing
                && (marker == NVK2_COMMIT_MARKER
                    || has_valid_committed_frame_after(bytes, frame_start + 1))
            {
                return Err(StorageError::Corrupt(format!(
                    "mid-log NVK2 record corruption at byte offset {frame_start}"
                )));
            }
            recovered_torn_tail = true;
            break;
        }

        // Checksum and commit marker both verified: any format/reserved/payload violation here
        // represents genuine data corruption (not an incomplete crash write).
        if rec_reserved != [0u8; 3] {
            return Err(StorageError::Corrupt(format!(
                "non-zero NVK2 record reserved bytes at offset {frame_start}"
            )));
        }
        apply_record_payload(kind, payload, &mut entries)?;

        last_seq = last_seq.max(seq_no);
        valid_len = r.pos();
    }

    Ok(ReplayResult {
        entries,
        last_seq,
        valid_len,
        recovered_torn_tail,
    })
}

enum BatchOpRef<'a> {
    Put { key: &'a [u8], value: &'a [u8] },
    Delete { key: &'a [u8] },
}

fn apply_record_payload(
    kind: u8,
    payload: &[u8],
    entries: &mut BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<(), StorageError> {
    let mut r = ByteReader::new(payload);
    match kind {
        KIND_PUT => {
            let key = r.read_bytes(MAX_KEY_LEN)?;
            let value = r.read_bytes(MAX_VALUE_LEN)?;
            r.finish()?;
            validate_key(key).map_err(map_replay_corrupt)?;
            validate_value(value).map_err(map_replay_corrupt)?;
            entries.insert(key.to_vec(), value.to_vec());
        }
        KIND_DELETE => {
            let key = r.read_bytes(MAX_KEY_LEN)?;
            r.finish()?;
            validate_key(key).map_err(map_replay_corrupt)?;
            entries.remove(key);
        }
        KIND_BATCH => {
            let count = r.read_len(MAX_BATCH_OPS, 5)?;
            let mut decoded = Vec::with_capacity(count);
            for _ in 0..count {
                match r.read_u8()? {
                    KIND_PUT => {
                        let key = r.read_bytes(MAX_KEY_LEN)?;
                        let value = r.read_bytes(MAX_VALUE_LEN)?;
                        validate_key(key).map_err(map_replay_corrupt)?;
                        validate_value(value).map_err(map_replay_corrupt)?;
                        decoded.push(BatchOpRef::Put { key, value });
                    }
                    KIND_DELETE => {
                        let key = r.read_bytes(MAX_KEY_LEN)?;
                        validate_key(key).map_err(map_replay_corrupt)?;
                        decoded.push(BatchOpRef::Delete { key });
                    }
                    other => {
                        return Err(StorageError::Corrupt(format!(
                            "invalid batch op kind {other}"
                        )));
                    }
                }
            }
            r.finish()?;
            for op in decoded {
                match op {
                    BatchOpRef::Put { key, value } => {
                        entries.insert(key.to_vec(), value.to_vec());
                    }
                    BatchOpRef::Delete { key } => {
                        entries.remove(key);
                    }
                }
            }
        }
        other => {
            return Err(StorageError::Corrupt(format!(
                "unknown NVK2 record kind {other}"
            )));
        }
    }
    Ok(())
}
