//! Append-only binary Write-Ahead Log (`.nvewal`) and crash-safe compaction for
//! Instant-Boot Frame-0 [`MmapSnapshot`] containers.
//!
//! # Binary Layout (`NVE_WAL1`, little-endian, zero JSON)
//!
//! Fixed 24-byte file header ([`WAL_HEADER_SIZE`]):
//! - `0..8`:   `magic: [u8; 8]` = `*b"NVE_WAL1"`
//! - `8..12`:  `format_version: u32` = `1`
//! - `12..16`: `base_schema_version: u32`
//! - `16..24`: `base_snapshot_checksum: u64`
//!
//! Followed by sequential framed records:
//! - `seq: u64` (strictly monotonic starting at `1`)
//! - `kind: u8` (`1 = StateDelta`, `2 = Frame0Delta`, `3 = Checkpoint`)
//! - `payload_len: u32`
//! - `payload_bytes: [u8; payload_len]`
//! - `checksum_fnv64: u64` (FNV-1a 64-bit over `seq || kind || payload_len || payload_bytes`)
//!
//! # Crash Recovery & Integrity Invariants
//!
//! - On [`SnapshotWal::open_or_create`], the log is scanned forward from offset `24`:
//!   - Any partial/torn **trailing** record (truncated header, partial payload/checksum, or torn
//!     final record checksum at EOF) is cleanly truncated back to the last valid record boundary
//!     so a mid-write crash never prevents cold boot.
//!   - Any corrupted **middle** record (invalid kind, non-sequential `seq`, or checksum mismatch
//!     with additional bytes following) is strictly rejected with a descriptive [`WalError`].
//! - [`SnapshotWal::compact_into_snapshot`] writes a fresh `NVE_SNAP` via sibling temp file +
//!   `sync_all()` + atomic `rename()`, then atomically rotates the WAL file to a clean 24-byte
//!   header bound to the new snapshot's checksum.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::snapshot::{
    fnv1a_64_update, write_snapshot_atomic, SnapshotError, SnapshotHeader,
};

/// 8-byte file magic identifying an NVE append-only snapshot WAL.
pub const NVE_WAL_MAGIC: [u8; 8] = *b"NVE_WAL1";

/// Current WAL binary container format version.
pub const NVE_WAL_FORMAT_VERSION: u32 = 1;

/// Fixed byte length of the `NVE_WAL1` file header.
pub const WAL_HEADER_SIZE: usize = 24;

/// Fixed byte length of each record prefix (`seq: u64` + `kind: u8` + `payload_len: u32`).
pub const WAL_RECORD_HEADER_SIZE: usize = 13;

/// Fixed byte length of each record trailing FNV-1a 64-bit checksum.
pub const WAL_RECORD_CHECKSUM_SIZE: usize = 8;

/// Total framing overhead per WAL record in bytes.
pub const WAL_RECORD_OVERHEAD: usize = WAL_RECORD_HEADER_SIZE + WAL_RECORD_CHECKSUM_SIZE;

const FNV1A_64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

static WAL_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(any(unix, windows))]
#[inline]
fn current_pid() -> u32 {
    std::process::id()
}

#[cfg(not(any(unix, windows)))]
#[inline]
fn current_pid() -> u32 {
    0
}

#[inline]
fn sync_parent_dir(dest: &Path) {
    #[cfg(unix)]
    if let Some(parent) = dest.parent() {
        let dir_path = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let _ = File::open(dir_path).and_then(|d| d.sync_all());
    }
    #[cfg(not(unix))]
    let _ = dest;
}

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

/// Durability mode controlling whether [`SnapshotWal::append_delta`] calls `sync_data()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WalDurability {
    /// OS page-cache buffered append (fastest; pair with explicit [`SnapshotWal::sync_data`]).
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

/// Zero-copy borrowed view of one validated WAL record passed to [`SnapshotWal::replay_into`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalRecordRef<'a> {
    pub seq: u64,
    pub kind: WalRecordKind,
    pub payload: &'a [u8],
    pub checksum_fnv64: u64,
}

/// Owned copy of one validated WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub seq: u64,
    pub kind: WalRecordKind,
    pub payload: Vec<u8>,
    pub checksum_fnv64: u64,
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
fn compute_record_checksum(prefix: &[u8; WAL_RECORD_HEADER_SIZE], payload: &[u8]) -> u64 {
    let h = fnv1a_64_update(FNV1A_64_OFFSET_BASIS, prefix);
    fnv1a_64_update(h, payload)
}

#[inline]
fn encode_record_prefix(seq: u64, kind: WalRecordKind, payload_len: u32) -> [u8; WAL_RECORD_HEADER_SIZE] {
    let mut prefix = [0u8; WAL_RECORD_HEADER_SIZE];
    prefix[0..8].copy_from_slice(&seq.to_le_bytes());
    prefix[8] = kind as u8;
    prefix[9..13].copy_from_slice(&payload_len.to_le_bytes());
    prefix
}

/// Atomically writes a fresh empty WAL containing only its 24-byte `NVE_WAL1` header.
fn write_fresh_wal_header_atomic(path: &Path, header: WalHeader) -> Result<(), WalError> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nve_snapshot.nvewal");
    let seq = WAL_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".{file_name}.tmp.{}.{seq}", current_pid());
    let tmp_path = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(&tmp_name),
        _ => PathBuf::from(&tmp_name),
    };

    let write_res = (|| -> io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        f.write_all(&header.encode())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp_path, path)?;
        sync_parent_dir(path);
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(WalError::Io(err));
    }
    Ok(())
}

/// Scans validated records from `data` starting at [`WAL_HEADER_SIZE`].
///
/// Returns `(valid_end_offset, record_count, next_seq)`.
/// When `allow_torn_tail` is true, any partial or bad-checksum trailing record at EOF stops
/// cleanly at the previous valid boundary instead of failing. Corrupt middle records (with
/// bytes remaining after `record_end`) always return `Err`.
fn scan_records_and_visit<E>(
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
            checksum_fnv64: stored_checksum,
        })
        .map_err(|e| WalError::ReplayCallback(e.to_string()))?;

        offset = record_end;
        expected_seq = expected_seq.saturating_add(1);
        record_count += 1;
    }

    Ok((offset, record_count, expected_seq))
}

/// Append-only binary WAL paired with an [`crate::MmapSnapshot`] base file.
pub struct SnapshotWal {
    path: PathBuf,
    file: File,
    header: WalHeader,
    durability: WalDurability,
    next_seq: u64,
    record_count: u64,
    file_len: u64,
    recovered_truncated_bytes: u64,
}

impl SnapshotWal {
    /// Opens an existing WAL or creates a new one bound to `(base_schema_version, base_checksum)`.
    ///
    /// Performs forward crash recovery on open: validates every record checksum and cleanly
    /// truncates any partial/torn trailing record while rejecting corrupted middle records.
    pub fn open_or_create(
        path: impl AsRef<Path>,
        base_schema_version: u32,
        base_checksum: u64,
    ) -> Result<Self, WalError> {
        Self::open_or_create_with_durability(
            path,
            base_schema_version,
            base_checksum,
            WalDurability::Buffered,
        )
    }

    /// Same as [`Self::open_or_create`] with an explicit [`WalDurability`] mode.
    pub fn open_or_create_with_durability(
        path: impl AsRef<Path>,
        base_schema_version: u32,
        base_checksum: u64,
        durability: WalDurability,
    ) -> Result<Self, WalError> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path_buf)?;

        let existing_len = file.metadata()?.len();
        if existing_len == 0 {
            let header = WalHeader::new(base_schema_version, base_checksum);
            file.write_all(&header.encode())?;
            file.sync_all()?;
            sync_parent_dir(&path_buf);
            return Ok(Self {
                path: path_buf,
                file,
                header,
                durability,
                next_seq: 1,
                record_count: 0,
                file_len: WAL_HEADER_SIZE as u64,
                recovered_truncated_bytes: 0,
            });
        }

        let data = fs::read(&path_buf)?;
        let header = WalHeader::decode(&data)?;
        if header.base_schema_version != base_schema_version
            || header.base_snapshot_checksum != base_checksum
        {
            return Err(WalError::BaseSnapshotMismatch {
                expected_schema: base_schema_version,
                actual_schema: header.base_schema_version,
                expected_checksum: base_checksum,
                actual_checksum: header.base_snapshot_checksum,
            });
        }

        let (valid_end, record_count, next_seq) =
            scan_records_and_visit(&data, true, |_| -> Result<(), std::convert::Infallible> {
                Ok(())
            })?;

        let mut recovered_truncated_bytes = 0u64;
        if valid_end < data.len() {
            recovered_truncated_bytes = (data.len() - valid_end) as u64;
            file.set_len(valid_end as u64)?;
            file.sync_data()?;
        }
        file.seek(SeekFrom::Start(valid_end as u64))?;

        Ok(Self {
            path: path_buf,
            file,
            header,
            durability,
            next_seq,
            record_count,
            file_len: valid_end as u64,
            recovered_truncated_bytes,
        })
    }

    /// Atomically creates/replaces the WAL at `path` with a fresh empty header bound to
    /// `(base_schema_version, base_checksum)`.
    pub fn create_fresh(
        path: impl AsRef<Path>,
        base_schema_version: u32,
        base_checksum: u64,
        durability: WalDurability,
    ) -> Result<Self, WalError> {
        let path_ref = path.as_ref();
        if let Some(parent) = path_ref.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let header = WalHeader::new(base_schema_version, base_checksum);
        write_fresh_wal_header_atomic(path_ref, header)?;
        Self::open_or_create_with_durability(path_ref, base_schema_version, base_checksum, durability)
    }

    #[inline]
    pub fn set_durability(&mut self, durability: WalDurability) {
        self.durability = durability;
    }

    #[inline]
    pub fn durability(&self) -> WalDurability {
        self.durability
    }

    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[inline]
    pub fn header(&self) -> WalHeader {
        self.header
    }

    #[inline]
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    #[inline]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    #[inline]
    pub fn file_len(&self) -> u64 {
        self.file_len
    }

    /// Number of torn/partial trailing bytes cleanly truncated during [`Self::open_or_create`].
    #[inline]
    pub fn recovered_truncated_bytes(&self) -> u64 {
        self.recovered_truncated_bytes
    }

    /// Appends one framed record in `O(1)` sequential I/O using the configured [`WalDurability`].
    ///
    /// Returns the monotonically assigned sequence number (`seq >= 1`).
    pub fn append_delta(&mut self, kind: WalRecordKind, payload: &[u8]) -> Result<u64, WalError> {
        let sync = matches!(self.durability, WalDurability::SyncData);
        self.append_delta_with_sync(kind, payload, sync)
    }

    /// Appends one framed record in `O(1)` sequential I/O with an explicit per-write `sync_data` flag.
    pub fn append_delta_with_sync(
        &mut self,
        kind: WalRecordKind,
        payload: &[u8],
        sync_data: bool,
    ) -> Result<u64, WalError> {
        let payload_len =
            u32::try_from(payload.len()).map_err(|_| WalError::PayloadTooLarge(payload.len()))?;
        let seq = self.next_seq;
        let prefix = encode_record_prefix(seq, kind, payload_len);
        let checksum = compute_record_checksum(&prefix, payload);

        // Single contiguous write for small/medium payloads avoids split syscalls.
        let total_len = WAL_RECORD_OVERHEAD + payload.len();
        let mut frame = Vec::with_capacity(total_len);
        frame.extend_from_slice(&prefix);
        frame.extend_from_slice(payload);
        frame.extend_from_slice(&checksum.to_le_bytes());

        self.file.write_all(&frame)?;
        if sync_data {
            self.file.sync_data()?;
        }

        self.next_seq = self.next_seq.saturating_add(1);
        self.record_count += 1;
        self.file_len += total_len as u64;
        Ok(seq)
    }

    /// Flushes OS data buffers for this WAL file to durable storage.
    pub fn sync_data(&self) -> Result<(), WalError> {
        self.file.sync_data()?;
        Ok(())
    }

    /// Replays every validated record in strict sequence order into `apply`, zero-copy per record.
    ///
    /// Returns the number of replayed records.
    pub fn replay_into<E>(
        &self,
        apply: impl FnMut(WalRecordRef<'_>) -> Result<(), E>,
    ) -> Result<usize, WalError>
    where
        E: fmt::Display,
    {
        let data = fs::read(&self.path)?;
        let header = WalHeader::decode(&data)?;
        if header != self.header {
            return Err(WalError::BaseSnapshotMismatch {
                expected_schema: self.header.base_schema_version,
                actual_schema: header.base_schema_version,
                expected_checksum: self.header.base_snapshot_checksum,
                actual_checksum: header.base_snapshot_checksum,
            });
        }
        let (_, count, _) = scan_records_and_visit(&data, false, apply)?;
        Ok(count as usize)
    }

    /// Reads and returns all validated records as owned [`WalRecord`] values in sequence order.
    pub fn read_all_records(&self) -> Result<Vec<WalRecord>, WalError> {
        let mut out = Vec::with_capacity(self.record_count as usize);
        self.replay_into(|rec| -> Result<(), std::convert::Infallible> {
            out.push(WalRecord {
                seq: rec.seq,
                kind: rec.kind,
                payload: rec.payload.to_vec(),
                checksum_fnv64: rec.checksum_fnv64,
            });
            Ok(())
        })?;
        Ok(out)
    }

    /// Returns `true` when the WAL has reached or exceeded either compaction threshold.
    ///
    /// Passing `0` for a threshold disables that specific limit.
    #[inline]
    pub fn should_compact(&self, max_bytes: u64, max_records: u64) -> bool {
        (max_bytes > 0 && self.file_len >= max_bytes)
            || (max_records > 0 && self.record_count >= max_records)
    }

    /// Crash-safely compacts folded Frame-0 + state + metadata into a fresh `NVE_SNAP` file
    /// (sibling temp file + `sync_all()` + atomic `rename()`) and atomically rotates `wal_path`
    /// to an empty WAL header bound to the new snapshot's checksum.
    ///
    /// Returns `(new_snapshot_header, rotated_wal)`.
    pub fn compact_into_snapshot(
        snapshot_path: impl AsRef<Path>,
        wal_path: impl AsRef<Path>,
        schema_version: u32,
        frame0_bytes: &[u8],
        folded_state_bytes: &[u8],
        metadata_bytes: &[u8],
    ) -> Result<(SnapshotHeader, SnapshotWal), WalError> {
        let snap_path = snapshot_path.as_ref();
        if let Some(parent) = snap_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let header = write_snapshot_atomic(
            snap_path,
            schema_version,
            0,
            frame0_bytes,
            folded_state_bytes,
            metadata_bytes,
        )?;
        let fresh_wal = Self::create_fresh(
            wal_path,
            schema_version,
            header.checksum_fnv64,
            WalDurability::Buffered,
        )?;
        Ok((header, fresh_wal))
    }

    /// Instance convenience wrapper around [`Self::compact_into_snapshot`] that preserves this
    /// WAL's path and durability setting and replaces `self` with the rotated empty WAL.
    pub fn compact(
        &mut self,
        snapshot_path: impl AsRef<Path>,
        schema_version: u32,
        frame0_bytes: &[u8],
        folded_state_bytes: &[u8],
        metadata_bytes: &[u8],
    ) -> Result<SnapshotHeader, WalError> {
        let durability = self.durability;
        let (header, mut rotated) = Self::compact_into_snapshot(
            snapshot_path,
            &self.path,
            schema_version,
            frame0_bytes,
            folded_state_bytes,
            metadata_bytes,
        )?;
        rotated.set_durability(durability);
        *self = rotated;
        Ok(header)
    }
}

impl fmt::Debug for SnapshotWal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotWal")
            .field("path", &self.path)
            .field("header", &self.header)
            .field("durability", &self.durability)
            .field("record_count", &self.record_count)
            .field("next_seq", &self.next_seq)
            .field("file_len", &self.file_len)
            .field("recovered_truncated_bytes", &self.recovered_truncated_bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{MmapSnapshot, RelocatableStateArena};
    use std::time::Instant;

    struct TempDirGuard(PathBuf);

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let seq = WAL_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "nve_wal_test_{label}_{}_{seq}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn append_and_replay_all_record_kinds_in_order() {
        let dir = TempDirGuard::new("append_replay");
        let wal_path = dir.path().join("app.nvewal");
        let base_checksum = 0x1234_5678_9abc_def0;

        {
            let mut wal = SnapshotWal::open_or_create_with_durability(
                &wal_path,
                42,
                base_checksum,
                WalDurability::SyncData,
            )
            .unwrap();
            assert_eq!(wal.record_count(), 0);
            assert_eq!(wal.next_seq(), 1);
            assert_eq!(wal.file_len(), WAL_HEADER_SIZE as u64);

            assert_eq!(
                wal.append_delta(WalRecordKind::StateDelta, b"delta-one")
                    .unwrap(),
                1
            );
            assert_eq!(
                wal.append_delta(WalRecordKind::Frame0Delta, b"frame0-patch")
                    .unwrap(),
                2
            );
            assert_eq!(
                wal.append_delta(WalRecordKind::Checkpoint, b"").unwrap(),
                3
            );
            assert_eq!(wal.record_count(), 3);
            assert_eq!(wal.next_seq(), 4);
        }

        // Reopen and verify exact replay order + payloads.
        let reopened = SnapshotWal::open_or_create(&wal_path, 42, base_checksum).unwrap();
        assert_eq!(reopened.record_count(), 3);
        assert_eq!(reopened.recovered_truncated_bytes(), 0);

        let mut seen = Vec::new();
        let count = reopened
            .replay_into(|rec| -> Result<(), std::convert::Infallible> {
                seen.push((rec.seq, rec.kind, rec.payload.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            seen,
            vec![
                (1, WalRecordKind::StateDelta, b"delta-one".to_vec()),
                (2, WalRecordKind::Frame0Delta, b"frame0-patch".to_vec()),
                (3, WalRecordKind::Checkpoint, Vec::new()),
            ]
        );

        // Mismatched base schema or checksum must be rejected.
        assert!(matches!(
            SnapshotWal::open_or_create(&wal_path, 99, base_checksum),
            Err(WalError::BaseSnapshotMismatch { .. })
        ));
        assert!(matches!(
            SnapshotWal::open_or_create(&wal_path, 42, base_checksum ^ 1),
            Err(WalError::BaseSnapshotMismatch { .. })
        ));
    }

    #[test]
    fn torn_tail_partial_writes_recover_cleanly_at_every_cut_point() {
        let dir = TempDirGuard::new("torn_tail");
        let base_checksum = 0xfeed_face_cafe_babe;

        // Build a reference 4-record WAL.
        let ref_path = dir.path().join("ref.nvewal");
        let mut wal = SnapshotWal::open_or_create(&ref_path, 3, base_checksum).unwrap();
        wal.append_delta(WalRecordKind::StateDelta, b"rec-1-alpha")
            .unwrap();
        wal.append_delta(WalRecordKind::StateDelta, b"rec-2-beta-payload")
            .unwrap();
        wal.append_delta(WalRecordKind::Frame0Delta, b"rec-3-gamma")
            .unwrap();
        let boundary_after_3 = wal.file_len() as usize;
        wal.append_delta(WalRecordKind::StateDelta, b"rec-4-torn-tail-record")
            .unwrap();
        let full_bytes = fs::read(&ref_path).unwrap();
        drop(wal);

        // Test every possible byte-level truncation inside record 4 (header, payload, checksum).
        for cut in (boundary_after_3 + 1)..full_bytes.len() {
            let p = dir.path().join(format!("cut_{cut}.nvewal"));
            fs::write(&p, &full_bytes[..cut]).unwrap();

            let mut recovered = SnapshotWal::open_or_create(&p, 3, base_checksum).unwrap();
            assert_eq!(recovered.record_count(), 3, "cut={cut}");
            assert_eq!(recovered.next_seq(), 4, "cut={cut}");
            assert_eq!(recovered.file_len(), boundary_after_3 as u64, "cut={cut}");
            assert_eq!(
                recovered.recovered_truncated_bytes(),
                (cut - boundary_after_3) as u64,
                "cut={cut}"
            );

            // Subsequent appends continue cleanly at seq=4.
            assert_eq!(
                recovered
                    .append_delta(WalRecordKind::StateDelta, b"rec-4-healed")
                    .unwrap(),
                4
            );
            let recs = recovered.read_all_records().unwrap();
            assert_eq!(recs.len(), 4);
            assert_eq!(recs[3].seq, 4);
            assert_eq!(recs[3].payload, b"rec-4-healed");
        }

        // Also test a torn final record where OS extended file length with corrupt tail checksum.
        let bad_tail_path = dir.path().join("bad_tail_checksum.nvewal");
        let mut corrupted_tail = full_bytes.clone();
        let last_idx = corrupted_tail.len() - 1;
        corrupted_tail[last_idx] ^= 0x5A;
        fs::write(&bad_tail_path, &corrupted_tail).unwrap();

        let recovered = SnapshotWal::open_or_create(&bad_tail_path, 3, base_checksum).unwrap();
        assert_eq!(recovered.record_count(), 3);
        assert_eq!(
            recovered.recovered_truncated_bytes(),
            (full_bytes.len() - boundary_after_3) as u64
        );
    }

    #[test]
    fn corrupted_middle_record_is_rejected() {
        let dir = TempDirGuard::new("corrupt_middle");
        let wal_path = dir.path().join("mid.nvewal");
        let base_checksum = 0x0bad_c0de_1122_3344;

        let mut wal = SnapshotWal::open_or_create(&wal_path, 1, base_checksum).unwrap();
        wal.append_delta(WalRecordKind::StateDelta, b"first-record")
            .unwrap();
        let rec2_offset = wal.file_len() as usize;
        wal.append_delta(WalRecordKind::StateDelta, b"second-record-payload")
            .unwrap();
        wal.append_delta(WalRecordKind::StateDelta, b"third-record")
            .unwrap();
        drop(wal);

        let original = fs::read(&wal_path).unwrap();

        // 1. Flip a byte inside record 2's payload -> CorruptRecordChecksum.
        let payload_corrupt = dir.path().join("mid_payload.nvewal");
        let mut bytes = original.clone();
        bytes[rec2_offset + WAL_RECORD_HEADER_SIZE + 2] ^= 0xFF;
        fs::write(&payload_corrupt, &bytes).unwrap();
        assert!(matches!(
            SnapshotWal::open_or_create(&payload_corrupt, 1, base_checksum),
            Err(WalError::CorruptRecordChecksum { seq: 2, .. })
        ));

        // 2. Corrupt record 2's kind byte -> InvalidRecordKind.
        let kind_corrupt = dir.path().join("mid_kind.nvewal");
        let mut bytes = original.clone();
        bytes[rec2_offset + 8] = 99;
        fs::write(&kind_corrupt, &bytes).unwrap();
        assert!(matches!(
            SnapshotWal::open_or_create(&kind_corrupt, 1, base_checksum),
            Err(WalError::InvalidRecordKind { raw_kind: 99, .. })
        ));

        // 3. Corrupt record 2's sequence number -> NonSequentialRecord.
        let seq_corrupt = dir.path().join("mid_seq.nvewal");
        let mut bytes = original.clone();
        bytes[rec2_offset..rec2_offset + 8].copy_from_slice(&77u64.to_le_bytes());
        fs::write(&seq_corrupt, &bytes).unwrap();
        assert!(matches!(
            SnapshotWal::open_or_create(&seq_corrupt, 1, base_checksum),
            Err(WalError::NonSequentialRecord {
                expected_seq: 2,
                actual_seq: 77,
                ..
            })
        ));
    }

    #[test]
    fn threshold_compaction_roundtrips_mmap_snapshot_and_rotates_wal() {
        let dir = TempDirGuard::new("compact");
        let snap_path = dir.path().join("app.nvesnap");
        let wal_path = dir.path().join("app.nvewal");

        let mut arena0 = RelocatableStateArena::new();
        arena0.insert_i64("count", 10);
        let initial_snap = write_snapshot_atomic(
            &snap_path,
            5,
            0,
            b"FRAME0_V1",
            &arena0.finish(),
            b"meta-v1",
        )
        .unwrap();

        let mut wal =
            SnapshotWal::open_or_create(&wal_path, 5, initial_snap.checksum_fnv64).unwrap();
        assert!(!wal.should_compact(512, 4));

        for i in 1..=4i64 {
            wal.append_delta(WalRecordKind::StateDelta, &i.to_le_bytes())
                .unwrap();
        }
        assert!(wal.should_compact(0, 4));
        assert!(wal.should_compact(wal.file_len(), 0));

        // Fold deltas + compact atomically into a fresh MmapSnapshot.
        let mut folded_count = 10i64;
        wal.replay_into(|rec| -> Result<(), std::convert::Infallible> {
            if rec.kind == WalRecordKind::StateDelta {
                folded_count += i64::from_le_bytes(rec.payload.try_into().unwrap());
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(folded_count, 20);

        let mut arena1 = RelocatableStateArena::new();
        arena1.insert_i64("count", folded_count);
        arena1.insert_str("status", "compacted");

        let (new_header, mut rotated_wal) = SnapshotWal::compact_into_snapshot(
            &snap_path,
            &wal_path,
            5,
            b"FRAME0_V2_COMPACTED",
            &arena1.finish(),
            b"meta-v2",
        )
        .unwrap();

        assert_ne!(new_header.checksum_fnv64, initial_snap.checksum_fnv64);
        assert_eq!(rotated_wal.record_count(), 0);
        assert_eq!(rotated_wal.next_seq(), 1);
        assert_eq!(rotated_wal.file_len(), WAL_HEADER_SIZE as u64);
        assert_eq!(
            rotated_wal.header().base_snapshot_checksum,
            new_header.checksum_fnv64
        );
        assert!(!rotated_wal.should_compact(512, 4));

        // MmapSnapshot::open must read the compacted snapshot with zero errors.
        let mapped = MmapSnapshot::open_with_schema(&snap_path, 5).unwrap();
        assert_eq!(mapped.checksum_fnv64(), new_header.checksum_fnv64);
        assert_eq!(mapped.frame0_command_buffer(), b"FRAME0_V2_COMPACTED");
        assert_eq!(mapped.meta_bytes(), b"meta-v2");
        let table = mapped.state_table().unwrap();
        assert_eq!(table.get_i64("count"), Some(20));
        assert_eq!(table.get_str("status"), Some("compacted"));

        // Further appends work seamlessly on the rotated WAL.
        assert_eq!(
            rotated_wal
                .append_delta(WalRecordKind::StateDelta, b"post-compact")
                .unwrap(),
            1
        );
        assert_eq!(rotated_wal.record_count(), 1);
    }

    #[test]
    fn thousand_delta_append_replay_and_compact_performance() {
        let dir = TempDirGuard::new("perf_1000");
        let snap_path = dir.path().join("perf.nvesnap");
        let wal_path = dir.path().join("perf.nvewal");

        let initial_header =
            write_snapshot_atomic(&snap_path, 1, 0, b"FRAME0_INIT", b"state0", b"").unwrap();
        let mut wal =
            SnapshotWal::open_or_create(&wal_path, 1, initial_header.checksum_fnv64).unwrap();

        let t_append = Instant::now();
        for i in 0..1_000u64 {
            let mut payload = [0u8; 24];
            payload[0..8].copy_from_slice(&i.to_le_bytes());
            payload[8..16].copy_from_slice(&(i * 3).to_le_bytes());
            payload[16..24].copy_from_slice(&(i ^ 0xA5A5_5A5A).to_le_bytes());
            let seq = wal
                .append_delta(WalRecordKind::StateDelta, &payload)
                .unwrap();
            assert_eq!(seq, i + 1);
        }
        wal.sync_data().unwrap();
        let append_elapsed = t_append.elapsed();

        assert_eq!(wal.record_count(), 1_000);
        assert!(wal.should_compact(0, 1_000));

        let t_replay = Instant::now();
        let mut sum = 0u64;
        let replayed = wal
            .replay_into(|rec| -> Result<(), std::convert::Infallible> {
                let v = u64::from_le_bytes(rec.payload[0..8].try_into().unwrap());
                sum += v;
                Ok(())
            })
            .unwrap();
        let replay_elapsed = t_replay.elapsed();
        assert_eq!(replayed, 1_000);
        assert_eq!(sum, 999 * 1_000 / 2);

        let t_compact = Instant::now();
        let new_header = wal
            .compact(&snap_path, 1, b"FRAME0_1000", &sum.to_le_bytes(), b"")
            .unwrap();
        let compact_elapsed = t_compact.elapsed();

        assert_eq!(wal.record_count(), 0);
        assert_eq!(wal.file_len(), WAL_HEADER_SIZE as u64);
        assert_eq!(wal.header().base_snapshot_checksum, new_header.checksum_fnv64);

        let mapped = MmapSnapshot::open_with_schema(&snap_path, 1).unwrap();
        assert_eq!(mapped.state_bytes(), &sum.to_le_bytes());

        // Generous upper bounds that catch catastrophic regressions even on slow CI runners.
        assert!(
            append_elapsed.as_secs() < 5,
            "1,000 WAL appends took {append_elapsed:?}"
        );
        assert!(
            replay_elapsed.as_secs() < 5,
            "1,000 WAL replays took {replay_elapsed:?}"
        );
        assert!(
            compact_elapsed.as_secs() < 5,
            "WAL compaction took {compact_elapsed:?}"
        );
    }
}
