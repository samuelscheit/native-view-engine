//! Zero-dependency cross-platform OS memory-mapping wrappers:
//! - Unix: POSIX `mmap`/`munmap`/`ftruncate`/`msync`/`getpagesize`
//! - Windows: Win32 `CreateFileMappingW`/`MapViewOfFile`/`FlushViewOfFile`/`UnmapViewOfFile`
//! - `wasm32` / other targets: clean in-memory `Vec<u8>` backing

#[cfg(not(any(unix, windows)))]
mod memory;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(not(any(unix, windows)))]
pub use memory::{page_size, AnonReservation, ReadOnlyMmap, ReadWriteMmap};
#[cfg(unix)]
pub use unix::{page_size, AnonReservation, ReadOnlyMmap, ReadWriteMmap};
#[cfg(windows)]
pub use windows::{page_size, AnonReservation, ReadOnlyMmap, ReadWriteMmap};

/// Writes a byte slice to disk (truncating existing content).
pub fn write_file_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)
}

/// Reads an entire file into a `Vec<u8>`.
pub fn read_file_bytes(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let len = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let mut out = Vec::with_capacity(len);
    file.read_to_end(&mut out)?;
    Ok(out)
}
