//! Workload A Candidate 2: Live file-backed `mmap` 32-bit Offset-Arena (`MmapOffsetArena`).
//!
//! Stores a live `MAP_SHARED` workspace directly in an OS-mapped file using 32-bit base-relative
//! [`OffsetPtr<T>`] and [`OffsetSlice<T>`] handles — zero absolute pointers, zero serialization,
//! zero deserialization on reopen, and nanosecond in-place mutation directly in mapped memory.

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};
use std::path::Path;

use nve_engine::snapshot::fnv1a_64_update;

use super::os_mmap::ReadWriteMmap;
use super::{fold_task_fields, OwnedTask};

pub const ARENA_MAGIC: [u8; 8] = *b"NVEARENA";
pub const ARENA_VERSION: u32 = 1;

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
                    "arena header checksum mismatch (expected={expected:#x}, actual={actual:#x})"
                )
            }
            Self::CorruptHeader(msg) => write!(f, "corrupt arena header: {msg}"),
            Self::NullPointer => write!(f, "null OffsetPtr dereference"),
            Self::Misaligned { offset, align } => {
                write!(f, "misaligned offset {offset:#x} for align {align}")
            }
            Self::OutOfBounds { offset, len, limit } => {
                write!(f, "out of bounds: {offset}..{} > {limit}", offset + len)
            }
            Self::OutOfCapacity { needed, capacity } => {
                write!(f, "arena out of capacity (needed {needed}, capacity {capacity})")
            }
            Self::InvalidUtf8 => write!(f, "invalid utf-8 in arena string"),
            Self::Overflow => write!(f, "arena offset/size overflow"),
        }
    }
}

impl std::error::Error for ArenaError {}

impl From<io::Error> for ArenaError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Sealed marker trait for verified `#[repr(C)]`, zero-padding POD types safe to store directly
/// inside [`MmapOffsetArena`].
///
/// # Safety
/// Implementing types must be `Copy`, `#[repr(C)]`, contain no pointers or invalid bit patterns,
/// and have no uninitialized padding bytes.
pub unsafe trait ArenaPod: Copy + 'static {}

// SAFETY: Primitives and verified `#[repr(C)]` zero-padding structs below.
unsafe impl ArenaPod for u8 {}
unsafe impl ArenaPod for u32 {}
unsafe impl ArenaPod for u64 {}
unsafe impl ArenaPod for ArenaHeader {}
unsafe impl ArenaPod for WorkspaceRoot {}
unsafe impl ArenaPod for ArenaTask {}

/// 32-bit base-relative pointer inside [`MmapOffsetArena`]. Offset `0` represents null.
#[repr(C)]
pub struct OffsetPtr<T> {
    pub offset: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for OffsetPtr<T> {}
impl<T> Clone for OffsetPtr<T> { fn clone(&self) -> Self { *self } }
impl<T> PartialEq for OffsetPtr<T> { fn eq(&self, o: &Self) -> bool { self.offset == o.offset } }
impl<T> Eq for OffsetPtr<T> {}
impl<T> fmt::Debug for OffsetPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OffsetPtr").field(&self.offset).finish()
    }
}

impl<T> OffsetPtr<T> {
    pub const NULL: Self = Self { offset: 0, _marker: PhantomData };
    pub const fn new(offset: u32) -> Self { Self { offset, _marker: PhantomData } }
    #[inline]
    pub const fn is_null(self) -> bool { self.offset == 0 }
}

/// 32-bit base-relative contiguous slice `(offset, len)` inside [`MmapOffsetArena`].
#[repr(C)]
pub struct OffsetSlice<T> {
    pub offset: u32,
    pub len: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Copy for OffsetSlice<T> {}
impl<T> Clone for OffsetSlice<T> { fn clone(&self) -> Self { *self } }
impl<T> PartialEq for OffsetSlice<T> {
    fn eq(&self, o: &Self) -> bool { self.offset == o.offset && self.len == o.len }
}
impl<T> Eq for OffsetSlice<T> {}
impl<T> fmt::Debug for OffsetSlice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OffsetSlice").field("offset", &self.offset).field("len", &self.len).finish()
    }
}

impl<T> OffsetSlice<T> {
    pub const EMPTY: Self = Self { offset: 0, len: 0, _marker: PhantomData };
    pub const fn new(offset: u32, len: u32) -> Self { Self { offset, len, _marker: PhantomData } }
    #[inline]
    pub const fn is_empty(self) -> bool { self.len == 0 }
}

/// 48-byte, 8-byte-aligned header stored at byte offset 0 of every [`MmapOffsetArena`] file.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArenaHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub flags: u32,
    pub capacity: u32,
    pub cursor: u32,
    pub generation: u64,
    pub header_checksum: u64,
    pub root: OffsetPtr<WorkspaceRoot>,
    pub reserved: u32,
}

/// Root record of a live mmap Studio Workspace.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceRoot {
    pub workspace_id: u64,
    pub tasks: OffsetSlice<ArenaTask>,
    pub name: OffsetSlice<u8>,
}

/// 64-byte, 8-byte-aligned live task record stored directly in `MAP_SHARED` memory.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ArenaTask {
    pub id: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub priority: u32,
    pub status: u32,
    pub assignee_id: u32,
    pub flags: u32,
    pub title: OffsetSlice<u8>,
    pub body: OffsetSlice<u8>,
    pub tag: OffsetSlice<u8>,
}

const _: () = {
    assert!(size_of::<OffsetPtr<WorkspaceRoot>>() == 4);
    assert!(align_of::<OffsetPtr<WorkspaceRoot>>() == 4);
    assert!(size_of::<OffsetSlice<ArenaTask>>() == 8);
    assert!(align_of::<OffsetSlice<ArenaTask>>() == 4);

    assert!(size_of::<ArenaHeader>() == 48);
    assert!(align_of::<ArenaHeader>() == 8);
    assert!(offset_of!(ArenaHeader, magic) == 0);
    assert!(offset_of!(ArenaHeader, version) == 8);
    assert!(offset_of!(ArenaHeader, flags) == 12);
    assert!(offset_of!(ArenaHeader, capacity) == 16);
    assert!(offset_of!(ArenaHeader, cursor) == 20);
    assert!(offset_of!(ArenaHeader, generation) == 24);
    assert!(offset_of!(ArenaHeader, header_checksum) == 32);
    assert!(offset_of!(ArenaHeader, root) == 40);
    assert!(offset_of!(ArenaHeader, reserved) == 44);

    assert!(size_of::<WorkspaceRoot>() == 24);
    assert!(align_of::<WorkspaceRoot>() == 8);
    assert!(offset_of!(WorkspaceRoot, workspace_id) == 0);
    assert!(offset_of!(WorkspaceRoot, tasks) == 8);
    assert!(offset_of!(WorkspaceRoot, name) == 16);

    assert!(size_of::<ArenaTask>() == 64);
    assert!(align_of::<ArenaTask>() == 8);
    assert!(offset_of!(ArenaTask, id) == 0);
    assert!(offset_of!(ArenaTask, created_at_ms) == 8);
    assert!(offset_of!(ArenaTask, updated_at_ms) == 16);
    assert!(offset_of!(ArenaTask, priority) == 24);
    assert!(offset_of!(ArenaTask, status) == 28);
    assert!(offset_of!(ArenaTask, assignee_id) == 32);
    assert!(offset_of!(ArenaTask, flags) == 36);
    assert!(offset_of!(ArenaTask, title) == 40);
    assert!(offset_of!(ArenaTask, body) == 48);
    assert!(offset_of!(ArenaTask, tag) == 56);
};

#[inline]
fn compute_header_checksum(hdr_bytes: &[u8; size_of::<ArenaHeader>()]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    // Skip `header_checksum` field at bytes 32..40.
    let h = fnv1a_64_update(OFFSET_BASIS, &hdr_bytes[0..32]);
    fnv1a_64_update(h, &hdr_bytes[40..48])
}

#[inline]
fn header_to_bytes(hdr: &ArenaHeader) -> [u8; size_of::<ArenaHeader>()] {
    let mut out = [0u8; size_of::<ArenaHeader>()];
    // SAFETY: `ArenaHeader` is `#[repr(C)]` with zero padding bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (hdr as *const ArenaHeader).cast::<u8>(),
            out.as_mut_ptr(),
            size_of::<ArenaHeader>(),
        );
    }
    out
}

#[inline]
fn align_up(value: usize, align: usize) -> Result<usize, ArenaError> {
    let rem = value % align;
    if rem == 0 {
        Ok(value)
    } else {
        value.checked_add(align - rem).ok_or(ArenaError::Overflow)
    }
}

/// File-backed `MAP_SHARED` 32-bit offset-arena supporting instant O(1) reopen and in-place
/// mutation without serialization.
pub struct MmapOffsetArena {
    mmap: ReadWriteMmap,
}

impl MmapOffsetArena {
    /// Creates and maps a fresh arena file sized to `capacity_bytes`.
    pub fn create(path: &Path, capacity_bytes: usize) -> Result<Self, ArenaError> {
        let min_cap = size_of::<ArenaHeader>();
        let cap_aligned = align_up(capacity_bytes.max(min_cap), 8)?;
        let cap_u32 = u32::try_from(cap_aligned).map_err(|_| ArenaError::Overflow)?;
        let mut mmap = ReadWriteMmap::create(path, cap_aligned)?;

        let mut hdr = ArenaHeader {
            magic: ARENA_MAGIC,
            version: ARENA_VERSION,
            flags: 0,
            capacity: cap_u32,
            cursor: size_of::<ArenaHeader>() as u32,
            generation: 1,
            header_checksum: 0,
            root: OffsetPtr::NULL,
            reserved: 0,
        };
        let raw = header_to_bytes(&hdr);
        hdr.header_checksum = compute_header_checksum(&raw);
        let raw_with_ck = header_to_bytes(&hdr);
        mmap.as_mut_slice()[..size_of::<ArenaHeader>()].copy_from_slice(&raw_with_ck);

        Ok(Self { mmap })
    }

    /// Reopens an existing arena file in O(1) time with zero deserialization and zero heap copies.
    pub fn open(path: &Path) -> Result<Self, ArenaError> {
        let mmap = ReadWriteMmap::open(path)?;
        let bytes = mmap.as_slice();
        if bytes.len() < size_of::<ArenaHeader>() {
            return Err(ArenaError::Truncated);
        }
        let base_addr = mmap.base_addr();
        if !base_addr.is_multiple_of(align_of::<ArenaHeader>()) {
            return Err(ArenaError::Misaligned {
                offset: base_addr,
                align: align_of::<ArenaHeader>(),
            });
        }
        let mut hdr_bytes = [0u8; size_of::<ArenaHeader>()];
        hdr_bytes.copy_from_slice(&bytes[..size_of::<ArenaHeader>()]);

        // SAFETY: `bytes.len() >= 48` and `base_addr` is page-aligned (>= 8).
        let hdr = unsafe { &*bytes.as_ptr().cast::<ArenaHeader>() };
        if hdr.magic != ARENA_MAGIC {
            return Err(ArenaError::InvalidMagic);
        }
        if hdr.version != ARENA_VERSION {
            return Err(ArenaError::UnsupportedVersion(hdr.version));
        }
        let expected_ck = compute_header_checksum(&hdr_bytes);
        if hdr.header_checksum != expected_ck {
            return Err(ArenaError::HeaderChecksumMismatch {
                expected: expected_ck,
                actual: hdr.header_checksum,
            });
        }
        let cap = hdr.capacity as usize;
        let cursor = hdr.cursor as usize;
        if cap != bytes.len() {
            return Err(ArenaError::CorruptHeader("capacity != file size"));
        }
        if cursor < size_of::<ArenaHeader>() || cursor > cap {
            return Err(ArenaError::CorruptHeader("cursor out of range"));
        }

        let arena = Self { mmap };
        if !hdr.root.is_null() {
            // Validate root pointer alignment and bounds in O(1).
            let root = arena.get(hdr.root)?;
            let _ = arena.slice(root.tasks)?;
            let _ = arena.slice(root.name)?;
        }
        Ok(arena)
    }

    /// Creates and populates a complete Studio Workspace inside a file-backed `MmapOffsetArena`.
    pub fn populate_workspace(
        path: &Path,
        workspace_id: u64,
        name: &str,
        tasks: &[OwnedTask],
        extra_headroom_bytes: usize,
    ) -> Result<Self, ArenaError> {
        let mut needed = size_of::<ArenaHeader>()
            + size_of::<WorkspaceRoot>()
            + name.len()
            + 64
            + tasks.len() * (size_of::<ArenaTask>() + 8);
        for t in tasks {
            needed += t.title.len() + t.body.len() + t.tag.len();
        }
        needed = needed
            .checked_add(extra_headroom_bytes)
            .ok_or(ArenaError::Overflow)?;

        let mut arena = Self::create(path, needed)?;
        let name_slice = arena.alloc_str(name)?;

        let mut pod_tasks = Vec::with_capacity(tasks.len());
        for t in tasks {
            let title = arena.alloc_str(&t.title)?;
            let body = arena.alloc_str(&t.body)?;
            let tag = arena.alloc_str(&t.tag)?;
            pod_tasks.push(ArenaTask {
                id: t.id,
                created_at_ms: t.created_at_ms,
                updated_at_ms: t.updated_at_ms,
                priority: t.priority,
                status: t.status,
                assignee_id: t.assignee_id,
                flags: t.flags,
                title,
                body,
                tag,
            });
        }
        let tasks_slice = arena.alloc_slice_copy(&pod_tasks)?;
        let root_ptr = arena.alloc_val(WorkspaceRoot {
            workspace_id,
            tasks: tasks_slice,
            name: name_slice,
        })?;
        arena.set_root(root_ptr)?;
        Ok(arena)
    }

    #[inline]
    pub fn header(&self) -> &ArenaHeader {
        // SAFETY: Mapping is always >= `size_of::<ArenaHeader>()` and page-aligned.
        unsafe { &*self.mmap.as_slice().as_ptr().cast::<ArenaHeader>() }
    }

    #[inline]
    fn write_header(&mut self, mut hdr: ArenaHeader) {
        let raw = header_to_bytes(&hdr);
        hdr.header_checksum = compute_header_checksum(&raw);
        let raw_with_ck = header_to_bytes(&hdr);
        self.mmap.as_mut_slice()[..size_of::<ArenaHeader>()].copy_from_slice(&raw_with_ck);
    }

    #[inline]
    pub fn base_addr(&self) -> usize {
        self.mmap.base_addr()
    }

    #[inline]
    pub fn used_bytes(&self) -> usize {
        self.header().cursor as usize
    }

    #[inline]
    pub fn capacity_bytes(&self) -> usize {
        self.header().capacity as usize
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.header().generation
    }

    fn alloc_raw(&mut self, size: usize, align: usize) -> Result<usize, ArenaError> {
        let mut hdr = *self.header();
        let start = align_up(hdr.cursor as usize, align)?;
        let end = start.checked_add(size).ok_or(ArenaError::Overflow)?;
        let cap = hdr.capacity as usize;
        if end > cap {
            return Err(ArenaError::OutOfCapacity {
                needed: end,
                capacity: cap,
            });
        }
        hdr.cursor = u32::try_from(end).map_err(|_| ArenaError::Overflow)?;
        hdr.generation = hdr.generation.wrapping_add(1);
        self.write_header(hdr);
        Ok(start)
    }

    /// Bump-allocates a single POD value inside the mapped file and returns its 32-bit offset.
    pub fn alloc_val<T: ArenaPod>(&mut self, val: T) -> Result<OffsetPtr<T>, ArenaError> {
        let off = self.alloc_raw(size_of::<T>(), align_of::<T>())?;
        let dst = &mut self.mmap.as_mut_slice()[off..off + size_of::<T>()];
        // SAFETY: `T: ArenaPod` is `#[repr(C)]` with zero padding bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (&raw const val).cast::<u8>(),
                dst.as_mut_ptr(),
                size_of::<T>(),
            );
        }
        Ok(OffsetPtr::new(off as u32))
    }

    /// Bump-allocates a contiguous slice of POD values and copies `items` into `mmap`.
    pub fn alloc_slice_copy<T: ArenaPod>(&mut self, items: &[T]) -> Result<OffsetSlice<T>, ArenaError> {
        if items.is_empty() {
            return Ok(OffsetSlice::EMPTY);
        }
        let byte_len = items
            .len()
            .checked_mul(size_of::<T>())
            .ok_or(ArenaError::Overflow)?;
        let off = self.alloc_raw(byte_len, align_of::<T>())?;
        let dst = &mut self.mmap.as_mut_slice()[off..off + byte_len];
        // SAFETY: `T: ArenaPod` has no padding or drop glue; `dst` has exact byte length.
        unsafe {
            std::ptr::copy_nonoverlapping(items.as_ptr().cast::<u8>(), dst.as_mut_ptr(), byte_len);
        }
        let len_u32 = u32::try_from(items.len()).map_err(|_| ArenaError::Overflow)?;
        Ok(OffsetSlice::new(off as u32, len_u32))
    }

    /// Bump-allocates a UTF-8 string directly inside the mapped file.
    pub fn alloc_str(&mut self, s: &str) -> Result<OffsetSlice<u8>, ArenaError> {
        self.alloc_slice_copy(s.as_bytes())
    }

    /// Updates the arena root pointer and refreshes the header checksum.
    pub fn set_root(&mut self, root: OffsetPtr<WorkspaceRoot>) -> Result<(), ArenaError> {
        let _ = self.get(root)?;
        let mut hdr = *self.header();
        hdr.root = root;
        hdr.generation = hdr.generation.wrapping_add(1);
        self.write_header(hdr);
        Ok(())
    }

    fn validate_span<T: ArenaPod>(&self, offset: usize, count: usize) -> Result<(), ArenaError> {
        let align = align_of::<T>();
        if !offset.is_multiple_of(align) {
            return Err(ArenaError::Misaligned { offset, align });
        }
        let byte_len = count
            .checked_mul(size_of::<T>())
            .ok_or(ArenaError::Overflow)?;
        let end = offset.checked_add(byte_len).ok_or(ArenaError::Overflow)?;
        let limit = self.used_bytes();
        if offset < size_of::<ArenaHeader>() || end > limit {
            return Err(ArenaError::OutOfBounds {
                offset,
                len: byte_len,
                limit,
            });
        }
        Ok(())
    }

    /// Resolves a base-relative [`OffsetPtr<T>`] into a direct borrowed reference into `mmap`.
    #[inline]
    pub fn get<T: ArenaPod>(&self, ptr: OffsetPtr<T>) -> Result<&T, ArenaError> {
        if ptr.is_null() {
            return Err(ArenaError::NullPointer);
        }
        let off = ptr.offset as usize;
        self.validate_span::<T>(off, 1)?;
        // SAFETY: `base_addr` is page-aligned, `off` is `align_of::<T>()`-aligned, and
        // `off + size_of::<T>() <= used_bytes <= mmap.len()`.
        Ok(unsafe { &*self.mmap.as_slice().as_ptr().add(off).cast::<T>() })
    }

    /// Resolves a base-relative [`OffsetPtr<T>`] into a direct mutable reference into `mmap`.
    #[inline]
    pub fn get_mut<T: ArenaPod>(&mut self, ptr: OffsetPtr<T>) -> Result<&mut T, ArenaError> {
        if ptr.is_null() {
            return Err(ArenaError::NullPointer);
        }
        let off = ptr.offset as usize;
        self.validate_span::<T>(off, 1)?;
        // SAFETY: Same alignment & bounds proof as `get`, with exclusive `&mut self`.
        Ok(unsafe { &mut *self.mmap.as_mut_slice().as_mut_ptr().add(off).cast::<T>() })
    }

    /// Resolves a base-relative [`OffsetSlice<T>`] into a direct borrowed slice into `mmap`.
    #[inline]
    pub fn slice<T: ArenaPod>(&self, s: OffsetSlice<T>) -> Result<&[T], ArenaError> {
        if s.is_empty() {
            return Ok(&[]);
        }
        let off = s.offset as usize;
        let len = s.len as usize;
        self.validate_span::<T>(off, len)?;
        // SAFETY: Validated alignment and bounds within `mmap.as_slice()`.
        Ok(unsafe {
            std::slice::from_raw_parts(self.mmap.as_slice().as_ptr().add(off).cast::<T>(), len)
        })
    }

    /// Resolves a base-relative [`OffsetSlice<T>`] into a direct mutable slice into `mmap`.
    #[inline]
    pub fn slice_mut<T: ArenaPod>(&mut self, s: OffsetSlice<T>) -> Result<&mut [T], ArenaError> {
        if s.is_empty() {
            return Ok(&mut []);
        }
        let off = s.offset as usize;
        let len = s.len as usize;
        self.validate_span::<T>(off, len)?;
        // SAFETY: Validated alignment and bounds within `mmap.as_mut_slice()` under `&mut self`.
        Ok(unsafe {
            std::slice::from_raw_parts_mut(
                self.mmap.as_mut_slice().as_mut_ptr().add(off).cast::<T>(),
                len,
            )
        })
    }

    /// Resolves an [`OffsetSlice<u8>`] into a borrowed `&str` directly inside `mmap`.
    #[inline]
    pub fn str_at(&self, s: OffsetSlice<u8>) -> Result<&str, ArenaError> {
        let bytes = self.slice(s)?;
        std::str::from_utf8(bytes).map_err(|_| ArenaError::InvalidUtf8)
    }

    #[inline]
    pub fn root(&self) -> Result<&WorkspaceRoot, ArenaError> {
        self.get(self.header().root)
    }

    #[inline]
    pub fn tasks(&self) -> Result<&[ArenaTask], ArenaError> {
        let root = *self.root()?;
        self.slice(root.tasks)
    }

    #[inline]
    pub fn tasks_mut(&mut self) -> Result<&mut [ArenaTask], ArenaError> {
        let tasks_slice = self.root()?.tasks;
        self.slice_mut(tasks_slice)
    }

    /// Mutates a task's status, priority, and timestamp directly in-place in `MAP_SHARED` memory.
    /// Returns the byte offset of the mutated [`ArenaTask`] for targeted page `msync`.
    #[inline]
    pub fn mutate_task_in_place(
        &mut self,
        index: usize,
        new_status: u32,
        new_priority: u32,
        updated_at_ms: i64,
    ) -> Result<usize, ArenaError> {
        let tasks_slice = self.root()?.tasks;
        let tasks = self.slice_mut(tasks_slice)?;
        let task = tasks.get_mut(index).ok_or(ArenaError::OutOfBounds {
            offset: index,
            len: 1,
            limit: tasks_slice.len as usize,
        })?;
        task.status = new_status;
        task.priority = new_priority;
        task.updated_at_ms = updated_at_ms;
        Ok(tasks_slice.offset as usize + index * size_of::<ArenaTask>())
    }

    /// Runs the apples-to-apples random-read workload directly over `MAP_SHARED` memory.
    pub fn random_read_checksum(&self, indices: &[usize]) -> Result<u64, ArenaError> {
        let tasks = self.tasks()?;
        let mut acc = 0u64;
        for &idx in indices {
            let t = &tasks[idx];
            let title = self.str_at(t.title)?;
            let tag = self.str_at(t.tag)?;
            acc = fold_task_fields(
                acc,
                t.id,
                t.updated_at_ms,
                t.priority,
                t.status,
                title,
                tag,
            );
        }
        Ok(acc)
    }

    /// Synchronizes the entire used region of the arena to durable storage via `msync(MS_SYNC)`.
    pub fn sync_all(&self) -> io::Result<()> {
        self.mmap.sync_range(0, self.used_bytes())
    }

    /// Synchronizes a specific mutated byte range (automatically rounded to page boundaries).
    pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
        self.mmap.sync_range(offset, len)
    }
}
