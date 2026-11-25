//! Core `NVERELOC` binary container primitives: 64-byte [`RelocHeader`], typed handles
//! ([`RelocPtr<T>`], [`RelocSlice<T>`], [`RelocStr`]), 64-byte-aligned [`RelocBuf`],
//! [`RelocWriter`], and [`RelocBundleRoot`].

use std::fmt;
use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};
use std::ops::Deref;
use std::path::Path;

use crate::hash::Xxh64Hasher;
use crate::mmap_arena::ArenaPod;

use super::view::{write_reloc_atomic, RelocView};
use super::{RelocError, RelocPod, VerifyMode};

/// 8-byte ASCII magic identifying every `NVERELOC` binary container.
pub const RELOC_MAGIC: [u8; 8] = *b"NVERELOC";

/// Current `NVERELOC` container format version.
pub const RELOC_VERSION: u32 = 1;

/// Fixed byte length of [`RelocHeader`] (exact 64-byte CPU cache line).
pub const RELOC_HEADER_SIZE: usize = 64;

/// Guaranteed base alignment of [`RelocBuf`] (64-byte cache line).
pub const RELOC_BUF_ALIGN: usize = 64;

/// `RelocHeader::content_kind` for generic/custom relocatable payloads.
pub const RELOC_KIND_GENERIC: u32 = 0;
/// `RelocHeader::content_kind` for pre-rendered Frame-0 [`super::SceneRelocView`] blobs.
pub const RELOC_KIND_SCENE: u32 = 1;
/// `RelocHeader::content_kind` for [`RelocBundleRoot`] Frame-0 + state + metadata bundles.
pub const RELOC_KIND_BUNDLE: u32 = 2;
/// `RelocHeader::content_kind` for relocatable workspace record tables.
pub const RELOC_KIND_WORKSPACE: u32 = 3;
/// `RelocHeader::content_kind` for [`super::NveTimelineArchiveView`] timed scene patch timelines.
pub const RELOC_KIND_TIMELINE: u32 = 4;

pub(crate) const CHECKSUM_OFFSET: usize = 24;
pub(crate) const CHECKSUM_END: usize = 32;

#[inline]
pub(crate) fn align_up(value: usize, align: usize) -> Result<usize, RelocError> {
    if align == 0 || !align.is_power_of_two() {
        return Err(RelocError::Overflow);
    }
    let rem = value & (align - 1);
    if rem == 0 {
        Ok(value)
    } else {
        value.checked_add(align - rem).ok_or(RelocError::Overflow)
    }
}

/// 64-byte cache-line header at byte offset `0` of every `NVERELOC` container.
///
/// All fields are fixed-width little-endian with zero implicit padding bytes.
/// The XXH64 checksum covers `bytes[0..24]` and `bytes[32..64 + payload_len]`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub flags: u32,
    pub payload_len: u32,
    pub root_offset: u32,
    pub checksum: u64,
    pub schema_version: u32,
    pub content_kind: u32,
    pub reserved: [u64; 3],
}

/// 32-bit buffer-base-relative pointer inside an `NVERELOC` container (`0` = null).
#[repr(C)]
pub struct RelocPtr<T> {
    pub offset: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for RelocPtr<T> {}
impl<T> Clone for RelocPtr<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for RelocPtr<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset
    }
}
impl<T> Eq for RelocPtr<T> {}
impl<T> Default for RelocPtr<T> {
    #[inline]
    fn default() -> Self {
        Self::NULL
    }
}
impl<T> fmt::Debug for RelocPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RelocPtr").field(&self.offset).finish()
    }
}

impl<T> RelocPtr<T> {
    pub const NULL: Self = Self {
        offset: 0,
        _marker: PhantomData,
    };

    #[inline]
    pub const fn new(offset: u32) -> Self {
        Self {
            offset,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub const fn is_null(self) -> bool {
        self.offset == 0
    }
}

/// 8-byte buffer-base-relative contiguous slice `(offset, len)` inside an `NVERELOC` container.
#[repr(C)]
pub struct RelocSlice<T> {
    pub offset: u32,
    pub len: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for RelocSlice<T> {}
impl<T> Clone for RelocSlice<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for RelocSlice<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.offset == other.offset && self.len == other.len
    }
}
impl<T> Eq for RelocSlice<T> {}
impl<T> Default for RelocSlice<T> {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
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
    pub const EMPTY: Self = Self {
        offset: 0,
        len: 0,
        _marker: PhantomData,
    };

    #[inline]
    pub const fn new(offset: u32, len: u32) -> Self {
        Self {
            offset,
            len,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// 8-byte buffer-base-relative UTF-8 string handle `(offset, len)` inside an `NVERELOC` container.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelocStr {
    pub offset: u32,
    pub len: u32,
}

impl RelocStr {
    pub const EMPTY: Self = Self { offset: 0, len: 0 };

    #[inline]
    pub const fn new(offset: u32, len: u32) -> Self {
        Self { offset, len }
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn len(self) -> usize {
        self.len as usize
    }
}

/// 32-byte root record for an `NVERELOC` Frame-0 + state + metadata bundle (`RELOC_KIND_BUNDLE`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelocBundleRoot {
    pub frame0: RelocSlice<u8>,
    pub state: RelocSlice<u8>,
    pub meta: RelocSlice<u8>,
    pub reserved: u64,
}

// SAFETY: All reloc handles and headers are `#[repr(C)]` with zero uninitialized padding.
unsafe impl ArenaPod for RelocHeader {}
unsafe impl ArenaPod for RelocStr {}
unsafe impl ArenaPod for RelocBundleRoot {}
unsafe impl<T: 'static> ArenaPod for RelocPtr<T> {}
unsafe impl<T: 'static> ArenaPod for RelocSlice<T> {}

const _: () = {
    assert!(size_of::<RelocHeader>() == RELOC_HEADER_SIZE);
    assert!(align_of::<RelocHeader>() == 8);
    assert!(offset_of!(RelocHeader, magic) == 0);
    assert!(offset_of!(RelocHeader, version) == 8);
    assert!(offset_of!(RelocHeader, flags) == 12);
    assert!(offset_of!(RelocHeader, payload_len) == 16);
    assert!(offset_of!(RelocHeader, root_offset) == 20);
    assert!(offset_of!(RelocHeader, checksum) == CHECKSUM_OFFSET);
    assert!(offset_of!(RelocHeader, schema_version) == 32);
    assert!(offset_of!(RelocHeader, content_kind) == 36);
    assert!(offset_of!(RelocHeader, reserved) == 40);

    assert!(size_of::<RelocPtr<u64>>() == 4);
    assert!(align_of::<RelocPtr<u64>>() == 4);
    assert!(size_of::<RelocSlice<u64>>() == 8);
    assert!(align_of::<RelocSlice<u64>>() == 4);
    assert!(size_of::<RelocStr>() == 8);
    assert!(align_of::<RelocStr>() == 4);

    assert!(size_of::<RelocBundleRoot>() == 32);
    assert!(align_of::<RelocBundleRoot>() == 8);
    assert!(offset_of!(RelocBundleRoot, frame0) == 0);
    assert!(offset_of!(RelocBundleRoot, state) == 8);
    assert!(offset_of!(RelocBundleRoot, meta) == 16);
    assert!(offset_of!(RelocBundleRoot, reserved) == 24);
};

/// Computes the canonical XXH64 checksum for an `NVERELOC` buffer (`bytes.len() >= 64`).
///
/// Hashes `bytes[0..24]` followed by `bytes[32..]` (skipping `RelocHeader::checksum`).
#[inline]
pub fn compute_reloc_checksum(bytes: &[u8]) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(&bytes[0..CHECKSUM_OFFSET]);
    h.update(&bytes[CHECKSUM_END..]);
    h.finish()
}

#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Align64Block([u8; RELOC_BUF_ALIGN]);

/// Owned 64-byte-aligned byte buffer for `NVERELOC` containers.
#[derive(Clone)]
pub struct RelocBuf {
    blocks: Vec<Align64Block>,
    len: usize,
}

impl RelocBuf {
    /// Copies `bytes` into a freshly allocated 64-byte-aligned [`RelocBuf`].
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let block_count = bytes.len().div_ceil(RELOC_BUF_ALIGN).max(1);
        let mut blocks = vec![Align64Block([0u8; RELOC_BUF_ALIGN]); block_count];
        let dst = Self::blocks_as_bytes_mut(&mut blocks);
        dst[..bytes.len()].copy_from_slice(bytes);
        Self {
            blocks,
            len: bytes.len(),
        }
    }

    fn blocks_as_bytes(blocks: &[Align64Block]) -> &[u8] {
        // SAFETY: `Align64Block` is `#[repr(C, align(64))]` wrapping `[u8; 64]`.
        unsafe {
            std::slice::from_raw_parts(
                blocks.as_ptr().cast::<u8>(),
                blocks.len() * RELOC_BUF_ALIGN,
            )
        }
    }

    fn blocks_as_bytes_mut(blocks: &mut [Align64Block]) -> &mut [u8] {
        // SAFETY: Exclusive borrow of contiguous `Align64Block` backing storage.
        unsafe {
            std::slice::from_raw_parts_mut(
                blocks.as_mut_ptr().cast::<u8>(),
                blocks.len() * RELOC_BUF_ALIGN,
            )
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &Self::blocks_as_bytes(&self.blocks)[..self.len]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn view(&self, mode: VerifyMode) -> Result<RelocView<'_>, RelocError> {
        RelocView::from_bytes(self.as_slice(), mode)
    }

    /// Crash-safely writes this buffer via sibling temp file + `sync_all()` + atomic rename.
    pub fn write_atomic(&self, path: impl AsRef<Path>) -> Result<(), RelocError> {
        write_reloc_atomic(path, self.as_slice())
    }
}

impl Deref for RelocBuf {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[u8]> for RelocBuf {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl fmt::Debug for RelocBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelocBuf").field("len", &self.len).finish()
    }
}

/// Single-pass builder producing 64-byte-aligned `NVERELOC` containers ([`RelocBuf`]).
pub struct RelocWriter {
    pub(crate) buf: Vec<u8>,
    content_kind: u32,
    schema_version: u32,
    flags: u32,
}

impl RelocWriter {
    pub fn new(content_kind: u32, schema_version: u32, flags: u32) -> Self {
        Self::with_capacity(content_kind, schema_version, flags, 256)
    }

    pub fn with_capacity(
        content_kind: u32,
        schema_version: u32,
        flags: u32,
        payload_capacity: usize,
    ) -> Self {
        let mut buf = Vec::with_capacity(RELOC_HEADER_SIZE.saturating_add(payload_capacity));
        buf.resize(RELOC_HEADER_SIZE, 0);
        Self {
            buf,
            content_kind,
            schema_version,
            flags,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.len() <= RELOC_HEADER_SIZE
    }

    fn pad_to(&mut self, align: usize) -> Result<usize, RelocError> {
        let aligned = align_up(self.buf.len(), align)?;
        if aligned > self.buf.len() {
            self.buf.resize(aligned, 0);
        }
        Ok(aligned)
    }

    /// Appends one `#[repr(C)]` POD value aligned to `align_of::<T>()` and returns its handle.
    pub fn write_val<T: RelocPod>(&mut self, val: &T) -> Result<RelocPtr<T>, RelocError> {
        let off = self.pad_to(align_of::<T>())?;
        let end = off.checked_add(size_of::<T>()).ok_or(RelocError::Overflow)?;
        let _ = u32::try_from(end).map_err(|_| RelocError::Overflow)?;
        self.buf.resize(end, 0);
        // SAFETY: `T: RelocPod` is `#[repr(C)]` with zero padding bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (val as *const T).cast::<u8>(),
                self.buf.as_mut_ptr().add(off),
                size_of::<T>(),
            );
        }
        Ok(RelocPtr::new(off as u32))
    }

    /// Overwrites an already-written `#[repr(C)]` POD value at `ptr`.
    pub(crate) fn patch_val<T: RelocPod>(&mut self, ptr: RelocPtr<T>, val: &T) {
        let off = ptr.offset as usize;
        // SAFETY: `ptr` was returned by `write_val::<T>` on this writer and `T: RelocPod`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (val as *const T).cast::<u8>(),
                self.buf.as_mut_ptr().add(off),
                size_of::<T>(),
            );
        }
    }

    /// Appends a contiguous slice of POD values aligned to `align_of::<T>()`.
    pub fn write_slice<T: RelocPod>(&mut self, items: &[T]) -> Result<RelocSlice<T>, RelocError> {
        if items.is_empty() {
            return Ok(RelocSlice::EMPTY);
        }
        let off = self.pad_to(align_of::<T>())?;
        let byte_len = items
            .len()
            .checked_mul(size_of::<T>())
            .ok_or(RelocError::Overflow)?;
        let end = off.checked_add(byte_len).ok_or(RelocError::Overflow)?;
        let _ = u32::try_from(end).map_err(|_| RelocError::Overflow)?;
        let len_u32 = u32::try_from(items.len()).map_err(|_| RelocError::Overflow)?;
        self.buf.resize(end, 0);
        // SAFETY: `T: RelocPod` is `#[repr(C)]` with zero padding bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                items.as_ptr().cast::<u8>(),
                self.buf.as_mut_ptr().add(off),
                byte_len,
            );
        }
        Ok(RelocSlice::new(off as u32, len_u32))
    }

    /// Appends a raw byte slice aligned to `align` (power of two).
    pub fn write_bytes(&mut self, bytes: &[u8], align: usize) -> Result<RelocSlice<u8>, RelocError> {
        if bytes.is_empty() {
            return Ok(RelocSlice::EMPTY);
        }
        let off = self.pad_to(align.max(1))?;
        let end = off.checked_add(bytes.len()).ok_or(RelocError::Overflow)?;
        let _ = u32::try_from(end).map_err(|_| RelocError::Overflow)?;
        let len_u32 = u32::try_from(bytes.len()).map_err(|_| RelocError::Overflow)?;
        self.buf.extend_from_slice(bytes);
        Ok(RelocSlice::new(off as u32, len_u32))
    }

    /// Appends a UTF-8 string and returns its buffer-relative [`RelocStr`] handle.
    pub fn write_str(&mut self, s: &str) -> Result<RelocStr, RelocError> {
        if s.is_empty() {
            return Ok(RelocStr::EMPTY);
        }
        let slice = self.write_bytes(s.as_bytes(), 1)?;
        Ok(RelocStr::new(slice.offset, slice.len))
    }

    /// Finalizes the container with `root` as its typed root pointer and returns a [`RelocBuf`].
    pub fn finish<T: RelocPod>(self, root: RelocPtr<T>) -> Result<RelocBuf, RelocError> {
        self.finish_raw(root.offset)
    }

    /// Finalizes the container with a raw `root_offset` (`0` if rootless).
    pub fn finish_raw(mut self, root_offset: u32) -> Result<RelocBuf, RelocError> {
        self.pad_to(8)?;
        if root_offset != 0
            && ((root_offset as usize) < RELOC_HEADER_SIZE
                || (root_offset as usize) >= self.buf.len())
        {
            return Err(RelocError::OutOfBounds("root_offset"));
        }
        let payload_len_usize = self.buf.len() - RELOC_HEADER_SIZE;
        let payload_len = u32::try_from(payload_len_usize).map_err(|_| RelocError::Overflow)?;

        let header = RelocHeader {
            magic: RELOC_MAGIC,
            version: RELOC_VERSION,
            flags: self.flags,
            payload_len,
            root_offset,
            checksum: 0,
            schema_version: self.schema_version,
            content_kind: self.content_kind,
            reserved: [0, 0, 0],
        };
        // SAFETY: `RelocHeader` is 64 bytes with zero implicit padding and `buf.len() >= 64`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&raw const header).cast::<u8>(),
                self.buf.as_mut_ptr(),
                RELOC_HEADER_SIZE,
            );
        }
        let checksum = compute_reloc_checksum(&self.buf);
        self.buf[CHECKSUM_OFFSET..CHECKSUM_END].copy_from_slice(&checksum.to_le_bytes());
        Ok(RelocBuf::from_bytes(&self.buf))
    }
}
