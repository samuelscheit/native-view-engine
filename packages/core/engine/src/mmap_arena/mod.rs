//! Production file-backed `mmap` 32-bit Offset-Arena ([`MmapOffsetArena`]).
//!
//! Features:
//! - Address-independent 32-bit base-relative handles ([`OffsetPtr<T>`], [`OffsetSlice<T>`],
//!   and [`TrustedUtf8Str`] / [`OffsetStr`]) safe across ASLR and distinct virtual mappings.
//! - Crash-atomic double-buffered root header slots (Slots A & B) protected by **XXH64**
//!   header checksums and monotonic `generation` counters.
//! - O(1) hot-path `&str` reads via [`TrustedUtf8Str`] with zero per-read UTF-8 re-scanning
//!   (validated once on write, plus explicit untrusted open/verification paths).
//! - Online compaction (`compact_to_path` / `compact_in_place`) reclaiming dead string slack
//!   space via tight copy + atomic rename.
//! - Zero external crates, POSIX `mmap` on Unix, Win32 `CreateFileMappingW` on Windows, and
//!   in-memory fallback on `wasm32`.

mod arena;
pub mod header;
pub mod os_mmap;
#[cfg(test)]
mod tests;

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};

pub use arena::MmapOffsetArena;
pub use header::{
    compute_slot_checksum, ArenaHeaderSlot, ARENA_DATA_START, ARENA_FLAG_UTF8_TRUSTED,
    ARENA_HEADER_REGION_SIZE, ARENA_HEADER_SLOT_SIZE, ARENA_MAGIC, ARENA_VERSION,
};
pub use os_mmap::{
    page_size, read_file_bytes, write_file_bytes, AnonReservation, ReadOnlyMmap, ReadWriteMmap,
};

/// Errors produced by [`MmapOffsetArena`] creation, opening, validation, access, or compaction.
#[derive(Debug)]
pub enum ArenaError {
    Io(io::Error),
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u32),
    HeaderChecksumMismatch { expected: u64, actual: u64 },
    CorruptHeader(&'static str),
    NullPointer,
    Misaligned { offset: usize, align: usize },
    OutOfBounds { offset: usize, len: usize, limit: usize },
    OutOfCapacity { needed: usize, capacity: usize },
    InvalidUtf8,
    Overflow,
    NotFileBacked,
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Truncated => write!(f, "truncated MmapOffsetArena file"),
            Self::InvalidMagic => write!(f, "invalid NVEARENA magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVEARENA version {v}"),
            Self::HeaderChecksumMismatch { expected, actual } => {
                write!(
                    f,
                    "arena header XXH64 checksum mismatch (expected={expected:#018x}, actual={actual:#018x})"
                )
            }
            Self::CorruptHeader(msg) => write!(f, "corrupt arena header: {msg}"),
            Self::NullPointer => write!(f, "null OffsetPtr dereference"),
            Self::Misaligned { offset, align } => {
                write!(f, "misaligned offset {offset:#x} for align {align}")
            }
            Self::OutOfBounds { offset, len, limit } => {
                write!(
                    f,
                    "out of bounds: {offset}..{} > {limit}",
                    offset.saturating_add(*len)
                )
            }
            Self::OutOfCapacity { needed, capacity } => {
                write!(
                    f,
                    "arena out of capacity (needed {needed}, capacity {capacity})"
                )
            }
            Self::InvalidUtf8 => write!(f, "invalid utf-8 in arena string"),
            Self::Overflow => write!(f, "arena offset/size overflow"),
            Self::NotFileBacked => {
                write!(f, "operation requires a file-backed MmapOffsetArena")
            }
        }
    }
}

impl std::error::Error for ArenaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ArenaError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Marker trait for verified `#[repr(C)]`, zero-padding POD types safe to store directly
/// inside [`MmapOffsetArena`].
///
/// # Safety
/// Implementing types must be `Copy + 'static`, `#[repr(C)]`, contain no raw pointers or invalid
/// bit patterns (e.g. `bool` or `NonZero*`), and have zero uninitialized padding bytes.
pub unsafe trait ArenaPod: Copy + 'static {}

// SAFETY: `u8` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for u8 {}
// SAFETY: `i8` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for i8 {}
// SAFETY: `u16` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for u16 {}
// SAFETY: `i16` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for i16 {}
// SAFETY: `u32` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for u32 {}
// SAFETY: `i32` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for i32 {}
// SAFETY: `u64` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for u64 {}
// SAFETY: `i64` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for i64 {}
// SAFETY: `f32` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for f32 {}
// SAFETY: `f64` is a Copy + 'static primitive with no padding or invalid bit patterns.
unsafe impl ArenaPod for f64 {}
// SAFETY: `ArenaHeaderSlot` is `#[repr(C)]`, 64 bytes, zero padding, all bit patterns valid.
unsafe impl ArenaPod for ArenaHeaderSlot {}
// SAFETY: `TrustedUtf8Str` is `#[repr(C)]`, zero padding, two `u32` offsets.
unsafe impl ArenaPod for TrustedUtf8Str {}
// SAFETY: `OffsetPtr<T>` is `#[repr(C)]` holding a `u32` offset and `PhantomData`.
unsafe impl<T: 'static> ArenaPod for OffsetPtr<T> {}
// SAFETY: `OffsetSlice<T>` is `#[repr(C)]` holding two `u32` fields and `PhantomData`.
unsafe impl<T: 'static> ArenaPod for OffsetSlice<T> {}

/// 32-bit base-relative pointer inside [`MmapOffsetArena`]. Offset `0` represents null.
#[repr(C)]
pub struct OffsetPtr<T> {
    pub offset: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for OffsetPtr<T> {}
impl<T> Clone for OffsetPtr<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for OffsetPtr<T> {
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        self.offset == o.offset
    }
}
impl<T> Eq for OffsetPtr<T> {}
impl<T> Default for OffsetPtr<T> {
    #[inline]
    fn default() -> Self {
        Self::NULL
    }
}
impl<T> fmt::Debug for OffsetPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OffsetPtr").field(&self.offset).finish()
    }
}

impl<T> OffsetPtr<T> {
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

/// 32-bit base-relative contiguous slice `(offset, len)` inside [`MmapOffsetArena`].
#[repr(C)]
pub struct OffsetSlice<T> {
    pub offset: u32,
    pub len: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for OffsetSlice<T> {}
impl<T> Clone for OffsetSlice<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for OffsetSlice<T> {
    #[inline]
    fn eq(&self, o: &Self) -> bool {
        self.offset == o.offset && self.len == o.len
    }
}
impl<T> Eq for OffsetSlice<T> {}
impl<T> Default for OffsetSlice<T> {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}
impl<T> fmt::Debug for OffsetSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OffsetSlice")
            .field("offset", &self.offset)
            .field("len", &self.len)
            .finish()
    }
}

impl<T> OffsetSlice<T> {
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

/// 8-byte base-relative UTF-8 string handle inside [`MmapOffsetArena`].
///
/// UTF-8 validity is guaranteed once on write ([`MmapOffsetArena::alloc_str`]) and verified on
/// untrusted opens ([`MmapOffsetArena::open_untrusted`] / [`MmapOffsetArena::open_and_verify`]),
/// allowing O(1) hot-path `&str` reads via [`MmapOffsetArena::str_at`] with zero per-read UTF-8
/// re-scanning.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrustedUtf8Str {
    pub offset: u32,
    pub len: u32,
}

impl TrustedUtf8Str {
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

    #[inline]
    pub const fn as_byte_slice(self) -> OffsetSlice<u8> {
        OffsetSlice::new(self.offset, self.len)
    }
}

/// Ergonomic alias for [`TrustedUtf8Str`].
pub type OffsetStr = TrustedUtf8Str;

const _: () = {
    assert!(size_of::<OffsetPtr<u64>>() == 4);
    assert!(align_of::<OffsetPtr<u64>>() == 4);
    assert!(offset_of!(OffsetPtr<u64>, offset) == 0);

    assert!(size_of::<OffsetSlice<u64>>() == 8);
    assert!(align_of::<OffsetSlice<u64>>() == 4);
    assert!(offset_of!(OffsetSlice<u64>, offset) == 0);
    assert!(offset_of!(OffsetSlice<u64>, len) == 4);

    assert!(size_of::<TrustedUtf8Str>() == 8);
    assert!(align_of::<TrustedUtf8Str>() == 4);
    assert!(offset_of!(TrustedUtf8Str, offset) == 0);
    assert!(offset_of!(TrustedUtf8Str, len) == 4);
};
