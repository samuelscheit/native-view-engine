//! Instant-Boot Frame-0 `CommandBuffer` freeze-dry + zero-copy `mmap` state snapshot.
//!
//! # Binary Layout (`NVE_SNAP`, little-endian, 8-byte aligned sections)
//!
//! Fixed 80-byte header (`SNAPSHOT_HEADER_SIZE`):
//! - `0..8`:   `magic: [u8; 8]` = `*b"NVE_SNAP"`
//! - `8..12`:  `format_version: u32` = `1`
//! - `12..16`: `app_schema_version: u32` (caller-supplied schema version)
//! - `16..20`: `flags: u32`
//! - `20..24`: `reserved: u32` (must be `0`, preserves 8-byte alignment)
//! - `24..32`: `checksum_fnv64: u64` (FNV-1a 64-bit over `0..24` + `32..file_len`)
//! - `32..40`: `frame0_offset: u64` (8-byte aligned offset of the Frame-0 `CommandBuffer`)
//! - `40..48`: `frame0_len: u64`
//! - `48..56`: `state_offset: u64` (8-byte aligned offset of relocatable app state)
//! - `56..64`: `state_len: u64`
//! - `64..72`: `meta_offset: u64` (8-byte aligned offset of optional metadata/viewport bytes)
//! - `72..80`: `meta_len: u64`
//!
//! Followed by zero-padded, 8-byte aligned payload sections (`frame0`, `state`, `meta`).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 8-byte file magic identifying an NVE instant-boot snapshot container.
pub const NVE_SNAP_MAGIC: [u8; 8] = *b"NVE_SNAP";

/// Current snapshot container format version.
pub const NVE_SNAP_FORMAT_VERSION: u32 = 1;

/// Fixed byte length of the aligned `NVE_SNAP` header (32-byte prefix + 48-byte section directory).
pub const SNAPSHOT_HEADER_SIZE: usize = 80;

/// Byte alignment enforced for every payload section offset.
pub const SNAPSHOT_SECTION_ALIGN: usize = 8;

/// Offset of the 8-byte `checksum_fnv64` field inside the header.
const CHECKSUM_FIELD_OFFSET: usize = 24;
const CHECKSUM_FIELD_END: usize = 32;

const FNV1A_64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_64_PRIME: u64 = 0x0000_0100_0000_01b3;

static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Computes FNV-1a 64-bit hash over `bytes`.
#[inline]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    fnv1a_64_update(FNV1A_64_OFFSET_BASIS, bytes)
}

/// Continues an FNV-1a 64-bit hash with additional `bytes`.
#[inline]
pub fn fnv1a_64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV1A_64_PRIME);
    }
    hash
}

#[inline]
fn align_up_8(n: usize) -> usize {
    (n + (SNAPSHOT_SECTION_ALIGN - 1)) & !(SNAPSHOT_SECTION_ALIGN - 1)
}

#[inline]
fn pad_to_8(buf: &mut Vec<u8>) {
    let aligned = align_up_8(buf.len());
    if aligned > buf.len() {
        buf.resize(aligned, 0);
    }
}

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

/// Errors produced when writing, mapping, validating, or reading an NVE snapshot.
#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    Truncated {
        expected: usize,
        actual: usize,
    },
    InvalidMagic([u8; 8]),
    UnsupportedFormatVersion(u32),
    InvalidReservedHeader(u32),
    SchemaVersionMismatch {
        expected: u32,
        actual: u32,
    },
    ChecksumMismatch {
        expected: u64,
        actual: u64,
    },
    UnalignedSection {
        section: &'static str,
        offset: u64,
    },
    SectionOutOfBounds {
        section: &'static str,
        offset: u64,
        len: u64,
        file_len: u64,
    },
    OverlappingSections {
        first: &'static str,
        second: &'static str,
    },
    InvalidRelocatableArena(&'static str),
}

impl From<io::Error> for SnapshotError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "snapshot I/O error: {e}"),
            Self::Truncated { expected, actual } => {
                write!(f, "truncated snapshot: expected at least {expected} bytes, got {actual}")
            }
            Self::InvalidMagic(magic) => write!(f, "invalid snapshot magic: {magic:?}"),
            Self::UnsupportedFormatVersion(v) => {
                write!(f, "unsupported snapshot format version: {v}")
            }
            Self::InvalidReservedHeader(v) => {
                write!(f, "non-zero reserved snapshot header field: {v:#x}")
            }
            Self::SchemaVersionMismatch { expected, actual } => {
                write!(f, "snapshot schema version mismatch: expected {expected}, got {actual}")
            }
            Self::ChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "snapshot FNV-1a checksum mismatch: expected {expected:#018x}, computed {actual:#018x}"
                )
            }
            Self::UnalignedSection { section, offset } => {
                write!(
                    f,
                    "snapshot section '{section}' offset {offset} is not {SNAPSHOT_SECTION_ALIGN}-byte aligned"
                )
            }
            Self::SectionOutOfBounds {
                section,
                offset,
                len,
                file_len,
            } => {
                write!(
                    f,
                    "snapshot section '{section}' ({offset}..+{len}) out of bounds for {file_len}-byte snapshot"
                )
            }
            Self::OverlappingSections { first, second } => {
                write!(f, "snapshot sections '{first}' and '{second}' overlap")
            }
            Self::InvalidRelocatableArena(msg) => {
                write!(f, "invalid relocatable state arena: {msg}")
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Decoded fixed header of an `NVE_SNAP` binary container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub magic: [u8; 8],
    pub format_version: u32,
    pub app_schema_version: u32,
    pub flags: u32,
    pub checksum_fnv64: u64,
    pub frame0_offset: u64,
    pub frame0_len: u64,
    pub state_offset: u64,
    pub state_len: u64,
    pub meta_offset: u64,
    pub meta_len: u64,
}

impl SnapshotHeader {
    fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < SNAPSHOT_HEADER_SIZE {
            return Err(SnapshotError::Truncated {
                expected: SNAPSHOT_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let magic: [u8; 8] = bytes[0..8].try_into().unwrap();
        if magic != NVE_SNAP_MAGIC {
            return Err(SnapshotError::InvalidMagic(magic));
        }
        let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if format_version != NVE_SNAP_FORMAT_VERSION {
            return Err(SnapshotError::UnsupportedFormatVersion(format_version));
        }
        let app_schema_version = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        if reserved != 0 {
            return Err(SnapshotError::InvalidReservedHeader(reserved));
        }
        let checksum_fnv64 = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let frame0_offset = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        let frame0_len = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let state_offset = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
        let state_len = u64::from_le_bytes(bytes[56..64].try_into().unwrap());
        let meta_offset = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
        let meta_len = u64::from_le_bytes(bytes[72..80].try_into().unwrap());

        let header = Self {
            magic,
            format_version,
            app_schema_version,
            flags,
            checksum_fnv64,
            frame0_offset,
            frame0_len,
            state_offset,
            state_len,
            meta_offset,
            meta_len,
        };
        header.validate_sections(bytes.len() as u64)?;
        Ok(header)
    }

    fn validate_sections(&self, file_len: u64) -> Result<(), SnapshotError> {
        let sections = [
            ("frame0", self.frame0_offset, self.frame0_len),
            ("state", self.state_offset, self.state_len),
            ("meta", self.meta_offset, self.meta_len),
        ];

        let mut spans: [(&'static str, u64, u64); 3] = [("", 0, 0); 3];
        for (idx, &(name, offset, len)) in sections.iter().enumerate() {
            if !offset.is_multiple_of(SNAPSHOT_SECTION_ALIGN as u64) {
                return Err(SnapshotError::UnalignedSection {
                    section: name,
                    offset,
                });
            }
            if offset < SNAPSHOT_HEADER_SIZE as u64 {
                return Err(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                });
            }
            let end = offset
                .checked_add(len)
                .ok_or(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                })?;
            if end > file_len || usize::try_from(end).is_err() {
                return Err(SnapshotError::SectionOutOfBounds {
                    section: name,
                    offset,
                    len,
                    file_len,
                });
            }
            spans[idx] = (name, offset, end);
        }

        // Reject overlapping non-empty sections.
        for i in 0..spans.len() {
            let (name_a, start_a, end_a) = spans[i];
            if start_a == end_a {
                continue;
            }
            for &(name_b, start_b, end_b) in &spans[(i + 1)..] {
                if start_b == end_b {
                    continue;
                }
                if start_a < end_b && start_b < end_a {
                    return Err(SnapshotError::OverlappingSections {
                        first: name_a,
                        second: name_b,
                    });
                }
            }
        }

        Ok(())
    }
}

/// Computes the canonical container checksum over the header prefix (`0..24`),
/// header tail (`32..80`), and all payload + padding bytes (`80..`).
fn compute_snapshot_checksum(bytes: &[u8]) -> u64 {
    let h = fnv1a_64_update(FNV1A_64_OFFSET_BASIS, &bytes[0..CHECKSUM_FIELD_OFFSET]);
    fnv1a_64_update(h, &bytes[CHECKSUM_FIELD_END..])
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

    let checksum_fnv64 = compute_snapshot_checksum(&buf);
    buf[CHECKSUM_FIELD_OFFSET..CHECKSUM_FIELD_END].copy_from_slice(&checksum_fnv64.to_le_bytes());

    let header = SnapshotHeader {
        magic: NVE_SNAP_MAGIC,
        format_version: NVE_SNAP_FORMAT_VERSION,
        app_schema_version,
        flags,
        checksum_fnv64,
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
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
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

// ============================================================================
// Platform Zero-Copy Memory Mapping (zero external crates)
// ============================================================================

#[cfg(unix)]
mod platform {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;
    use std::path::Path;
    use std::ptr;

    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;
    const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

    #[cfg(any(target_pointer_width = "64", target_vendor = "apple"))]
    type OffT = i64;
    #[cfg(not(any(target_pointer_width = "64", target_vendor = "apple")))]
    type OffT = isize;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: OffT,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> i32;
    }

    pub(super) struct MappedRegion {
        ptr: *const u8,
        len: usize,
    }

    // SAFETY: The mapped region is read-only (PROT_READ | MAP_PRIVATE) and never mutated.
    unsafe impl Send for MappedRegion {}
    unsafe impl Sync for MappedRegion {}

    impl MappedRegion {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            let file = File::open(path)?;
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "snapshot path is not a regular file",
                ));
            }
            let len = usize::try_from(meta.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "snapshot exceeds address space")
            })?;
            if len == 0 {
                // POSIX mmap rejects len == 0 with EINVAL; return an empty region directly.
                return Ok(Self {
                    ptr: ptr::NonNull::<u8>::dangling().as_ptr(),
                    len: 0,
                });
            }
            // SAFETY: Valid open regular-file fd, non-zero length, read-only private mapping.
            let mapped = unsafe {
                mmap(
                    ptr::null_mut(),
                    len,
                    PROT_READ,
                    MAP_PRIVATE,
                    file.as_raw_fd(),
                    0,
                )
            };
            if mapped == MAP_FAILED || mapped.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                ptr: mapped as *const u8,
                len,
            })
        }

        #[inline]
        pub(super) fn as_slice(&self) -> &[u8] {
            if self.len == 0 {
                &[]
            } else {
                // SAFETY: `self.ptr` is a valid read-only mapping of `self.len` bytes until Drop.
                unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
            }
        }
    }

    impl Drop for MappedRegion {
        fn drop(&mut self) {
            if self.len > 0 {
                // SAFETY: `self.ptr` and `self.len` were returned by a successful `mmap`.
                unsafe {
                    let _ = munmap(self.ptr as *mut c_void, self.len);
                }
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::ptr;

    type Handle = *mut c_void;

    const PAGE_READONLY: u32 = 0x02;
    const FILE_MAP_READ: u32 = 0x0004;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileMappingW(
            hfile: Handle,
            attributes: *const c_void,
            fl_protect: u32,
            max_size_high: u32,
            max_size_low: u32,
            name: *const u16,
        ) -> Handle;
        fn MapViewOfFile(
            hfile_mapping_object: Handle,
            dw_desired_access: u32,
            dw_file_offset_high: u32,
            dw_file_offset_low: u32,
            dw_number_of_bytes_to_map: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(lp_base_address: *const c_void) -> i32;
        fn CloseHandle(hobject: Handle) -> i32;
    }

    pub(super) struct MappedRegion {
        ptr: *const u8,
        len: usize,
        mapping: Handle,
    }

    // SAFETY: The mapped view is read-only (PAGE_READONLY / FILE_MAP_READ) and immutable until Drop.
    unsafe impl Send for MappedRegion {}
    unsafe impl Sync for MappedRegion {}

    impl MappedRegion {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            let file = File::open(path)?;
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "snapshot path is not a regular file",
                ));
            }
            let len = usize::try_from(meta.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "snapshot exceeds address space")
            })?;
            if len == 0 {
                // CreateFileMappingW returns ERROR_FILE_INVALID for 0-byte files.
                return Ok(Self {
                    ptr: ptr::NonNull::<u8>::dangling().as_ptr(),
                    len: 0,
                    mapping: ptr::null_mut(),
                });
            }
            // SAFETY: Valid file handle, read-only mapping covering the full file.
            let mapping = unsafe {
                CreateFileMappingW(
                    file.as_raw_handle() as Handle,
                    ptr::null(),
                    PAGE_READONLY,
                    0,
                    0,
                    ptr::null(),
                )
            };
            if mapping.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: Valid file mapping handle, read-only view of `len` bytes.
            let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, len) };
            if view.is_null() {
                let err = io::Error::last_os_error();
                unsafe {
                    let _ = CloseHandle(mapping);
                }
                return Err(err);
            }
            Ok(Self {
                ptr: view as *const u8,
                len,
                mapping,
            })
        }

        #[inline]
        pub(super) fn as_slice(&self) -> &[u8] {
            if self.len == 0 {
                &[]
            } else {
                // SAFETY: `self.ptr` points to a live read-only view of `self.len` bytes until Drop.
                unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
            }
        }
    }

    impl Drop for MappedRegion {
        fn drop(&mut self) {
            if self.len > 0 && !self.ptr.is_null() {
                unsafe {
                    let _ = UnmapViewOfFile(self.ptr as *const c_void);
                }
            }
            if !self.mapping.is_null() {
                unsafe {
                    let _ = CloseHandle(self.mapping);
                }
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::io;
    use std::path::Path;

    pub(super) struct MappedRegion {
        bytes: Vec<u8>,
    }

    impl MappedRegion {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            let bytes = std::fs::read(path)?;
            Ok(Self { bytes })
        }

        #[inline]
        pub(super) fn as_slice(&self) -> &[u8] {
            &self.bytes
        }
    }
}

enum SnapshotBacking {
    Mapped(platform::MappedRegion),
    Owned(Vec<u8>),
}

impl SnapshotBacking {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mapped(region) => region.as_slice(),
            Self::Owned(vec) => vec.as_slice(),
        }
    }
}

/// Read-only zero-copy memory-mapped `NVE_SNAP` snapshot handle.
///
/// All section accessors borrow directly out of the mapped virtual address range without
/// allocating or copying payload bytes.
pub struct MmapSnapshot {
    backing: SnapshotBacking,
    header: SnapshotHeader,
}

impl MmapSnapshot {
    /// Opens and memory-maps a snapshot file, validating header, bounds, alignment, and FNV-1a checksum.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let region = platform::MappedRegion::open(path.as_ref())?;
        Self::from_backing(SnapshotBacking::Mapped(region), true)
    }

    /// Opens and memory-maps a snapshot file, validating header/alignment/bounds without scanning
    /// the payload for checksum verification.
    ///
    /// Call [`Self::verify_checksum`] later (e.g. off the Frame-0 critical path) when deferred
    /// integrity verification is desired.
    pub fn open_unchecked_checksum(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let region = platform::MappedRegion::open(path.as_ref())?;
        Self::from_backing(SnapshotBacking::Mapped(region), false)
    }

    /// Opens a snapshot file, validates integrity, and enforces `app_schema_version == expected_schema_version`.
    pub fn open_with_schema(
        path: impl AsRef<Path>,
        expected_schema_version: u32,
    ) -> Result<Self, SnapshotError> {
        let snap = Self::open(path)?;
        snap.verify_schema(expected_schema_version)?;
        Ok(snap)
    }

    /// Wraps an in-memory snapshot buffer with full header, alignment, bounds, and checksum validation.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SnapshotError> {
        Self::from_backing(SnapshotBacking::Owned(bytes), true)
    }

    /// Wraps an in-memory snapshot buffer, validating header/alignment/bounds without verifying checksum.
    pub fn from_bytes_unchecked_checksum(bytes: Vec<u8>) -> Result<Self, SnapshotError> {
        Self::from_backing(SnapshotBacking::Owned(bytes), false)
    }

    /// Wraps an in-memory snapshot buffer and enforces `app_schema_version == expected_schema_version`.
    pub fn from_bytes_with_schema(
        bytes: Vec<u8>,
        expected_schema_version: u32,
    ) -> Result<Self, SnapshotError> {
        let snap = Self::from_bytes(bytes)?;
        snap.verify_schema(expected_schema_version)?;
        Ok(snap)
    }

    fn from_backing(backing: SnapshotBacking, verify_checksum: bool) -> Result<Self, SnapshotError> {
        let header = SnapshotHeader::decode(backing.as_slice())?;
        let snap = Self { backing, header };
        if verify_checksum {
            snap.verify_checksum()?;
        }
        Ok(snap)
    }

    /// Verifies the stored FNV-1a 64-bit checksum against the mapped bytes.
    pub fn verify_checksum(&self) -> Result<(), SnapshotError> {
        let computed = compute_snapshot_checksum(self.as_bytes());
        if computed != self.header.checksum_fnv64 {
            return Err(SnapshotError::ChecksumMismatch {
                expected: self.header.checksum_fnv64,
                actual: computed,
            });
        }
        Ok(())
    }

    /// Ensures the snapshot's application schema version matches `expected`.
    pub fn verify_schema(&self, expected: u32) -> Result<(), SnapshotError> {
        let actual = self.header.app_schema_version;
        if actual != expected {
            return Err(SnapshotError::SchemaVersionMismatch { expected, actual });
        }
        Ok(())
    }

    #[inline]
    pub fn header(&self) -> SnapshotHeader {
        self.header
    }

    #[inline]
    pub fn format_version(&self) -> u32 {
        self.header.format_version
    }

    #[inline]
    pub fn app_schema_version(&self) -> u32 {
        self.header.app_schema_version
    }

    #[inline]
    pub fn flags(&self) -> u32 {
        self.header.flags
    }

    #[inline]
    pub fn checksum_fnv64(&self) -> u64 {
        self.header.checksum_fnv64
    }

    /// Returns the entire mapped snapshot byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.backing.as_slice()
    }

    /// Zero-copy borrowed slice of the freeze-dried Frame-0 `CommandBuffer`.
    #[inline]
    pub fn frame0_command_buffer(&self) -> &[u8] {
        let start = self.header.frame0_offset as usize;
        let end = start + self.header.frame0_len as usize;
        &self.as_bytes()[start..end]
    }

    /// Zero-copy borrowed slice of the relocatable application state section.
    #[inline]
    pub fn state_bytes(&self) -> &[u8] {
        let start = self.header.state_offset as usize;
        let end = start + self.header.state_len as usize;
        &self.as_bytes()[start..end]
    }

    /// Zero-copy borrowed slice of the optional metadata/viewport/generation section.
    #[inline]
    pub fn meta_bytes(&self) -> &[u8] {
        let start = self.header.meta_offset as usize;
        let end = start + self.header.meta_len as usize;
        &self.as_bytes()[start..end]
    }

    /// Opens a zero-copy [`RelocatableTableView`] directly over [`Self::state_bytes`].
    #[inline]
    pub fn state_table(&self) -> Result<RelocatableTableView<'_>, SnapshotError> {
        RelocatableTableView::from_bytes(self.state_bytes())
    }
}

impl fmt::Debug for MmapSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MmapSnapshot")
            .field("len", &self.as_bytes().len())
            .field("header", &self.header)
            .finish()
    }
}

// ============================================================================
// Pointer-Free Relocatable Offset-Arena (`RelocatableStateArena` / `RelocatableTableView`)
// ============================================================================

/// Magic identifying a relocatable offset-indexed state table (`NVERELOC`).
pub const RELOCATABLE_ARENA_MAGIC: [u8; 8] = *b"NVERELOC";

/// Format version for `RelocatableStateArena` / `RelocatableTableView`.
pub const RELOCATABLE_ARENA_VERSION: u32 = 1;

const ARENA_HEADER_SIZE: usize = 16;
const ARENA_SLOT_SIZE: usize = 32;

const VALUE_KIND_I64: u32 = 1;
const VALUE_KIND_F64: u32 = 2;
const VALUE_KIND_STR: u32 = 3;
const VALUE_KIND_BYTES: u32 = 4;
const VALUE_KIND_BOOL: u32 = 5;

#[derive(Debug, Clone)]
enum OwnedValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
}

/// Borrowed zero-copy value read directly out of a [`RelocatableTableView`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RelocatableValue<'a> {
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(&'a str),
    Bytes(&'a [u8]),
}

/// Builder for a pointer-free, offset-indexed key/value state blob suitable for embedding
/// inside an `NVE_SNAP` state section and reading directly from `mmap` with zero allocations.
#[derive(Debug, Default, Clone)]
pub struct RelocatableStateArena {
    entries: BTreeMap<String, OwnedValue>,
}

impl RelocatableStateArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_i64(&mut self, key: impl Into<String>, value: i64) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::I64(value));
        self
    }

    pub fn insert_f64(&mut self, key: impl Into<String>, value: f64) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::F64(value));
        self
    }

    pub fn insert_bool(&mut self, key: impl Into<String>, value: bool) -> &mut Self {
        self.entries.insert(key.into(), OwnedValue::Bool(value));
        self
    }

    pub fn insert_str(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.entries
            .insert(key.into(), OwnedValue::Str(value.into()));
        self
    }

    pub fn insert_bytes(&mut self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> &mut Self {
        self.entries
            .insert(key.into(), OwnedValue::Bytes(value.into()));
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serializes all entries into a compact, deterministic, pointer-free binary buffer sorted by
    /// `(fnv1a_64(key), key)` for `O(log N)` zero-allocation binary-search lookup.
    pub fn finish(&self) -> Vec<u8> {
        let mut sorted: Vec<(u64, &str, &OwnedValue)> = self
            .entries
            .iter()
            .map(|(k, v)| (fnv1a_64(k.as_bytes()), k.as_str(), v))
            .collect();
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_bytes().cmp(b.1.as_bytes())));

        let entry_count = u32::try_from(sorted.len()).expect("too many entries in state arena");
        let table_size = sorted.len() * ARENA_SLOT_SIZE;
        let data_start = ARENA_HEADER_SIZE + table_size;

        let payload_bytes_estimate: usize = sorted
            .iter()
            .map(|(_, k, v)| {
                k.len()
                    + match v {
                        OwnedValue::Str(s) => s.len(),
                        OwnedValue::Bytes(b) => b.len(),
                        OwnedValue::I64(_) | OwnedValue::F64(_) | OwnedValue::Bool(_) => 0,
                    }
            })
            .sum();

        let mut buf = Vec::with_capacity(data_start + payload_bytes_estimate);
        buf.extend_from_slice(&RELOCATABLE_ARENA_MAGIC);
        buf.extend_from_slice(&RELOCATABLE_ARENA_VERSION.to_le_bytes());
        buf.extend_from_slice(&entry_count.to_le_bytes());
        buf.resize(data_start, 0);

        for (idx, (key_hash, key, val)) in sorted.into_iter().enumerate() {
            let key_offset = u32::try_from(buf.len()).expect("relocatable arena exceeds 4 GiB");
            let key_bytes = key.as_bytes();
            let key_len = u32::try_from(key_bytes.len()).expect("key exceeds 4 GiB");
            buf.extend_from_slice(key_bytes);

            let (value_kind, value_len, payload_or_offset): (u32, u32, u64) = match val {
                OwnedValue::I64(v) => (VALUE_KIND_I64, 8, *v as u64),
                OwnedValue::F64(v) => (VALUE_KIND_F64, 8, v.to_bits()),
                OwnedValue::Bool(v) => (VALUE_KIND_BOOL, 1, u64::from(*v)),
                OwnedValue::Str(s) => {
                    let offset = buf.len() as u64;
                    let b = s.as_bytes();
                    let len = u32::try_from(b.len()).expect("string value exceeds 4 GiB");
                    buf.extend_from_slice(b);
                    (VALUE_KIND_STR, len, offset)
                }
                OwnedValue::Bytes(b) => {
                    let offset = buf.len() as u64;
                    let len = u32::try_from(b.len()).expect("byte value exceeds 4 GiB");
                    buf.extend_from_slice(b);
                    (VALUE_KIND_BYTES, len, offset)
                }
            };

            let slot_pos = ARENA_HEADER_SIZE + idx * ARENA_SLOT_SIZE;
            buf[slot_pos..slot_pos + 8].copy_from_slice(&key_hash.to_le_bytes());
            buf[slot_pos + 8..slot_pos + 12].copy_from_slice(&key_offset.to_le_bytes());
            buf[slot_pos + 12..slot_pos + 16].copy_from_slice(&key_len.to_le_bytes());
            buf[slot_pos + 16..slot_pos + 20].copy_from_slice(&value_kind.to_le_bytes());
            buf[slot_pos + 20..slot_pos + 24].copy_from_slice(&value_len.to_le_bytes());
            buf[slot_pos + 24..slot_pos + 32].copy_from_slice(&payload_or_offset.to_le_bytes());
        }

        buf
    }
}

#[derive(Debug, Clone, Copy)]
struct ArenaSlot {
    key_hash: u64,
    key_offset: u32,
    key_len: u32,
    value_kind: u32,
    value_len: u32,
    payload_or_offset: u64,
}

/// Zero-copy, zero-allocation borrowed view into a [`RelocatableStateArena`] byte slice
/// (typically straight from [`MmapSnapshot::state_bytes`]).
#[derive(Debug, Clone, Copy)]
pub struct RelocatableTableView<'a> {
    bytes: &'a [u8],
    entry_count: usize,
    data_start: usize,
}

impl<'a> RelocatableTableView<'a> {
    /// Opens a relocatable table view in `O(1)` time without touching payload pages.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, SnapshotError> {
        if bytes.len() < ARENA_HEADER_SIZE {
            return Err(SnapshotError::InvalidRelocatableArena(
                "truncated relocatable arena header",
            ));
        }
        if bytes[0..8] != RELOCATABLE_ARENA_MAGIC {
            return Err(SnapshotError::InvalidRelocatableArena(
                "invalid relocatable arena magic",
            ));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != RELOCATABLE_ARENA_VERSION {
            return Err(SnapshotError::InvalidRelocatableArena(
                "unsupported relocatable arena version",
            ));
        }
        let entry_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let table_bytes = entry_count
            .checked_mul(ARENA_SLOT_SIZE)
            .ok_or(SnapshotError::InvalidRelocatableArena(
                "entry table size overflow",
            ))?;
        let data_start = ARENA_HEADER_SIZE
            .checked_add(table_bytes)
            .ok_or(SnapshotError::InvalidRelocatableArena(
                "data_start overflow",
            ))?;
        if data_start > bytes.len() {
            return Err(SnapshotError::InvalidRelocatableArena(
                "entry table extends past end of buffer",
            ));
        }
        Ok(Self {
            bytes,
            entry_count,
            data_start,
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entry_count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entry_count == 0
    }

    /// Eagerly validates every slot's bounds, key sort order, UTF-8 validity, and value kind.
    pub fn validate_all(&self) -> Result<(), SnapshotError> {
        let mut prev: Option<(u64, &'a [u8])> = None;
        for idx in 0..self.entry_count {
            let slot = self.read_slot(idx);
            let key_bytes = self
                .slot_key_bytes(slot)
                .ok_or(SnapshotError::InvalidRelocatableArena(
                    "slot key range out of bounds",
                ))?;
            let _ = std::str::from_utf8(key_bytes).map_err(|_| {
                SnapshotError::InvalidRelocatableArena("slot key is not valid UTF-8")
            })?;
            if fnv1a_64(key_bytes) != slot.key_hash {
                return Err(SnapshotError::InvalidRelocatableArena(
                    "slot key hash mismatch",
                ));
            }
            if let Some((prev_hash, prev_key)) = prev {
                let ord = prev_hash
                    .cmp(&slot.key_hash)
                    .then_with(|| prev_key.cmp(key_bytes));
                if !ord.is_lt() {
                    return Err(SnapshotError::InvalidRelocatableArena(
                        "entry table is not strictly sorted",
                    ));
                }
            }
            prev = Some((slot.key_hash, key_bytes));
            let _ = self
                .decode_slot_value(slot)
                .ok_or(SnapshotError::InvalidRelocatableArena(
                    "slot value is malformed or out of bounds",
                ))?;
        }
        Ok(())
    }

    #[inline]
    fn read_slot(&self, index: usize) -> ArenaSlot {
        let base = ARENA_HEADER_SIZE + index * ARENA_SLOT_SIZE;
        let s = &self.bytes[base..base + ARENA_SLOT_SIZE];
        ArenaSlot {
            key_hash: u64::from_le_bytes(s[0..8].try_into().unwrap()),
            key_offset: u32::from_le_bytes(s[8..12].try_into().unwrap()),
            key_len: u32::from_le_bytes(s[12..16].try_into().unwrap()),
            value_kind: u32::from_le_bytes(s[16..20].try_into().unwrap()),
            value_len: u32::from_le_bytes(s[20..24].try_into().unwrap()),
            payload_or_offset: u64::from_le_bytes(s[24..32].try_into().unwrap()),
        }
    }

    #[inline]
    fn slot_key_bytes(&self, slot: ArenaSlot) -> Option<&'a [u8]> {
        let start = slot.key_offset as usize;
        if start < self.data_start {
            return None;
        }
        let end = start.checked_add(slot.key_len as usize)?;
        self.bytes.get(start..end)
    }

    #[inline]
    fn slot_payload_bytes(&self, slot: ArenaSlot) -> Option<&'a [u8]> {
        let start = usize::try_from(slot.payload_or_offset).ok()?;
        if start < self.data_start {
            return None;
        }
        let end = start.checked_add(slot.value_len as usize)?;
        self.bytes.get(start..end)
    }

    fn decode_slot_value(&self, slot: ArenaSlot) -> Option<RelocatableValue<'a>> {
        match slot.value_kind {
            VALUE_KIND_I64 if slot.value_len == 8 => {
                Some(RelocatableValue::I64(slot.payload_or_offset as i64))
            }
            VALUE_KIND_F64 if slot.value_len == 8 => {
                Some(RelocatableValue::F64(f64::from_bits(slot.payload_or_offset)))
            }
            VALUE_KIND_BOOL if slot.value_len == 1 && slot.payload_or_offset <= 1 => {
                Some(RelocatableValue::Bool(slot.payload_or_offset != 0))
            }
            VALUE_KIND_STR => {
                let raw = self.slot_payload_bytes(slot)?;
                let s = std::str::from_utf8(raw).ok()?;
                Some(RelocatableValue::Str(s))
            }
            VALUE_KIND_BYTES => {
                let raw = self.slot_payload_bytes(slot)?;
                Some(RelocatableValue::Bytes(raw))
            }
            _ => None,
        }
    }

    fn find_slot(&self, key: &str) -> Option<ArenaSlot> {
        let key_bytes = key.as_bytes();
        let target_hash = fnv1a_64(key_bytes);
        let mut lo = 0usize;
        let mut hi = self.entry_count;

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let slot = self.read_slot(mid);
            let ord = match slot.key_hash.cmp(&target_hash) {
                std::cmp::Ordering::Equal => {
                    let mid_key = self.slot_key_bytes(slot)?;
                    mid_key.cmp(key_bytes)
                }
                other => other,
            };
            match ord {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(slot),
            }
        }
        None
    }

    /// Returns true if an entry exists for `key`.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.find_slot(key).is_some()
    }

    /// Looks up any typed value by `key` with zero allocations.
    #[inline]
    pub fn get(&self, key: &str) -> Option<RelocatableValue<'a>> {
        let slot = self.find_slot(key)?;
        self.decode_slot_value(slot)
    }

    /// Looks up an `i64` entry by `key` with zero allocations.
    #[inline]
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            RelocatableValue::I64(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up an `f64` entry by `key` with zero allocations.
    #[inline]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            RelocatableValue::F64(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up a `bool` entry by `key` with zero allocations.
    #[inline]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            RelocatableValue::Bool(v) => Some(v),
            _ => None,
        }
    }

    /// Looks up a borrowed `&'a str` entry directly inside the mapped buffer with zero allocations.
    #[inline]
    pub fn get_str(&self, key: &str) -> Option<&'a str> {
        match self.get(key)? {
            RelocatableValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Looks up a borrowed `&'a [u8]` entry directly inside the mapped buffer with zero allocations.
    #[inline]
    pub fn get_bytes(&self, key: &str) -> Option<&'a [u8]> {
        match self.get(key)? {
            RelocatableValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Iterates all valid `(key, value)` pairs in sorted hash order without allocating.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, RelocatableValue<'a>)> + '_ {
        (0..self.entry_count).filter_map(|idx| {
            let slot = self.read_slot(idx);
            let key = std::str::from_utf8(self.slot_key_bytes(slot)?).ok()?;
            let val = self.decode_slot_value(slot)?;
            Some((key, val))
        })
    }
}
