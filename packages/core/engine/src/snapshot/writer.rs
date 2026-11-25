use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::format::{
    align_up_8, compute_snapshot_checksum, pad_to_8, SnapshotError, SnapshotHeader,
    CHECKSUM_FIELD_END, CHECKSUM_FIELD_OFFSET, NVE_SNAP_FORMAT_VERSION, NVE_SNAP_MAGIC,
    SNAPSHOT_HEADER_SIZE,
};

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

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

/// Encodes a complete, 8-byte aligned `NVE_SNAP` binary container in memory.
///
/// Suitable for WASM hosts, unit tests, and Tier-2 hot-swap transfers.
pub fn encode_snapshot(
    app_schema_version: u32,
    flags: u32,
    frame0_command_buffer: &[u8],
    state_bytes: &[u8],
    meta_bytes: &[u8],
) -> Vec<u8> {
    let (bytes, _) = encode_snapshot_with_header(
        app_schema_version,
        flags,
        frame0_command_buffer,
        state_bytes,
        meta_bytes,
    );
    bytes
}

fn encode_snapshot_with_header(
    app_schema_version: u32,
    flags: u32,
    frame0_command_buffer: &[u8],
    state_bytes: &[u8],
    meta_bytes: &[u8],
) -> (Vec<u8>, SnapshotHeader) {
    let estimated_size = SNAPSHOT_HEADER_SIZE
        + align_up_8(frame0_command_buffer.len())
        + align_up_8(state_bytes.len())
        + align_up_8(meta_bytes.len());
    let mut buf = Vec::with_capacity(estimated_size);

    // Reserve header bytes (zeroed initially).
    buf.resize(SNAPSHOT_HEADER_SIZE, 0);

    let frame0_offset = buf.len() as u64;
    let frame0_len = frame0_command_buffer.len() as u64;
    buf.extend_from_slice(frame0_command_buffer);
    pad_to_8(&mut buf);

    let state_offset = buf.len() as u64;
    let state_len = state_bytes.len() as u64;
    buf.extend_from_slice(state_bytes);
    pad_to_8(&mut buf);

    let meta_offset = buf.len() as u64;
    let meta_len = meta_bytes.len() as u64;
    buf.extend_from_slice(meta_bytes);
    pad_to_8(&mut buf);

    // Populate header fields (except checksum).
    buf[0..8].copy_from_slice(&NVE_SNAP_MAGIC);
    buf[8..12].copy_from_slice(&NVE_SNAP_FORMAT_VERSION.to_le_bytes());
    buf[12..16].copy_from_slice(&app_schema_version.to_le_bytes());
    buf[16..20].copy_from_slice(&flags.to_le_bytes());
    buf[20..24].copy_from_slice(&0u32.to_le_bytes());
    buf[32..40].copy_from_slice(&frame0_offset.to_le_bytes());
    buf[40..48].copy_from_slice(&frame0_len.to_le_bytes());
    buf[48..56].copy_from_slice(&state_offset.to_le_bytes());
    buf[56..64].copy_from_slice(&state_len.to_le_bytes());
    buf[64..72].copy_from_slice(&meta_offset.to_le_bytes());
    buf[72..80].copy_from_slice(&meta_len.to_le_bytes());

    let checksum_xxh64 = compute_snapshot_checksum(&buf);
    buf[CHECKSUM_FIELD_OFFSET..CHECKSUM_FIELD_END].copy_from_slice(&checksum_xxh64.to_le_bytes());

    let header = SnapshotHeader {
        magic: NVE_SNAP_MAGIC,
        format_version: NVE_SNAP_FORMAT_VERSION,
        app_schema_version,
        flags,
        checksum_xxh64,
        frame0_offset,
        frame0_len,
        state_offset,
        state_len,
        meta_offset,
        meta_len,
    };
    (buf, header)
}

/// Writes an `NVE_SNAP` snapshot crash-safely via sibling temp file + `sync_all()` + atomic rename.
///
/// Any existing snapshot mapped by readers remains valid because the rename replaces the directory
/// entry with a newly written inode rather than mutating the previous file in place.
pub fn write_snapshot_atomic(
    path: impl AsRef<Path>,
    app_schema_version: u32,
    flags: u32,
    frame0_command_buffer: &[u8],
    state_bytes: &[u8],
    meta_bytes: &[u8],
) -> Result<SnapshotHeader, SnapshotError> {
    let dest = path.as_ref();
    let (bytes, header) = encode_snapshot_with_header(
        app_schema_version,
        flags,
        frame0_command_buffer,
        state_bytes,
        meta_bytes,
    );

    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("nve_snapshot");
    let seq = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".{file_name}.tmp.{}.{seq}", current_pid());
    let tmp_path = match dest.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(&tmp_name),
        _ => PathBuf::from(&tmp_name),
    };

    let write_result = (|| -> io::Result<()> {
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        fs::rename(&tmp_path, dest)?;
        #[cfg(unix)]
        if let Some(parent) = dest.parent() {
            let dir_path = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            let _ = fs::File::open(dir_path).and_then(|d| d.sync_all());
        }
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(SnapshotError::Io(err));
    }

    Ok(header)
}
