use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::compact::{sync_parent_dir, write_fresh_wal_header_atomic};
use super::format::{
    compute_record_checksum, encode_record_prefix, scan_records_and_visit, WalDurability, WalError,
    WalHeader, WalRecord, WalRecordKind, WalRecordRef, WAL_HEADER_SIZE, WAL_RECORD_OVERHEAD,
};

/// Append-only binary WAL paired with an [`crate::MmapSnapshot`] base file.
pub struct SnapshotWal {
    pub(super) path: PathBuf,
    file: File,
    header: WalHeader,
    pub(super) durability: WalDurability,
    next_seq: u64,
    pub(super) record_count: u64,
    pub(super) file_len: u64,
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
        if let Some(parent) = path_buf.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
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
        if let Some(parent) = path_ref.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
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
                checksum_xxh64: rec.checksum_xxh64,
            });
            Ok(())
        })?;
        Ok(out)
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
