//! Append-only binary Write-Ahead Log (`.nvewal`) and crash-safe compaction for
//! Instant-Boot Frame-0 [`crate::MmapSnapshot`] containers.
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
//! - `checksum_xxh64: u64` (XXH64 over `seq || kind || payload_len || payload_bytes`)
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

mod compact;
mod format;
mod log;
#[cfg(test)]
mod tests;

pub use format::{
    WalDurability, WalError, WalHeader, WalRecord, WalRecordKind, WalRecordRef,
    NVE_WAL_FORMAT_VERSION, NVE_WAL_MAGIC, WAL_HEADER_SIZE, WAL_RECORD_CHECKSUM_SIZE,
    WAL_RECORD_HEADER_SIZE, WAL_RECORD_OVERHEAD,
};
pub use log::SnapshotWal;
