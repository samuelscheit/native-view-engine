//! POSIX `mmap`/`munmap`/`ftruncate`/`msync`/`getpagesize` implementation.

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
// SAFETY: Mapping is read-only (`PROT_READ | MAP_PRIVATE`) and safe to share across threads.
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

enum RwBacking {
    Mapped {
        ptr: *mut u8,
        len: usize,
        _file: File,
    },
    InMemory(Vec<u8>),
}

pub struct ReadWriteMmap {
    backing: RwBacking,
}

// SAFETY: Exclusive `&mut self` is required for mutation; owning struct is Send.
unsafe impl Send for ReadWriteMmap {}

impl ReadWriteMmap {
    pub fn in_memory(len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot create zero-length writable mmap",
            ));
        }
        Ok(Self {
            backing: RwBacking::InMemory(vec![0u8; len]),
        })
    }

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
            backing: RwBacking::Mapped {
                ptr: mapped as *mut u8,
                len,
                _file: file,
            },
        })
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match &self.backing {
            RwBacking::Mapped { ptr, len, .. } => {
                // SAFETY: Valid `len`-byte mapping owned by `self`.
                unsafe { std::slice::from_raw_parts(*ptr, *len) }
            }
            RwBacking::InMemory(buf) => buf.as_slice(),
        }
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.backing {
            RwBacking::Mapped { ptr, len, .. } => {
                // SAFETY: Valid `len`-byte writable mapping with exclusive `&mut self`.
                unsafe { std::slice::from_raw_parts_mut(*ptr, *len) }
            }
            RwBacking::InMemory(buf) => buf.as_mut_slice(),
        }
    }

    #[inline]
    pub fn base_addr(&self) -> usize {
        match &self.backing {
            RwBacking::Mapped { ptr, .. } => *ptr as usize,
            RwBacking::InMemory(buf) => buf.as_ptr() as usize,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.backing {
            RwBacking::Mapped { len, .. } => *len,
            RwBacking::InMemory(buf) => buf.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn sync_all(&self) -> io::Result<()> {
        self.sync_range(0, self.len())
    }

    pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
        if len == 0 {
            return Ok(());
        }
        let total = self.len();
        let end = offset.checked_add(len).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "msync range overflow")
        })?;
        if end > total {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "msync range exceeds mapping length",
            ));
        }
        match &self.backing {
            RwBacking::Mapped { ptr, .. } => {
                let ps = page_size();
                let aligned_start = offset & !(ps - 1);
                let aligned_len = end - aligned_start;
                // SAFETY: `ptr.add(aligned_start)` is page-aligned (`mmap` base is page-aligned)
                // and `aligned_start + aligned_len == end <= total`.
                let rc = unsafe {
                    msync(
                        ptr.add(aligned_start) as *mut c_void,
                        aligned_len,
                        MS_SYNC,
                    )
                };
                if rc != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }
            RwBacking::InMemory(_) => Ok(()),
        }
    }
}

impl Drop for ReadWriteMmap {
    fn drop(&mut self) {
        if let RwBacking::Mapped { ptr, len, .. } = &self.backing
            && *len > 0
        {
            // SAFETY: `ptr` and `len` came from a successful `mmap`.
            unsafe {
                let _ = munmap(*ptr as *mut c_void, *len);
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
