use std::fmt;
use std::io;

use crate::hash::Xxh64Hasher;
use crate::snapshot::SnapshotError;

/// 8-byte file magic identifying an NVE append-only snapshot WAL.
pub const NVE_WAL_MAGIC: [u8; 8] = *b"NVE_WAL1";

/// Current WAL binary container format version.
pub const NVE_WAL_FORMAT_VERSION: u32 = 1;

/// Fixed byte length of the `NVE_WAL1` file header.
pub const WAL_HEADER_SIZE: usize = 24;

/// Fixed byte length of each record prefix (`seq: u64` + `kind: u8` + `payload_len: u32`).
pub const WAL_RECORD_HEADER_SIZE: usize = 13;

/// Fixed byte length of each record trailing XXH64 checksum.
pub const WAL_RECORD_CHECKSUM_SIZE: usize = 8;

/// Total framing overhead per WAL record in bytes.
pub const WAL_RECORD_OVERHEAD: usize = WAL_RECORD_HEADER_SIZE + WAL_RECORD_CHECKSUM_SIZE;

/// Discriminant tag for sequential records inside an `NVE_WAL1` log.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WalRecordKind {
    /// Incremental binary application/signal state delta.
    StateDelta = 1,
    /// Incremental or replacement Frame-0 `CommandBuffer` delta.
    Frame0Delta = 2,
    /// Explicit checkpoint marker / folded state payload.
    Checkpoint = 3,
}

impl WalRecordKind {
    #[inline]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::StateDelta),
            2 => Some(Self::Frame0Delta),
            3 => Some(Self::Checkpoint),
            _ => None,
        }
    }
}

/// Durability mode controlling whether [`super::SnapshotWal::append_delta`] calls `sync_data()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalDurability {
    /// OS page-cache buffered append (fastest; pair with explicit [`super::SnapshotWal::sync_data`]).
    #[default]
    Buffered,
    /// Call `File::sync_data()` after every appended record.
    SyncData,
}

/// Decoded 24-byte header of an `NVE_WAL1` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalHeader {
    pub magic: [u8; 8],
    pub format_version: u32,
    pub base_schema_version: u32,
    pub base_snapshot_checksum: u64,
}

impl WalHeader {
    #[inline]
    pub const fn new(base_schema_version: u32, base_snapshot_checksum: u64) -> Self {
        Self {
            magic: NVE_WAL_MAGIC,
            format_version: NVE_WAL_FORMAT_VERSION,
            base_schema_version,
            base_snapshot_checksum,
        }
    }

    #[inline]
    pub fn encode(&self) -> [u8; WAL_HEADER_SIZE] {
        let mut out = [0u8; WAL_HEADER_SIZE];
        out[0..8].copy_from_slice(&self.magic);
        out[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        out[12..16].copy_from_slice(&self.base_schema_version.to_le_bytes());
        out[16..24].copy_from_slice(&self.base_snapshot_checksum.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WalError> {
        if bytes.len() < WAL_HEADER_SIZE {
            return Err(WalError::TruncatedHeader {
                expected: WAL_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().unwrap();
        if magic != NVE_WAL_MAGIC {
            return Err(WalError::InvalidMagic(magic));
        }
        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if format_version != NVE_WAL_FORMAT_VERSION {
            return Err(WalError::UnsupportedFormatVersion(format_version));
        }
        let base_schema_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let base_snapshot_checksum = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        Ok(Self {
            magic,
            format_version,
            base_schema_version,
            base_snapshot_checksum,
        })
    }
}

/// Zero-copy borrowed view of one validated WAL record passed to [`super::SnapshotWal::replay_into`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalRecordRef<'a> {
    pub seq: u64,
    pub kind: WalRecordKind,
    pub payload: &'a [u8],
    pub checksum_xxh64: u64,
}

/// Owned copy of one validated WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub seq: u64,
    pub kind: WalRecordKind,
    pub payload: Vec<u8>,
    pub checksum_xxh64: u64,
}

/// Errors returned while opening, validating, appending, replaying, or compacting a WAL.
#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    Snapshot(SnapshotError),
    TruncatedHeader {
        expected: usize,
        actual: usize,
    },
    InvalidMagic([u8; 8]),
    UnsupportedFormatVersion(u32),
    BaseSnapshotMismatch {
        expected_schema: u32,
        actual_schema: u32,
        expected_checksum: u64,
        actual_checksum: u64,
    },
    InvalidRecordKind {
        offset: u64,
        raw_kind: u8,
    },
    NonSequentialRecord {
        offset: u64,
        expected_seq: u64,
        actual_seq: u64,
    },
    CorruptRecordChecksum {
        offset: u64,
        seq: u64,
        expected: u64,
        actual: u64,
    },
    PayloadTooLarge(usize),
    ReplayCallback(String),
}

impl From<io::Error> for WalError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<SnapshotError> for WalError {
    fn from(err: SnapshotError) -> Self {
        Self::Snapshot(err)
    }
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "WAL I/O error: {e}"),
            Self::Snapshot(e) => write!(f, "WAL compaction snapshot error: {e}"),
            Self::TruncatedHeader { expected, actual } => {
                write!(f, "truncated WAL header: expected {expected} bytes, got {actual}")
            }
            Self::InvalidMagic(m) => write!(f, "invalid WAL magic: {m:?}"),
            Self::UnsupportedFormatVersion(v) => write!(f, "unsupported WAL format version: {v}"),
            Self::BaseSnapshotMismatch {
                expected_schema,
                actual_schema,
                expected_checksum,
                actual_checksum,
            } => write!(
                f,
                "WAL base snapshot mismatch: expected (schema={expected_schema}, checksum={expected_checksum:#018x}), got (schema={actual_schema}, checksum={actual_checksum:#018x})"
            ),
            Self::InvalidRecordKind { offset, raw_kind } => {
                write!(f, "invalid WAL record kind {raw_kind} at byte offset {offset}")
            }
            Self::NonSequentialRecord {
                offset,
                expected_seq,
                actual_seq,
            } => write!(
                f,
                "non-sequential WAL record at offset {offset}: expected seq {expected_seq}, got {actual_seq}"
            ),
            Self::CorruptRecordChecksum {
                offset,
                seq,
                expected,
                actual,
            } => write!(
                f,
                "corrupt WAL record seq={seq} at offset {offset}: expected checksum {expected:#018x}, computed {actual:#018x}"
            ),
            Self::PayloadTooLarge(len) => {
                write!(f, "WAL record payload length {len} exceeds u32::MAX")
            }
            Self::ReplayCallback(msg) => write!(f, "WAL replay callback failed: {msg}"),
        }
    }
}

impl std::error::Error for WalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Snapshot(e) => Some(e),
            _ => None,
        }
    }
}

#[inline]
pub(super) fn compute_record_checksum(
    prefix: &[u8; WAL_RECORD_HEADER_SIZE],
    payload: &[u8],
) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(prefix);
    h.update(payload);
    h.finish()
}

#[inline]
pub(super) fn encode_record_prefix(
    seq: u64,
    kind: WalRecordKind,
    payload_len: u32,
) -> [u8; WAL_RECORD_HEADER_SIZE] {
    let mut prefix = [0u8; WAL_RECORD_HEADER_SIZE];
    prefix[0..8].copy_from_slice(&seq.to_le_bytes());
    prefix[8] = kind as u8;
    prefix[9..13].copy_from_slice(&payload_len.to_le_bytes());
    prefix
}

/// Scans validated records from `data` starting at [`WAL_HEADER_SIZE`].
///
/// Returns `(valid_end_offset, record_count, next_seq)`.
/// When `allow_torn_tail` is true, any partial or bad-checksum trailing record at EOF stops
/// cleanly at the previous valid boundary instead of failing. Corrupt middle records (with
/// bytes remaining after `record_end`) always return `Err`.
pub(super) fn scan_records_and_visit<E>(
    data: &[u8],
    allow_torn_tail: bool,
    mut on_record: impl FnMut(WalRecordRef<'_>) -> Result<(), E>,
) -> Result<(usize, u64, u64), WalError>
where
    E: fmt::Display,
{
    let mut offset = WAL_HEADER_SIZE;
    let mut expected_seq = 1u64;
    let mut record_count = 0u64;

    while offset < data.len() {
        let remaining = data.len() - offset;
        if remaining < WAL_RECORD_HEADER_SIZE {
            if allow_torn_tail {
                break;
            }
            return Err(WalError::CorruptRecordChecksum {
                offset: offset as u64,
                seq: expected_seq,
                expected: 0,
                actual: 0,
            });
        }

        let prefix: [u8; WAL_RECORD_HEADER_SIZE] =
            data[offset..offset + WAL_RECORD_HEADER_SIZE].try_into().unwrap();
        let seq = u64::from_le_bytes(prefix[0..8].try_into().unwrap());
        let raw_kind = prefix[8];
        let payload_len = u32::from_le_bytes(prefix[9..13].try_into().unwrap()) as usize;

        let record_end = offset
            .checked_add(WAL_RECORD_HEADER_SIZE)
            .and_then(|v| v.checked_add(payload_len))
            .and_then(|v| v.checked_add(WAL_RECORD_CHECKSUM_SIZE));

        let Some(record_end) = record_end else {
            if allow_torn_tail {
                break;
            }
            return Err(WalError::PayloadTooLarge(payload_len));
        };

        if record_end > data.len() {
            if allow_torn_tail {
                // Partial/torn trailing record payload or checksum.
                break;
            }
            return Err(WalError::CorruptRecordChecksum {
                offset: offset as u64,
                seq,
                expected: 0,
                actual: 0,
            });
        }

        let is_trailing_record = record_end == data.len();
        let payload_start = offset + WAL_RECORD_HEADER_SIZE;
        let payload_end = payload_start + payload_len;
        let payload = &data[payload_start..payload_end];
        let stored_checksum =
            u64::from_le_bytes(data[payload_end..record_end].try_into().unwrap());
        let computed_checksum = compute_record_checksum(&prefix, payload);

        let kind = match WalRecordKind::from_u8(raw_kind) {
            Some(k) => k,
            None => {
                if allow_torn_tail && is_trailing_record {
                    break;
                }
                return Err(WalError::InvalidRecordKind {
                    offset: offset as u64,
                    raw_kind,
                });
            }
        };

        if seq != expected_seq {
            if allow_torn_tail && is_trailing_record {
                break;
            }
            return Err(WalError::NonSequentialRecord {
                offset: offset as u64,
                expected_seq,
                actual_seq: seq,
            });
        }

        if computed_checksum != stored_checksum {
            if allow_torn_tail && is_trailing_record {
                break;
            }
            return Err(WalError::CorruptRecordChecksum {
                offset: offset as u64,
                seq,
                expected: stored_checksum,
                actual: computed_checksum,
            });
        }

        on_record(WalRecordRef {
            seq,
            kind,
            payload,
            checksum_xxh64: stored_checksum,
        })
        .map_err(|e| WalError::ReplayCallback(e.to_string()))?;

        offset = record_end;
        expected_seq = expected_seq.saturating_add(1);
        record_count += 1;
    }

    Ok((offset, record_count, expected_seq))
}
