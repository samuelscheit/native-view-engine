use std::fmt;
use std::path::Path;

use super::arena::RelocatableTableView;
use super::format::{compute_snapshot_checksum, SnapshotError, SnapshotHeader};

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
    /// Opens and memory-maps a snapshot file, validating header, bounds, alignment, and XXH64 checksum.
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

    /// Verifies the stored XXH64 checksum against the mapped bytes.
    pub fn verify_checksum(&self) -> Result<(), SnapshotError> {
        let computed = compute_snapshot_checksum(self.as_bytes());
        if computed != self.header.checksum_xxh64 {
            return Err(SnapshotError::ChecksumMismatch {
                expected: self.header.checksum_xxh64,
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
    pub fn checksum_xxh64(&self) -> u64 {
        self.header.checksum_xxh64
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
