#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::snapshot::{write_snapshot_atomic, SnapshotHeader};

use super::format::{WalDurability, WalError, WalHeader};
use super::log::SnapshotWal;

pub(super) static WAL_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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
pub(super) fn sync_parent_dir(dest: &Path) {
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

/// Atomically writes a fresh empty WAL containing only its 24-byte `NVE_WAL1` header.
pub(super) fn write_fresh_wal_header_atomic(
    path: &Path,
    header: WalHeader,
) -> Result<(), WalError> {
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
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            f.write_all(&header.encode())?;
            f.sync_all()?;
        }
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

impl SnapshotWal {
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
        if let Some(parent) = snap_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
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
