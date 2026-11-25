//! Zero-dependency POSIX `mmap`/`munmap`/`ftruncate`/`msync` wrappers (plus portable
//! non-Unix fallback) used by the `NVERELOC` and `MmapOffsetArena` prototypes.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

#[cfg(unix)]
mod imp {
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::AsRawFd;
    use std::path::Path;
    use std::ptr;

    const PROT_READ: i32 = 0x01;
    const PROT_WRITE: i32 = 0x02;
    const MAP_SHARED: i32 = 0x0001;
    const MAP_PRIVATE: i32 = 0x0002;
    const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const MS_SYNC: i32 = 0x0010;
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    const MS_SYNC: i32 = 0x0004;

    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    const MAP_ANON: i32 = 0x1000;
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    const MAP_ANON: i32 = 0x0020;

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
        fn ftruncate(fd: i32, length: OffT) -> i32;
        fn msync(addr: *mut c_void, len: usize, flags: i32) -> i32;
        fn getpagesize() -> i32;
    }

    pub fn page_size() -> usize {
        // SAFETY: `getpagesize` is a pure POSIX query with no preconditions.
        let ps = unsafe { getpagesize() };
        if ps > 0 { ps as usize } else { 4096 }
    }

    pub struct ReadOnlyMmap {
        ptr: *const u8,
        len: usize,
    }

    // SAFETY: Mapping is read-only (`PROT_READ | MAP_PRIVATE`) and immutable for its lifetime.
    unsafe impl Send for ReadOnlyMmap {}
    unsafe impl Sync for ReadOnlyMmap {}

    impl ReadOnlyMmap {
        pub fn open(path: &Path) -> io::Result<Self> {
            let file = File::open(path)?;
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "mmap path is not a regular file",
                ));
            }
            let len = usize::try_from(meta.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "file exceeds address space")
            })?;
            if len == 0 {
                return Ok(Self {
                    ptr: ptr::NonNull::<u8>::dangling().as_ptr(),
                    len: 0,
                });
            }
            // SAFETY: Regular open file descriptor, non-zero length, private read-only mapping.
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
        pub fn as_slice(&self) -> &[u8] {
            if self.len == 0 {
                &[]
            } else {
                // SAFETY: `self.ptr` points to a valid `self.len`-byte mapping until Drop.
                unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
            }
        }

        #[inline]
        pub fn base_addr(&self) -> usize {
            self.ptr as usize
        }

        #[inline]
        pub fn len(&self) -> usize {
            self.len
        }

        #[inline]
        pub fn is_empty(&self) -> bool {
            self.len == 0
        }
    }

    impl Drop for ReadOnlyMmap {
        fn drop(&mut self) {
            if self.len > 0 {
                // SAFETY: `self.ptr` and `self.len` came from a successful `mmap`.
                unsafe {
                    let _ = munmap(self.ptr as *mut c_void, self.len);
                }
            }
        }
    }

    pub struct ReadWriteMmap {
        ptr: *mut u8,
        len: usize,
        _file: File,
    }

    // SAFETY: Exclusive `&mut self` is required for mutation; owning struct is Send.
    unsafe impl Send for ReadWriteMmap {}

    impl ReadWriteMmap {
        pub fn create(path: &Path, len: usize) -> io::Result<Self> {
            if len == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot create zero-length writable mmap",
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)?;
            let off_len = OffT::try_from(len).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "mmap length exceeds off_t")
            })?;
            // SAFETY: Valid writable file descriptor and non-negative length.
            if unsafe { ftruncate(file.as_raw_fd(), off_len) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Self::map_file(file, len)
        }

        pub fn open(path: &Path) -> io::Result<Self> {
            let file = OpenOptions::new().read(true).write(true).open(path)?;
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "mmap path is not a regular file",
                ));
            }
            let len = usize::try_from(meta.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "file exceeds address space")
            })?;
            if len == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "cannot open zero-length writable mmap",
                ));
            }
            Self::map_file(file, len)
        }

        fn map_file(file: File, len: usize) -> io::Result<Self> {
            // SAFETY: Valid read/write file descriptor sized to `len`, shared mapping.
            let mapped = unsafe {
                mmap(
                    ptr::null_mut(),
                    len,
                    PROT_READ | PROT_WRITE,
                    MAP_SHARED,
                    file.as_raw_fd(),
                    0,
                )
            };
            if mapped == MAP_FAILED || mapped.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                ptr: mapped as *mut u8,
                len,
                _file: file,
            })
        }

        #[inline]
        pub fn as_slice(&self) -> &[u8] {
            // SAFETY: Valid `self.len`-byte mapping owned by `self`.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }

        #[inline]
        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            // SAFETY: Valid `self.len`-byte writable mapping with exclusive `&mut self`.
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
        }

        #[inline]
        pub fn base_addr(&self) -> usize {
            self.ptr as usize
        }

        #[inline]
        pub fn len(&self) -> usize {
            self.len
        }

        #[inline]
        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn sync_all(&self) -> io::Result<()> {
            self.sync_range(0, self.len)
        }

        pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
            if len == 0 {
                return Ok(());
            }
            let end = offset.checked_add(len).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "msync range overflow")
            })?;
            if end > self.len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "msync range exceeds mapping length",
                ));
            }
            let ps = page_size();
            let aligned_start = offset & !(ps - 1);
            let aligned_len = end - aligned_start;
            // SAFETY: `self.ptr.add(aligned_start)` is page-aligned (since `mmap` returns a
            // page-aligned base) and `aligned_start + aligned_len == end <= self.len`.
            let rc = unsafe {
                msync(
                    self.ptr.add(aligned_start) as *mut c_void,
                    aligned_len,
                    MS_SYNC,
                )
            };
            if rc != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Drop for ReadWriteMmap {
        fn drop(&mut self) {
            if self.len > 0 {
                // SAFETY: `self.ptr` and `self.len` came from a successful `mmap`.
                unsafe {
                    let _ = munmap(self.ptr as *mut c_void, self.len);
                }
            }
        }
    }

    /// Anonymous virtual-address reservation used in tests to verify offset-arena
    /// address-independence across distinct virtual mappings.
    pub struct AnonReservation {
        ptr: *mut c_void,
        len: usize,
    }

    impl AnonReservation {
        pub fn new(len: usize) -> io::Result<Self> {
            let alloc_len = len.max(page_size());
            // SAFETY: Anonymous private read-only reservation, fd = -1.
            let mapped = unsafe {
                mmap(
                    ptr::null_mut(),
                    alloc_len,
                    PROT_READ,
                    MAP_PRIVATE | MAP_ANON,
                    -1,
                    0,
                )
            };
            if mapped == MAP_FAILED || mapped.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                ptr: mapped,
                len: alloc_len,
            })
        }

        pub fn base_addr(&self) -> usize {
            self.ptr as usize
        }
    }

    impl Drop for AnonReservation {
        fn drop(&mut self) {
            // SAFETY: `self.ptr` and `self.len` came from `mmap`.
            unsafe {
                let _ = munmap(self.ptr, self.len);
            }
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};

    pub fn page_size() -> usize {
        4096
    }

    pub struct ReadOnlyMmap {
        buf: Vec<u8>,
    }

    impl ReadOnlyMmap {
        pub fn open(path: &Path) -> io::Result<Self> {
            let buf = std::fs::read(path)?;
            Ok(Self { buf })
        }

        #[inline]
        pub fn as_slice(&self) -> &[u8] {
            &self.buf
        }

        #[inline]
        pub fn base_addr(&self) -> usize {
            self.buf.as_ptr() as usize
        }

        #[inline]
        pub fn len(&self) -> usize {
            self.buf.len()
        }

        #[inline]
        pub fn is_empty(&self) -> bool {
            self.buf.is_empty()
        }
    }

    pub struct ReadWriteMmap {
        buf: Vec<u8>,
        path: PathBuf,
    }

    impl ReadWriteMmap {
        pub fn create(path: &Path, len: usize) -> io::Result<Self> {
            if len == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot create zero-length writable mmap",
                ));
            }
            let buf = vec![0u8; len];
            std::fs::write(path, &buf)?;
            Ok(Self {
                buf,
                path: path.to_path_buf(),
            })
        }

        pub fn open(path: &Path) -> io::Result<Self> {
            let mut file = File::open(path)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            if buf.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "cannot open zero-length writable mmap",
                ));
            }
            Ok(Self {
                buf,
                path: path.to_path_buf(),
            })
        }

        #[inline]
        pub fn as_slice(&self) -> &[u8] {
            &self.buf
        }

        #[inline]
        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            &mut self.buf
        }

        #[inline]
        pub fn base_addr(&self) -> usize {
            self.buf.as_ptr() as usize
        }

        #[inline]
        pub fn len(&self) -> usize {
            self.buf.len()
        }

        #[inline]
        pub fn is_empty(&self) -> bool {
            self.buf.is_empty()
        }

        pub fn sync_all(&self) -> io::Result<()> {
            let mut f = OpenOptions::new().write(true).open(&self.path)?;
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&self.buf)?;
            f.sync_data()
        }

        pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
            let end = offset.checked_add(len).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "sync range overflow")
            })?;
            if end > self.buf.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "sync range exceeds mapping length",
                ));
            }
            let mut f = OpenOptions::new().write(true).open(&self.path)?;
            f.seek(SeekFrom::Start(offset as u64))?;
            f.write_all(&self.buf[offset..end])?;
            f.sync_data()
        }
    }

    impl Drop for ReadWriteMmap {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.path, &self.buf);
        }
    }

    pub struct AnonReservation {
        buf: Vec<u8>,
    }

    impl AnonReservation {
        pub fn new(len: usize) -> io::Result<Self> {
            Ok(Self {
                buf: vec![0u8; len.max(4096)],
            })
        }

        pub fn base_addr(&self) -> usize {
            self.buf.as_ptr() as usize
        }
    }
}

pub use imp::{page_size, AnonReservation, ReadOnlyMmap, ReadWriteMmap};

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
