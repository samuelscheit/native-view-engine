//! Core [`MmapOffsetArena`] implementation: double-buffered crash-atomic root commits,
//! O(1) `TrustedUtf8Str` reads, and online file compaction.

use std::fs::{self, OpenOptions};
use std::io;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::header::{
    encode_slot_with_checksum, select_active_slot, slot_byte_offset, ArenaHeaderSlot,
    ARENA_DATA_START, ARENA_FLAG_UTF8_TRUSTED, ARENA_HEADER_SLOT_SIZE, ARENA_MAGIC, ARENA_VERSION,
};
use super::os_mmap::ReadWriteMmap;
use super::{ArenaError, ArenaPod, OffsetPtr, OffsetSlice, TrustedUtf8Str};

static COMPACT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

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

#[inline]
fn align_up(value: usize, align: usize) -> Result<usize, ArenaError> {
    let rem = value % align;
    if rem == 0 {
        Ok(value)
    } else {
        value.checked_add(align - rem).ok_or(ArenaError::Overflow)
    }
}

#[cfg(unix)]
#[inline]
fn sync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
}

/// Live file-backed (or in-memory) 32-bit offset-arena supporting instant O(1) reopen,
/// in-place mutation, crash-atomic double-buffered root commits, O(1) `TrustedUtf8Str` reads,
/// and online compaction.
pub struct MmapOffsetArena {
    mmap: ReadWriteMmap,
    path: Option<PathBuf>,
    active_slot: usize,
    active_header: ArenaHeaderSlot,
    used_bytes: usize,
    string_count: u32,
    synced_cursor: usize,
    utf8_trusted: bool,
}

impl MmapOffsetArena {
    /// Creates and maps a fresh file-backed arena sized to at least `capacity_bytes`.
    ///
    /// Initializes Slot A (`generation = 1`) with an **XXH64** header checksum.
    pub fn create(path: impl AsRef<Path>, capacity_bytes: usize) -> Result<Self, ArenaError> {
        let path_ref = path.as_ref();
        let cap_aligned = align_up(capacity_bytes.max(ARENA_DATA_START), 8)?;
        let cap_u32 = u32::try_from(cap_aligned).map_err(|_| ArenaError::Overflow)?;
        let mmap = ReadWriteMmap::create(path_ref, cap_aligned)?;
        Ok(Self::init_fresh(mmap, Some(path_ref.to_path_buf()), cap_u32))
    }

    /// Creates an in-memory offset-arena (zero filesystem I/O; ideal for `wasm32` and unit tests).
    pub fn new_in_memory(capacity_bytes: usize) -> Result<Self, ArenaError> {
        let cap_aligned = align_up(capacity_bytes.max(ARENA_DATA_START), 8)?;
        let cap_u32 = u32::try_from(cap_aligned).map_err(|_| ArenaError::Overflow)?;
        let mmap = ReadWriteMmap::in_memory(cap_aligned)?;
        Ok(Self::init_fresh(mmap, None, cap_u32))
    }

    fn init_fresh(mut mmap: ReadWriteMmap, path: Option<PathBuf>, capacity: u32) -> Self {
        let mut slot = ArenaHeaderSlot {
            magic: ARENA_MAGIC,
            version: ARENA_VERSION,
            flags: ARENA_FLAG_UTF8_TRUSTED,
            capacity,
            used_bytes: ARENA_DATA_START as u32,
            root_offset: 0,
            string_count: 0,
            generation: 1,
            header_checksum: 0,
            reserved: [0, 0],
        };
        let encoded = encode_slot_with_checksum(&mut slot);
        mmap.as_mut_slice()[..ARENA_HEADER_SLOT_SIZE].copy_from_slice(&encoded);
        Self {
            mmap,
            path,
            active_slot: 0,
            active_header: slot,
            used_bytes: ARENA_DATA_START,
            string_count: 0,
            synced_cursor: ARENA_DATA_START,
            utf8_trusted: true,
        }
    }

    /// Reopens a trusted local arena file in O(1) time with zero deserialization.
    ///
    /// Inspects both double-buffered header slots (A & B), recovers transparently if one slot
    /// was torn mid-write, and enables fast O(1) [`Self::str_at`] when the header carries
    /// [`ARENA_FLAG_UTF8_TRUSTED`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArenaError> {
        Self::open_internal(path.as_ref(), true)
    }

    /// Reopens an untrusted arena file in O(1) time without trusting stored UTF-8 flags.
    ///
    /// [`Self::str_at`] validates UTF-8 on every read until callers verify strings explicitly.
    pub fn open_untrusted(path: impl AsRef<Path>) -> Result<Self, ArenaError> {
        Self::open_internal(path.as_ref(), false)
    }

    /// Reopens an untrusted arena file, runs `verify` to validate all reachable records and
    /// strings, and promotes the arena to trusted O(1) [`Self::str_at`] reads upon success.
    pub fn open_and_verify<F>(path: impl AsRef<Path>, verify: F) -> Result<Self, ArenaError>
    where
        F: FnOnce(&Self) -> Result<(), ArenaError>,
    {
        let mut arena = Self::open_untrusted(path)?;
        verify(&arena)?;
        arena.utf8_trusted = true;
        Ok(arena)
    }

    fn open_internal(path: &Path, trust_local_utf8: bool) -> Result<Self, ArenaError> {
        let mmap = ReadWriteMmap::open(path)?;
        let base_addr = mmap.base_addr();
        if !base_addr.is_multiple_of(align_of::<ArenaHeaderSlot>()) {
            return Err(ArenaError::Misaligned {
                offset: base_addr,
                align: align_of::<ArenaHeaderSlot>(),
            });
        }
        let (active_slot, active_header) = select_active_slot(mmap.as_slice())?;
        let used_bytes = active_header.used_bytes as usize;
        let string_count = active_header.string_count;
        let utf8_trusted =
            trust_local_utf8 && (active_header.flags & ARENA_FLAG_UTF8_TRUSTED) != 0;
        Ok(Self {
            mmap,
            path: Some(path.to_path_buf()),
            active_slot,
            active_header,
            used_bytes,
            string_count,
            synced_cursor: used_bytes,
            utf8_trusted,
        })
    }

    #[inline]
    pub fn header(&self) -> ArenaHeaderSlot {
        self.active_header
    }

    /// Returns the index (`0` = Slot A, `1` = Slot B) of the currently active committed slot.
    #[inline]
    pub fn active_slot(&self) -> usize {
        self.active_slot
    }

    #[inline]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[inline]
    pub fn base_addr(&self) -> usize {
        self.mmap.base_addr()
    }

    #[inline]
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    #[inline]
    pub fn capacity_bytes(&self) -> usize {
        self.mmap.len()
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.active_header.generation
    }

    #[inline]
    pub fn string_count(&self) -> u32 {
        self.string_count
    }

    #[inline]
    pub fn root_offset(&self) -> u32 {
        self.active_header.root_offset
    }

    #[inline]
    pub fn root_ptr<T: ArenaPod>(&self) -> OffsetPtr<T> {
        OffsetPtr::new(self.active_header.root_offset)
    }

    #[inline]
    pub fn is_utf8_trusted(&self) -> bool {
        self.utf8_trusted
    }

    /// Reserves `size` bytes with `align` alignment inside the mapped arena without modifying
    /// the on-disk active header slot (call [`Self::commit_root`] to publish atomically).
    fn alloc_raw(&mut self, size: usize, align: usize) -> Result<usize, ArenaError> {
        let start = align_up(self.used_bytes, align)?;
        let end = start.checked_add(size).ok_or(ArenaError::Overflow)?;
        let cap = self.mmap.len();
        if end > cap {
            return Err(ArenaError::OutOfCapacity {
                needed: end,
                capacity: cap,
            });
        }
        let _ = u32::try_from(end).map_err(|_| ArenaError::Overflow)?;
        self.used_bytes = end;
        Ok(start)
    }

    /// Bump-allocates a single POD value inside the mapped arena and returns its 32-bit offset.
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

    /// Bump-allocates a contiguous slice of POD values and copies `items` into the mapped arena.
    pub fn alloc_slice_copy<T: ArenaPod>(
        &mut self,
        items: &[T],
    ) -> Result<OffsetSlice<T>, ArenaError> {
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

    /// Bump-allocates a UTF-8 string directly inside the mapped arena.
    ///
    /// UTF-8 validity is guaranteed once here via `&str`, allowing zero-rescan O(1) reads via
    /// [`Self::str_at`].
    pub fn alloc_str(&mut self, s: &str) -> Result<TrustedUtf8Str, ArenaError> {
        if s.is_empty() {
            return Ok(TrustedUtf8Str::EMPTY);
        }
        let slice = self.alloc_slice_copy(s.as_bytes())?;
        self.string_count = self.string_count.saturating_add(1);
        Ok(TrustedUtf8Str::new(slice.offset, slice.len))
    }

    /// Crash-atomically commits `root` into the **inactive** header slot (`generation + 1`),
    /// flushing any newly allocated payload bytes first and `msync`ing the new header slot.
    ///
    /// Returns the newly committed `generation`.
    pub fn commit_root<T: ArenaPod>(&mut self, root: OffsetPtr<T>) -> Result<u64, ArenaError> {
        if !root.is_null() {
            let _ = self.get(root)?;
        }
        if self.used_bytes > self.synced_cursor {
            self.mmap
                .sync_range(self.synced_cursor, self.used_bytes - self.synced_cursor)?;
            self.synced_cursor = self.used_bytes;
        }
        let (next_slot, gen_val) = self.write_inactive_slot(root.offset, self.mmap.len() as u32)?;
        self.mmap
            .sync_range(slot_byte_offset(next_slot), ARENA_HEADER_SLOT_SIZE)?;
        Ok(gen_val)
    }

    /// Writes `root` into the **inactive** header slot (`generation + 1`) without calling `msync`.
    ///
    /// Pair with an explicit [`Self::sync_all`] or [`Self::commit_root`] when batching writes.
    pub fn commit_root_buffered<T: ArenaPod>(
        &mut self,
        root: OffsetPtr<T>,
    ) -> Result<u64, ArenaError> {
        if !root.is_null() {
            let _ = self.get(root)?;
        }
        let (_, gen_val) = self.write_inactive_slot(root.offset, self.mmap.len() as u32)?;
        Ok(gen_val)
    }

    fn write_inactive_slot(
        &mut self,
        root_offset: u32,
        capacity_u32: u32,
    ) -> Result<(usize, u64), ArenaError> {
        let next_slot = 1 - self.active_slot;
        let next_gen = self
            .active_header
            .generation
            .checked_add(1)
            .ok_or(ArenaError::Overflow)?;
        let used_u32 = u32::try_from(self.used_bytes).map_err(|_| ArenaError::Overflow)?;
        let flags = if self.utf8_trusted {
            ARENA_FLAG_UTF8_TRUSTED
        } else {
            0
        };
        let mut slot = ArenaHeaderSlot {
            magic: ARENA_MAGIC,
            version: ARENA_VERSION,
            flags,
            capacity: capacity_u32,
            used_bytes: used_u32,
            root_offset,
            string_count: self.string_count,
            generation: next_gen,
            header_checksum: 0,
            reserved: [0, 0],
        };
        let encoded = encode_slot_with_checksum(&mut slot);
        let off = slot_byte_offset(next_slot);
        self.mmap.as_mut_slice()[off..off + ARENA_HEADER_SLOT_SIZE].copy_from_slice(&encoded);
        self.active_slot = next_slot;
        self.active_header = slot;
        Ok((next_slot, next_gen))
    }

    #[inline]
    fn validate_span<T: ArenaPod>(&self, offset: usize, count: usize) -> Result<(), ArenaError> {
        let align = align_of::<T>();
        if !offset.is_multiple_of(align) {
            return Err(ArenaError::Misaligned { offset, align });
        }
        let byte_len = count
            .checked_mul(size_of::<T>())
            .ok_or(ArenaError::Overflow)?;
        let end = offset.checked_add(byte_len).ok_or(ArenaError::Overflow)?;
        let limit = self.used_bytes;
        if offset < ARENA_DATA_START || end > limit {
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
        // SAFETY: `base_addr` is >= 8-byte aligned, `off` is `align_of::<T>()`-aligned, and
        // `off + size_of::<T>() <= self.used_bytes <= self.mmap.len()`.
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
        // SAFETY: Same alignment & bounds proof as `get`, under exclusive `&mut self`.
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
        // SAFETY: Validated alignment and bounds within `self.mmap.as_slice()`.
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
        // SAFETY: Validated alignment and bounds within `self.mmap.as_mut_slice()` under `&mut self`.
        Ok(unsafe {
            std::slice::from_raw_parts_mut(
                self.mmap.as_mut_slice().as_mut_ptr().add(off).cast::<T>(),
                len,
            )
        })
    }

    /// Resolves a [`TrustedUtf8Str`] into a borrowed `&str` in O(1) time.
    ///
    /// Performs bounds checks on every call and skips UTF-8 re-scanning when the arena is in
    /// trusted mode (created in-process, opened locally with [`ARENA_FLAG_UTF8_TRUSTED`], or
    /// verified via [`Self::open_and_verify`]).
    #[inline]
    pub fn str_at(&self, s: TrustedUtf8Str) -> Result<&str, ArenaError> {
        if s.len == 0 {
            return Ok("");
        }
        let start = s.offset as usize;
        let len = s.len as usize;
        let end = start.checked_add(len).ok_or(ArenaError::Overflow)?;
        if start < ARENA_DATA_START || end > self.used_bytes {
            return Err(ArenaError::OutOfBounds {
                offset: start,
                len,
                limit: self.used_bytes,
            });
        }
        let bytes = &self.mmap.as_slice()[start..end];
        if self.utf8_trusted {
            // SAFETY: Validated once on write (`alloc_str`) or verified on open (`open_and_verify`).
            Ok(unsafe { std::str::from_utf8_unchecked(bytes) })
        } else {
            std::str::from_utf8(bytes).map_err(|_| ArenaError::InvalidUtf8)
        }
    }

    /// Always performs full bounds + UTF-8 validation on `s`, regardless of trust mode.
    #[inline]
    pub fn str_at_checked(&self, s: TrustedUtf8Str) -> Result<&str, ArenaError> {
        if s.len == 0 {
            return Ok("");
        }
        let start = s.offset as usize;
        let len = s.len as usize;
        let end = start.checked_add(len).ok_or(ArenaError::Overflow)?;
        if start < ARENA_DATA_START || end > self.used_bytes {
            return Err(ArenaError::OutOfBounds {
                offset: start,
                len,
                limit: self.used_bytes,
            });
        }
        let bytes = &self.mmap.as_slice()[start..end];
        std::str::from_utf8(bytes).map_err(|_| ArenaError::InvalidUtf8)
    }

    /// Synchronizes the entire used region of the arena to durable storage.
    pub fn sync_all(&self) -> io::Result<()> {
        self.mmap.sync_range(0, self.used_bytes)
    }

    /// Synchronizes a specific mutated byte range (automatically rounded to page boundaries).
    pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
        self.mmap.sync_range(offset, len)
    }

    /// Online compaction into a fresh file at `dest_path`: invokes `copy_live(old, new)` to copy
    /// only live reachable records and strings, commits the new root with monotonically advancing
    /// `generation`, shrinks the compacted file tightly to `used_bytes + headroom_bytes`, and
    /// returns the newly opened compacted arena.
    pub fn compact_to_path<R, F>(
        &self,
        dest_path: impl AsRef<Path>,
        headroom_bytes: usize,
        copy_live: F,
    ) -> Result<Self, ArenaError>
    where
        R: ArenaPod,
        F: FnOnce(&Self, &mut Self) -> Result<OffsetPtr<R>, ArenaError>,
    {
        let dest = dest_path.as_ref();
        let upper_cap = align_up(
            self.used_bytes
                .max(ARENA_DATA_START)
                .checked_add(headroom_bytes)
                .ok_or(ArenaError::Overflow)?,
            8,
        )?;
        let mut next = Self::create(dest, upper_cap)?;
        let new_root = copy_live(self, &mut next)?;
        if !new_root.is_null() {
            let _ = next.get(new_root)?;
        }

        // Preserve monotonic generation across compactions.
        let base_gen = self.generation().max(next.generation());
        next.active_header.generation = base_gen;

        let tight_cap = align_up(
            next.used_bytes
                .max(ARENA_DATA_START)
                .checked_add(headroom_bytes)
                .ok_or(ArenaError::Overflow)?,
            8,
        )?;

        if tight_cap < next.capacity_bytes() {
            let tight_u32 = u32::try_from(tight_cap).map_err(|_| ArenaError::Overflow)?;
            if next.used_bytes > next.synced_cursor {
                next.mmap
                    .sync_range(next.synced_cursor, next.used_bytes - next.synced_cursor)?;
                next.synced_cursor = next.used_bytes;
            }
            let (next_slot, _) = next.write_inactive_slot(new_root.offset, tight_u32)?;
            // Slot A carries the temporary `upper_cap` (rejected after truncation), while
            // `next_slot` carries `capacity == tight_cap` and `generation == base_gen + 1`.
            next.mmap
                .sync_range(slot_byte_offset(next_slot), ARENA_HEADER_SLOT_SIZE)?;
            drop(next);

            {
                let file = OpenOptions::new().read(true).write(true).open(dest)?;
                file.set_len(tight_cap as u64)?;
                file.sync_all()?;
            }

            Self::open(dest)
        } else {
            next.commit_root(new_root)?;
            Ok(next)
        }
    }

    /// Online in-place compaction of this file-backed arena: compacts live data into a sibling
    /// temporary file, commits and syncs it, atomically swaps it onto `self.path()` via rename,
    /// and remaps `self` onto the tight compacted file.
    ///
    /// Returns `(used_bytes_before, used_bytes_after)`.
    pub fn compact_in_place<R, F>(
        &mut self,
        headroom_bytes: usize,
        copy_live: F,
    ) -> Result<(usize, usize), ArenaError>
    where
        R: ArenaPod,
        F: FnOnce(&Self, &mut Self) -> Result<OffsetPtr<R>, ArenaError>,
    {
        let dest = self.path.clone().ok_or(ArenaError::NotFileBacked)?;
        let used_before = self.used_bytes;

        let file_name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("nve.nvearena");
        let seq = COMPACT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp_name = format!(".{file_name}.compact.{}.{seq}", current_pid());
        let tmp_path = match dest.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join(&tmp_name),
            _ => PathBuf::from(&tmp_name),
        };

        let compacted = match self.compact_to_path(&tmp_path, headroom_bytes, copy_live) {
            Ok(c) => c,
            Err(err) => {
                let _ = fs::remove_file(&tmp_path);
                return Err(err);
            }
        };
        let used_after = compacted.used_bytes;

        #[cfg(windows)]
        {
            // Windows rejects replacing a file with an active file mapping view: drop the
            // compacted view and replace `self.mmap` with a tiny placeholder before renaming.
            drop(compacted);
            self.mmap = ReadWriteMmap::in_memory(ARENA_DATA_START)?;
            if let Err(err) = fs::rename(&tmp_path, &dest) {
                let _ = fs::remove_file(&tmp_path);
                return Err(ArenaError::Io(err));
            }
            *self = Self::open(&dest)?;
        }

        #[cfg(not(windows))]
        {
            if let Err(err) = fs::rename(&tmp_path, &dest) {
                drop(compacted);
                let _ = fs::remove_file(&tmp_path);
                return Err(ArenaError::Io(err));
            }
            #[cfg(unix)]
            sync_parent_dir(&dest);
            let mut swapped = compacted;
            swapped.path = Some(dest);
            *self = swapped;
        }

        Ok((used_before, used_after))
    }
}
