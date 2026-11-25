//! Win32 `CreateFileMappingW`/`MapViewOfFile`/`FlushViewOfFile`/`UnmapViewOfFile` implementation.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;

type Handle = *mut c_void;

const PAGE_READONLY: u32 = 0x02;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_WRITE: u32 = 0x0002;
const FILE_MAP_READ: u32 = 0x0004;
const MEM_RESERVE: u32 = 0x0000_2000;
const MEM_RELEASE: u32 = 0x0000_8000;
const PAGE_NOACCESS: u32 = 0x01;

#[repr(C)]
#[allow(dead_code)]
struct SystemInfo {
    oem_id_or_arch: u32,
    page_size: u32,
    min_app_addr: *mut c_void,
    max_app_addr: *mut c_void,
    active_processor_mask: usize,
    number_of_processors: u32,
    processor_type: u32,
    allocation_granularity: u32,
    processor_level: u16,
    processor_revision: u16,
}

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
    fn FlushViewOfFile(
        lp_base_address: *const c_void,
        dw_number_of_bytes_to_flush: usize,
    ) -> i32;
    fn UnmapViewOfFile(lp_base_address: *const c_void) -> i32;
    fn CloseHandle(hobject: Handle) -> i32;
    fn GetSystemInfo(lp_system_info: *mut SystemInfo);
    fn VirtualAlloc(
        lp_address: *mut c_void,
        dw_size: usize,
        fl_allocation_type: u32,
        fl_protect: u32,
    ) -> *mut c_void;
    fn VirtualFree(lp_address: *mut c_void, dw_size: usize, dw_free_type: u32) -> i32;
}

pub fn page_size() -> usize {
    let mut info = SystemInfo {
        oem_id_or_arch: 0,
        page_size: 4096,
        min_app_addr: ptr::null_mut(),
        max_app_addr: ptr::null_mut(),
        active_processor_mask: 0,
        number_of_processors: 0,
        processor_type: 0,
        allocation_granularity: 0,
        processor_level: 0,
        processor_revision: 0,
    };
    // SAFETY: `info` is a valid writable `SYSTEM_INFO` struct.
    unsafe { GetSystemInfo(&raw mut info) };
    if info.page_size > 0 {
        info.page_size as usize
    } else {
        4096
    }
}

pub struct ReadOnlyMmap {
    ptr: *const u8,
    len: usize,
    mapping: Handle,
}

// SAFETY: Mapping is read-only (`PAGE_READONLY` / `FILE_MAP_READ`) and immutable until Drop.
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
    pub fn as_slice(&self) -> &[u8] {
        if self.len == 0 {
            &[]
        } else {
            // SAFETY: Valid read-only view of `self.len` bytes until Drop.
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

enum RwBacking {
    Mapped {
        ptr: *mut u8,
        len: usize,
        mapping: Handle,
        file: File,
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
        file.set_len(len as u64)?;
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
        let len_u64 = len as u64;
        let max_high = (len_u64 >> 32) as u32;
        let max_low = (len_u64 & 0xFFFF_FFFF) as u32;
        // SAFETY: Valid read/write file handle sized to `len`.
        let mapping = unsafe {
            CreateFileMappingW(
                file.as_raw_handle() as Handle,
                ptr::null(),
                PAGE_READWRITE,
                max_high,
                max_low,
                ptr::null(),
            )
        };
        if mapping.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: Valid file mapping handle, writable view of `len` bytes.
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, len) };
        if view.is_null() {
            let err = io::Error::last_os_error();
            unsafe {
                let _ = CloseHandle(mapping);
            }
            return Err(err);
        }
        Ok(Self {
            backing: RwBacking::Mapped {
                ptr: view as *mut u8,
                len,
                mapping,
                file,
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
            io::Error::new(io::ErrorKind::InvalidInput, "sync range overflow")
        })?;
        if end > total {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sync range exceeds mapping length",
            ));
        }
        match &self.backing {
            RwBacking::Mapped { ptr, file, .. } => {
                // SAFETY: `offset + len <= total` within live mapped view.
                let ok = unsafe { FlushViewOfFile(ptr.add(offset) as *const c_void, len) };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                file.sync_data()
            }
            RwBacking::InMemory(_) => Ok(()),
        }
    }
}

impl Drop for ReadWriteMmap {
    fn drop(&mut self) {
        if let RwBacking::Mapped {
            ptr, len, mapping, ..
        } = &self.backing
        {
            if *len > 0 && !ptr.is_null() {
                unsafe {
                    let _ = UnmapViewOfFile(*ptr as *const c_void);
                }
            }
            if !mapping.is_null() {
                unsafe {
                    let _ = CloseHandle(*mapping);
                }
            }
        }
    }
}

pub struct AnonReservation {
    ptr: *mut c_void,
}

impl AnonReservation {
    pub fn new(len: usize) -> io::Result<Self> {
        let alloc_len = len.max(page_size());
        // SAFETY: Anonymous virtual address reservation.
        let ptr = unsafe { VirtualAlloc(ptr::null_mut(), alloc_len, MEM_RESERVE, PAGE_NOACCESS) };
        if ptr.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { ptr })
    }

    pub fn base_addr(&self) -> usize {
        self.ptr as usize
    }
}

impl Drop for AnonReservation {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `self.ptr` came from `VirtualAlloc`.
            unsafe {
                let _ = VirtualFree(self.ptr, 0, MEM_RELEASE);
            }
        }
    }
}
