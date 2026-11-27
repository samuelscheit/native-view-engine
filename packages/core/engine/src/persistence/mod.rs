//! Crash-atomic persistence (`Frame0File` + `StateFile`) with zero `mmap` and zero `unsafe`.

mod frame0_file;
mod state_file;

use std::fmt;
use std::io;
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicU64, Ordering};

use nve_store::wire::WireError;

pub use frame0_file::{
    DEFAULT_FRAME0_FILE_NAME, FRAME0_FOOTER_SIZE, Frame0File, Frame0Footer, Frame0Meta, Frame0Miss,
    MAX_FRAME0_FILE_BYTES,
};
pub use state_file::{
    DEFAULT_STATE_FILE_NAME, MAX_STATE_FILE_BYTES, STATE_FILE_HEADER_SIZE, STATE_FILE_MAGIC,
    StateFile, StateRecoveryStats, StateSaveOutcome,
};

#[cfg(not(target_arch = "wasm32"))]
static ATOMIC_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[inline]
pub(crate) fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline]
pub(crate) fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

#[derive(Debug)]
pub enum PersistenceError {
    Io(io::Error),
    PayloadTooLarge(usize),
    InvalidNve2(WireError),
    Corrupt(&'static str),
    SchemaMismatch { expected: u32, actual: u32 },
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "persistence I/O error: {e}"),
            Self::PayloadTooLarge(n) => write!(f, "payload length {n} exceeds persistence limit"),
            Self::InvalidNve2(e) => write!(f, "invalid NVE2 keyframe: {e}"),
            Self::Corrupt(m) => write!(f, "corrupt persistence file: {m}"),
            Self::SchemaMismatch { expected, actual } => {
                write!(f, "schema mismatch: {expected} != {actual}")
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

impl From<io::Error> for PersistenceError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<WireError> for PersistenceError {
    fn from(e: WireError) -> Self {
        Self::InvalidNve2(e)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum ReadBoundedError {
    Io(io::Error),
    TooLarge,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file_bounded(
    file: &mut std::fs::File,
    max_bytes: usize,
    buf: &mut Vec<u8>,
) -> Result<(), ReadBoundedError> {
    use std::io::Read as _;

    buf.clear();
    if let Ok(meta) = file.metadata()
        && let Ok(len) = usize::try_from(meta.len())
    {
        let reserve_target = len.min(max_bytes);
        if buf.capacity() < reserve_target {
            let _ = buf.try_reserve(reserve_target.saturating_sub(buf.len()));
        }
    }
    let limit = (max_bytes as u64).saturating_add(1);
    file.take(limit)
        .read_to_end(buf)
        .map_err(ReadBoundedError::Io)?;
    if buf.len() > max_bytes {
        buf.clear();
        return Err(ReadBoundedError::TooLarge);
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rename_with_retry(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut attempt = 0u32;
        loop {
            match std::fs::rename(src, dst) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && attempt < 4 => {
                    std::thread::sleep(std::time::Duration::from_millis(1u64 << attempt));
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(src, dst)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn backup_rotated_file(path: &Path, suffix: &str) -> io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "persistence path must include a file name",
        ));
    };
    let mut backup_name = name.to_os_string();
    backup_name.push(".");
    backup_name.push(suffix);
    let backup = path.with_file_name(backup_name);
    rename_with_retry(path, &backup)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn backup_corrupt_file(path: &Path) {
    if backup_rotated_file(path, "corrupt").is_err() {
        let _ = std::fs::remove_file(path);
    }
}

#[inline]
pub(crate) fn remove_corrupt_frame0(disk_path: Option<&Path>) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = disk_path {
        let _ = std::fs::remove_file(path);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = disk_path;
}

/// Atomically writes `bytes` to `path` via exclusive `create_new` sibling temp file ->
/// optional `sync_all` -> `rename` -> optional parent-directory `sync_all`.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file_atomic(path: &Path, bytes: &[u8], sync: bool) -> Result<(), PersistenceError> {
    use std::io::Write as _;

    let Some(file_name) = path.file_name() else {
        return Err(PersistenceError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "persistence path must include a file name",
        )));
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u64, |d| d.as_nanos() as u64);
    let mut created: Option<(PathBuf, std::fs::File)> = None;
    for _ in 0..32 {
        let seq = ATOMIC_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let mut tmp_name = file_name.to_os_string();
        tmp_name.push(format!(".tmp.{pid}.{nonce:x}.{seq}"));
        let tmp = path.with_file_name(tmp_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => {
                created = Some((tmp, file));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(PersistenceError::Io(e)),
        }
    }
    let Some((tmp, mut file)) = created else {
        return Err(PersistenceError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate unique atomic temp file",
        )));
    };

    let write_res = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        if sync {
            file.sync_all()?;
        }
        Ok(())
    })();
    drop(file);

    if let Err(e) = write_res.and_then(|()| rename_with_retry(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(PersistenceError::Io(e));
    }
    if sync {
        sync_parent_dir(path)?;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn write_file_atomic(_path: &Path, _bytes: &[u8], _sync: bool) -> Result<(), PersistenceError> {
    Err(PersistenceError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        "unsupported on wasm32",
    )))
}
