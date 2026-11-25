//! Workload A Candidate 1 (`NVERELOC` / `RelocSlice<T>` aligned zero-copy buffer).
//!
//! Stores a flat `#[repr(C)]` relocatable layout directly readable from `mmap` with zero
//! allocations and zero deserialization:
//! - [`RelocHeader`] (48 bytes, 8-byte aligned, XXH64 checksum)
//! - [`RelocSlice<TaskPod>`] (contiguous 64-byte cache-line-aligned `TaskPod` slab)
//! - UTF-8 string pool addressed via [`RelocStr`] `(offset, len)` pairs
//!
//! Supports three explicit cold-load validation modes:
//! - [`VerifyMode::Full`]: Untrusted validation (header + full XXH64 checksum + every
//!   `TaskPod`'s `RelocStr` bounds and UTF-8 validity).
//! - [`VerifyMode::HeaderOnly`]: Trusted local `mmap` O(1) header + section bounds/alignment check
//!   without faulting payload pages.
//! - [`VerifyMode::Unchecked`]: Zero-check pointer cast (`unsafe` precondition: valid buffer).

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};
use std::path::Path;

use nve_engine::hash::Xxh64Hasher;
use nve_engine::mmap_arena::{write_file_bytes, ReadOnlyMmap};

use super::{fold_task_fields, OwnedTask};

pub const RELOC_MAGIC: [u8; 8] = *b"NVERELOC";
pub const RELOC_VERSION: u32 = 1;

/// Validation tier for cold-loading an `NVERELOC` zero-copy buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// Untrusted validation: header, alignment, XXH64 checksum, and every string slice's UTF-8.
    Full,
    /// Trusted local `mmap`: O(1) header, alignment, and section bounds check only.
    HeaderOnly,
    /// Zero-check direct pointer cast (caller upholds layout & UTF-8 invariants).
    Unchecked,
}

#[derive(Debug)]
pub enum RelocError {
    Io(io::Error),
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u32),
    Misaligned { expected: usize, actual: usize },
    TotalBytesMismatch { header: u64, actual: usize },
    ChecksumMismatch { expected: u64, actual: u64 },
    OutOfBounds(&'static str),
    InvalidUtf8,
    Overflow,
}

impl fmt::Display for RelocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Truncated => write!(f, "truncated NVERELOC buffer"),
            Self::InvalidMagic => write!(f, "invalid NVERELOC magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVERELOC version {v}"),
            Self::Misaligned { expected, actual } => {
                write!(f, "misaligned NVERELOC buffer (need {expected}, got {actual:#x})")
            }
            Self::TotalBytesMismatch { header, actual } => {
                write!(f, "total_bytes mismatch (header={header}, actual={actual})")
            }
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "checksum mismatch (expected={expected:#x}, actual={actual:#x})")
            }
            Self::OutOfBounds(what) => write!(f, "NVERELOC out of bounds: {what}"),
            Self::InvalidUtf8 => write!(f, "invalid utf-8 in NVERELOC string pool"),
            Self::Overflow => write!(f, "NVERELOC offset/size overflow"),
        }
    }
}

impl std::error::Error for RelocError {}

impl From<io::Error> for RelocError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Base-relative `(offset, len)` descriptor for a contiguous slice of `T` inside an `NVERELOC` blob.
#[repr(C)]
pub struct RelocSlice<T> {
    pub offset: u32,
    pub len: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for RelocSlice<T> {}
impl<T> Clone for RelocSlice<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for RelocSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelocSlice")
            .field("offset", &self.offset)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> RelocSlice<T> {
    pub const fn new(offset: u32, len: u32) -> Self {
        Self {
            offset,
            len,
            _marker: PhantomData,
        }
    }
}

/// String-pool-relative `(offset, len)` UTF-8 string handle.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocStr {
    pub offset: u32,
    pub len: u32,
}

/// Fixed-size, 8-byte-aligned, zero-padding POD representation of a Studio Workspace task (64 B).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskPod {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: RelocStr,
    pub body: RelocStr,
    pub tag: RelocStr,
}

/// 48-byte, 8-byte-aligned header at byte offset 0 of every `NVERELOC` workspace blob.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RelocHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub flags: u32,
    pub total_bytes: u64,
    pub checksum_xxh64: u64,
    pub tasks: RelocSlice<TaskPod>,
    pub string_pool_offset: u32,
    pub string_pool_len: u32,
}

const _: () = {
    assert!(size_of::<RelocStr>() == 8);
    assert!(align_of::<RelocStr>() == 4);
    assert!(size_of::<RelocSlice<TaskPod>>() == 8);
    assert!(align_of::<RelocSlice<TaskPod>>() == 4);

    assert!(size_of::<TaskPod>() == 64);
    assert!(align_of::<TaskPod>() == 8);
    assert!(offset_of!(TaskPod, id) == 0);
    assert!(offset_of!(TaskPod, created_at_ms) == 8);
    assert!(offset_of!(TaskPod, updated_at_ms) == 16);
    assert!(offset_of!(TaskPod, priority) == 24);
    assert!(offset_of!(TaskPod, status) == 28);
    assert!(offset_of!(TaskPod, assignee_id) == 32);
    assert!(offset_of!(TaskPod, flags) == 36);
    assert!(offset_of!(TaskPod, title) == 40);
    assert!(offset_of!(TaskPod, body) == 48);
    assert!(offset_of!(TaskPod, tag) == 56);

    assert!(size_of::<RelocHeader>() == 48);
    assert!(align_of::<RelocHeader>() == 8);
    assert!(offset_of!(RelocHeader, magic) == 0);
    assert!(offset_of!(RelocHeader, version) == 8);
    assert!(offset_of!(RelocHeader, flags) == 12);
    assert!(offset_of!(RelocHeader, total_bytes) == 16);
    assert!(offset_of!(RelocHeader, checksum_xxh64) == 24);
    assert!(offset_of!(RelocHeader, tasks) == 32);
    assert!(offset_of!(RelocHeader, string_pool_offset) == 40);
    assert!(offset_of!(RelocHeader, string_pool_len) == 44);
};

#[inline]
fn align_up(value: usize, align: usize) -> Result<usize, RelocError> {
    let rem = value % align;
    if rem == 0 {
        Ok(value)
    } else {
        value.checked_add(align - rem).ok_or(RelocError::Overflow)
    }
}

#[inline]
pub(crate) fn compute_reloc_checksum(bytes: &[u8]) -> u64 {
    // Hash header[0..24] + header[32..48] + body[48..] (skipping checksum_xxh64 at 24..32).
    let mut h = Xxh64Hasher::new(0);
    h.update(&bytes[0..24]);
    h.update(&bytes[32..]);
    h.finish()
}

/// Builds an 8-byte aligned `NVERELOC` binary buffer from owned Studio Workspace tasks.
pub fn build_reloc_workspace(tasks: &[OwnedTask]) -> Result<Vec<u8>, RelocError> {
    let header_size = size_of::<RelocHeader>();
    let tasks_offset = align_up(header_size, align_of::<TaskPod>())?;
    let tasks_bytes = tasks
        .len()
        .checked_mul(size_of::<TaskPod>())
        .ok_or(RelocError::Overflow)?;
    let string_pool_offset = tasks_offset
        .checked_add(tasks_bytes)
        .ok_or(RelocError::Overflow)?;

    let mut string_pool_len = 0usize;
    for t in tasks {
        string_pool_len = string_pool_len
            .checked_add(t.title.len() + t.body.len() + t.tag.len())
            .ok_or(RelocError::Overflow)?;
    }

    let unpadded_total = string_pool_offset
        .checked_add(string_pool_len)
        .ok_or(RelocError::Overflow)?;
    let total_bytes = align_up(unpadded_total, align_of::<RelocHeader>())?;

    let tasks_offset_u32 = u32::try_from(tasks_offset).map_err(|_| RelocError::Overflow)?;
    let tasks_len_u32 = u32::try_from(tasks.len()).map_err(|_| RelocError::Overflow)?;
    let pool_offset_u32 = u32::try_from(string_pool_offset).map_err(|_| RelocError::Overflow)?;
    let pool_len_u32 = u32::try_from(string_pool_len).map_err(|_| RelocError::Overflow)?;

    let mut buf = vec![0u8; total_bytes];

    // Write `TaskPod` array and contiguous UTF-8 string pool in a single pass.
    let mut str_cursor = 0usize;
    for (i, t) in tasks.iter().enumerate() {
        let title_off = u32::try_from(str_cursor).map_err(|_| RelocError::Overflow)?;
        let title_len = u32::try_from(t.title.len()).map_err(|_| RelocError::Overflow)?;
        let abs_title = string_pool_offset + str_cursor;
        buf[abs_title..abs_title + t.title.len()].copy_from_slice(t.title.as_bytes());
        str_cursor += t.title.len();

        let body_off = u32::try_from(str_cursor).map_err(|_| RelocError::Overflow)?;
        let body_len = u32::try_from(t.body.len()).map_err(|_| RelocError::Overflow)?;
        let abs_body = string_pool_offset + str_cursor;
        buf[abs_body..abs_body + t.body.len()].copy_from_slice(t.body.as_bytes());
        str_cursor += t.body.len();

        let tag_off = u32::try_from(str_cursor).map_err(|_| RelocError::Overflow)?;
        let tag_len = u32::try_from(t.tag.len()).map_err(|_| RelocError::Overflow)?;
        let abs_tag = string_pool_offset + str_cursor;
        buf[abs_tag..abs_tag + t.tag.len()].copy_from_slice(t.tag.as_bytes());
        str_cursor += t.tag.len();

        let pod = TaskPod {
            id: t.id,
            created_at_ms: t.created_at_ms,
            updated_at_ms: t.updated_at_ms,
            priority: t.priority,
            status: t.status,
            assignee_id: t.assignee_id,
            flags: t.flags,
            title: RelocStr {
                offset: title_off,
                len: title_len,
            },
            body: RelocStr {
                offset: body_off,
                len: body_len,
            },
            tag: RelocStr {
                offset: tag_off,
                len: tag_len,
            },
        };
        let dst_off = tasks_offset + i * size_of::<TaskPod>();
        // SAFETY: `TaskPod` is `#[repr(C)]` with zero padding bytes; copying its byte
        // representation into `buf[dst_off..dst_off + 64]` is valid and alignment-independent.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&raw const pod).cast::<u8>(),
                buf.as_mut_ptr().add(dst_off),
                size_of::<TaskPod>(),
            );
        }
    }

    let mut header = RelocHeader {
        magic: RELOC_MAGIC,
        version: RELOC_VERSION,
        flags: 0,
        total_bytes: total_bytes as u64,
        checksum_xxh64: 0,
        tasks: RelocSlice::new(tasks_offset_u32, tasks_len_u32),
        string_pool_offset: pool_offset_u32,
        string_pool_len: pool_len_u32,
    };

    // SAFETY: `RelocHeader` is `#[repr(C)]` with zero padding bytes and `buf.len() >= 48`.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const header).cast::<u8>(),
            buf.as_mut_ptr(),
            size_of::<RelocHeader>(),
        );
    }

    header.checksum_xxh64 = compute_reloc_checksum(&buf);
    buf[24..32].copy_from_slice(&header.checksum_xxh64.to_le_bytes());
    Ok(buf)
}

/// Builds and writes an `NVERELOC` workspace blob to `path`.
pub fn write_reloc_workspace(path: &Path, tasks: &[OwnedTask]) -> Result<usize, RelocError> {
    let bytes = build_reloc_workspace(tasks)?;
    write_file_bytes(path, &bytes)?;
    Ok(bytes.len())
}

/// Zero-copy borrowed view directly over an `NVERELOC` byte buffer (typically from `mmap`).
#[derive(Clone, Copy)]
pub struct RelocWorkspaceView<'a> {
    header: &'a RelocHeader,
    tasks: &'a [TaskPod],
    string_pool: &'a [u8],
    utf8_verified: bool,
}

impl<'a> RelocWorkspaceView<'a> {
    /// Validates and opens a zero-copy view according to [`VerifyMode::Full`] or
    /// [`VerifyMode::HeaderOnly`]. For [`VerifyMode::Unchecked`], call
    /// [`Self::from_bytes_unchecked`].
    pub fn from_bytes(bytes: &'a [u8], mode: VerifyMode) -> Result<Self, RelocError> {
        match mode {
            VerifyMode::Unchecked => {
                // Keep safe API strictly sound: route `Unchecked` through HeaderOnly if invoked
                // via the safe entrypoint; benchmarks call `from_bytes_unchecked` directly.
                Self::from_bytes_header_only(bytes)
            }
            VerifyMode::HeaderOnly => Self::from_bytes_header_only(bytes),
            VerifyMode::Full => {
                let mut view = Self::from_bytes_header_only(bytes)?;
                let actual = compute_reloc_checksum(bytes);
                if actual != view.header.checksum_xxh64 {
                    return Err(RelocError::ChecksumMismatch {
                        expected: view.header.checksum_xxh64,
                        actual,
                    });
                }
                for pod in view.tasks {
                    validate_reloc_str(view.string_pool, pod.title)?;
                    validate_reloc_str(view.string_pool, pod.body)?;
                    validate_reloc_str(view.string_pool, pod.tag)?;
                }
                view.utf8_verified = true;
                Ok(view)
            }
        }
    }

    fn from_bytes_header_only(bytes: &'a [u8]) -> Result<Self, RelocError> {
        if bytes.len() < size_of::<RelocHeader>() {
            return Err(RelocError::Truncated);
        }
        let base_addr = bytes.as_ptr() as usize;
        if !base_addr.is_multiple_of(align_of::<RelocHeader>()) {
            return Err(RelocError::Misaligned {
                expected: align_of::<RelocHeader>(),
                actual: base_addr,
            });
        }
        // SAFETY: `bytes.len() >= size_of::<RelocHeader>()` and pointer is 8-byte aligned.
        let header = unsafe { &*bytes.as_ptr().cast::<RelocHeader>() };
        if header.magic != RELOC_MAGIC {
            return Err(RelocError::InvalidMagic);
        }
        if header.version != RELOC_VERSION {
            return Err(RelocError::UnsupportedVersion(header.version));
        }
        if header.total_bytes as usize != bytes.len() {
            return Err(RelocError::TotalBytesMismatch {
                header: header.total_bytes,
                actual: bytes.len(),
            });
        }

        let tasks_off = header.tasks.offset as usize;
        let tasks_len = header.tasks.len as usize;
        if !tasks_off.is_multiple_of(align_of::<TaskPod>()) {
            return Err(RelocError::Misaligned {
                expected: align_of::<TaskPod>(),
                actual: tasks_off,
            });
        }
        let tasks_bytes = tasks_len
            .checked_mul(size_of::<TaskPod>())
            .ok_or(RelocError::Overflow)?;
        let tasks_end = tasks_off
            .checked_add(tasks_bytes)
            .ok_or(RelocError::Overflow)?;
        if tasks_end > bytes.len() {
            return Err(RelocError::OutOfBounds("tasks slice"));
        }

        let pool_off = header.string_pool_offset as usize;
        let pool_len = header.string_pool_len as usize;
        let pool_end = pool_off.checked_add(pool_len).ok_or(RelocError::Overflow)?;
        if pool_end > bytes.len() {
            return Err(RelocError::OutOfBounds("string pool"));
        }

        // SAFETY: `base_addr` and `tasks_off` are both 8-byte aligned, and
        // `tasks_off + tasks_len * size_of::<TaskPod>() <= bytes.len()`.
        let tasks = unsafe {
            std::slice::from_raw_parts(bytes.as_ptr().add(tasks_off).cast::<TaskPod>(), tasks_len)
        };
        let string_pool = &bytes[pool_off..pool_end];

        Ok(Self {
            header,
            tasks,
            string_pool,
            utf8_verified: false,
        })
    }

    /// Zero-check pointer cast directly onto an `NVERELOC` buffer.
    ///
    /// # Safety
    /// Caller guarantees `bytes` is an 8-byte aligned, valid `NVERELOC` buffer whose offsets,
    /// lengths, and UTF-8 string pool are well-formed.
    #[inline]
    pub unsafe fn from_bytes_unchecked(bytes: &'a [u8]) -> Self {
        let base = bytes.as_ptr();
        // SAFETY: Upheld by caller.
        let header = unsafe { &*base.cast::<RelocHeader>() };
        let tasks = unsafe {
            std::slice::from_raw_parts(
                base.add(header.tasks.offset as usize).cast::<TaskPod>(),
                header.tasks.len as usize,
            )
        };
        let string_pool = unsafe {
            std::slice::from_raw_parts(
                base.add(header.string_pool_offset as usize),
                header.string_pool_len as usize,
            )
        };
        Self {
            header,
            tasks,
            string_pool,
            utf8_verified: true,
        }
    }

    #[inline]
    pub fn header(&self) -> &'a RelocHeader {
        self.header
    }

    #[inline]
    pub fn tasks(&self) -> &'a [TaskPod] {
        self.tasks
    }

    /// Resolves a [`RelocStr`] into a borrowed `&str` with bounds + UTF-8 check (unless already
    /// validated by [`VerifyMode::Full`] or [`Self::from_bytes_unchecked`]).
    #[inline]
    pub fn str_at(&self, s: RelocStr) -> Result<&'a str, RelocError> {
        let start = s.offset as usize;
        let end = start
            .checked_add(s.len as usize)
            .ok_or(RelocError::Overflow)?;
        let slice = self
            .string_pool
            .get(start..end)
            .ok_or(RelocError::OutOfBounds("reloc str"))?;
        if self.utf8_verified {
            // SAFETY: Validated during `VerifyMode::Full` or upheld by `from_bytes_unchecked`.
            Ok(unsafe { std::str::from_utf8_unchecked(slice) })
        } else {
            std::str::from_utf8(slice).map_err(|_| RelocError::InvalidUtf8)
        }
    }

    /// Performs the apples-to-apples random-read workload directly out of the mapped buffer.
    pub fn random_read_checksum(&self, indices: &[usize]) -> Result<u64, RelocError> {
        let mut acc = 0u64;
        for &idx in indices {
            let pod = &self.tasks[idx];
            let title = self.str_at(pod.title)?;
            let tag = self.str_at(pod.tag)?;
            acc = fold_task_fields(
                acc,
                pod.id,
                pod.updated_at_ms,
                pod.priority,
                pod.status,
                title,
                tag,
            );
        }
        Ok(acc)
    }
}

#[inline]
fn validate_reloc_str(pool: &[u8], s: RelocStr) -> Result<&str, RelocError> {
    let start = s.offset as usize;
    let end = start
        .checked_add(s.len as usize)
        .ok_or(RelocError::Overflow)?;
    let slice = pool
        .get(start..end)
        .ok_or(RelocError::OutOfBounds("reloc str"))?;
    std::str::from_utf8(slice).map_err(|_| RelocError::InvalidUtf8)
}

/// Owned `mmap`-backed `NVERELOC` workspace handle.
pub struct MmapRelocWorkspace {
    mmap: ReadOnlyMmap,
    mode: VerifyMode,
}

impl MmapRelocWorkspace {
    /// Opens and maps an `NVERELOC` file with [`VerifyMode::Full`] or [`VerifyMode::HeaderOnly`].
    pub fn open(path: &Path, mode: VerifyMode) -> Result<Self, RelocError> {
        let mmap = ReadOnlyMmap::open(path)?;
        let effective = match mode {
            VerifyMode::Unchecked => VerifyMode::HeaderOnly,
            other => other,
        };
        let _ = RelocWorkspaceView::from_bytes(mmap.as_slice(), effective)?;
        Ok(Self {
            mmap,
            mode: effective,
        })
    }

    /// Opens and maps an `NVERELOC` file with zero-check pointer casting (`VerifyMode::Unchecked`).
    ///
    /// # Safety
    /// File contents must be a valid `NVERELOC` workspace blob produced by [`build_reloc_workspace`].
    pub unsafe fn open_unchecked(path: &Path) -> Result<Self, RelocError> {
        let mmap = ReadOnlyMmap::open(path)?;
        Ok(Self {
            mmap,
            mode: VerifyMode::Unchecked,
        })
    }

    #[inline]
    pub fn view(&self) -> RelocWorkspaceView<'_> {
        match self.mode {
            VerifyMode::Unchecked | VerifyMode::Full => {
                // SAFETY: `Full` validated checksum + all slices/UTF-8 on open;
                // `Unchecked` was upheld by `open_unchecked`.
                unsafe { RelocWorkspaceView::from_bytes_unchecked(self.mmap.as_slice()) }
            }
            VerifyMode::HeaderOnly => {
                // Header & section bounds were already validated in `open`.
                let mut v =
                    unsafe { RelocWorkspaceView::from_bytes_unchecked(self.mmap.as_slice()) };
                v.utf8_verified = false;
                v
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mmap.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.mmap.len() == 0
    }
}
