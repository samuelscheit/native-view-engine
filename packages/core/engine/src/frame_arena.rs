//! High-speed 64-byte-aligned contiguous slab bump allocator ([`FrameArena`]) with O(1)
//! borrow-checked frame reset and automatic overflow growth.
//!
//! Designed for per-frame UI render trees, layout prop tables, and transient frame buffers:
//! - Allocations take `&self` via a `Cell<usize>` bump cursor so many references live concurrently
//!   during a frame with zero per-object heap overhead.
//! - [`FrameArena::reset`] requires `&mut self`, statically proving via Rust's borrow checker that
//!   no frame references can outlive the reset.
//! - If a frame temporarily exceeds the primary slab's capacity, cold-path overflow slabs absorb
//!   the spike safely without invalidating existing references; the next [`FrameArena::reset`]
//!   coalesces the primary slab to the high-water mark so subsequent frames hit a single
//!   contiguous 64-byte-aligned slab.

use std::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use std::cell::{Cell, RefCell};
use std::mem::{align_of, size_of};
use std::ptr::NonNull;

/// Base alignment (1 cache line = 64 bytes) guaranteed for every [`FrameArena`] slab.
pub const FRAME_ARENA_BASE_ALIGN: usize = 64;

struct Slab {
    ptr: NonNull<u8>,
    capacity: usize,
    cursor: usize,
}

impl Slab {
    fn new(capacity_bytes: usize) -> Self {
        let cap = capacity_bytes
            .max(FRAME_ARENA_BASE_ALIGN)
            .checked_next_multiple_of(FRAME_ARENA_BASE_ALIGN)
            .expect("FrameArena slab capacity overflow");
        let layout =
            Layout::from_size_align(cap, FRAME_ARENA_BASE_ALIGN).expect("valid FrameArena layout");
        // SAFETY: `cap >= 64` (non-zero) and alignment is power-of-two 64.
        let raw = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            handle_alloc_error(layout);
        };
        Self {
            ptr,
            capacity: cap,
            cursor: 0,
        }
    }

    #[inline]
    fn try_alloc(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let mask = align - 1;
        let aligned = self.cursor.checked_add(mask)? & !mask;
        let end = aligned.checked_add(size)?;
        if end > self.capacity {
            return None;
        }
        self.cursor = end;
        // SAFETY: `aligned + size <= self.capacity` within this owned slab.
        Some(unsafe { self.ptr.as_ptr().add(aligned) })
    }
}

impl Drop for Slab {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.capacity, FRAME_ARENA_BASE_ALIGN)
            .expect("valid FrameArena layout");
        // SAFETY: Matches the exact `alloc(layout)` in `Slab::new`.
        unsafe {
            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

/// Contiguous 64-byte-aligned slab bump allocator with O(1) borrow-checked [`Self::reset`]
/// and automatic cold-path overflow growth.
pub struct FrameArena {
    ptr: NonNull<u8>,
    capacity: usize,
    cursor: Cell<usize>,
    overflow_used: Cell<usize>,
    high_water: Cell<usize>,
    overflow: RefCell<Vec<Slab>>,
}

// SAFETY: `FrameArena` exclusively owns its heap slabs; transferring ownership across threads
// while no shared borrows exist is sound. (`FrameArena` remains `!Sync` due to `Cell`/`RefCell`.)
unsafe impl Send for FrameArena {}

impl FrameArena {
    /// Allocates a contiguous 64-byte-aligned primary slab of at least `capacity_bytes`.
    pub fn with_capacity(capacity_bytes: usize) -> Self {
        let cap = capacity_bytes
            .max(FRAME_ARENA_BASE_ALIGN)
            .checked_next_multiple_of(FRAME_ARENA_BASE_ALIGN)
            .expect("FrameArena capacity overflow");
        let layout =
            Layout::from_size_align(cap, FRAME_ARENA_BASE_ALIGN).expect("valid FrameArena layout");
        // SAFETY: `cap >= 64` (non-zero) and alignment is power-of-two 64.
        let raw = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(raw) else {
            handle_alloc_error(layout);
        };
        Self {
            ptr,
            capacity: cap,
            cursor: Cell::new(0),
            overflow_used: Cell::new(0),
            high_water: Cell::new(0),
            overflow: RefCell::new(Vec::new()),
        }
    }

    /// Total byte capacity currently allocated across the primary slab and any active overflow slabs.
    #[inline]
    pub fn capacity_bytes(&self) -> usize {
        let overflow = self.overflow.borrow();
        if overflow.is_empty() {
            self.capacity
        } else {
            self.capacity + overflow.iter().map(|s| s.capacity).sum::<usize>()
        }
    }

    /// Primary slab byte capacity (excluding any transient overflow slabs).
    #[inline]
    pub fn primary_capacity_bytes(&self) -> usize {
        self.capacity
    }

    /// Number of bytes bumped during the current frame (including alignment padding).
    #[inline]
    pub fn used_bytes(&self) -> usize {
        self.cursor.get() + self.overflow_used.get()
    }

    /// Peak [`Self::used_bytes`] observed across all frames since creation.
    #[inline]
    pub fn high_water_bytes(&self) -> usize {
        self.high_water.get()
    }

    /// O(1) frame reset: rewinds the bump cursor to 0 without freeing or zeroing the primary slab.
    ///
    /// If the previous frame spilled into overflow slabs, `reset()` coalesces the primary slab to
    /// the high-water mark and frees the overflow slabs so subsequent frames stay 100% contiguous.
    #[inline]
    pub fn reset(&mut self) {
        let overflow = self.overflow.get_mut();
        if !overflow.is_empty() {
            self.coalesce_after_overflow();
        }
        self.cursor.set(0);
    }

    #[cold]
    #[inline(never)]
    fn coalesce_after_overflow(&mut self) {
        self.overflow.get_mut().clear();
        self.overflow_used.set(0);

        let target_cap = self
            .high_water
            .get()
            .max(self.capacity)
            .checked_next_multiple_of(FRAME_ARENA_BASE_ALIGN)
            .expect("FrameArena coalesced capacity overflow");
        if target_cap > self.capacity {
            let new_layout = Layout::from_size_align(target_cap, FRAME_ARENA_BASE_ALIGN)
                .expect("valid FrameArena layout");
            // SAFETY: `target_cap >= 64` and 64-byte aligned.
            let new_raw = unsafe { alloc(new_layout) };
            let Some(new_ptr) = NonNull::new(new_raw) else {
                handle_alloc_error(new_layout);
            };
            let old_layout = Layout::from_size_align(self.capacity, FRAME_ARENA_BASE_ALIGN)
                .expect("valid FrameArena layout");
            // SAFETY: Matches the exact layout used to allocate `self.ptr`.
            unsafe {
                dealloc(self.ptr.as_ptr(), old_layout);
            }
            self.ptr = new_ptr;
            self.capacity = target_cap;
        }
    }

    /// Fallible primary-slab raw bump allocation returning the byte offset from primary base.
    ///
    /// Does not grow into overflow slabs; returns `None` without modifying the cursor when full.
    #[inline]
    pub fn try_alloc_raw(&self, size: usize, align: usize) -> Option<usize> {
        debug_assert!(align.is_power_of_two() && align <= FRAME_ARENA_BASE_ALIGN);
        let cur = self.cursor.get();
        let mask = align - 1;
        let aligned = cur.checked_add(mask)? & !mask;
        let end = aligned.checked_add(size)?;
        if end > self.capacity {
            return None;
        }
        self.cursor.set(end);
        let total = end + self.overflow_used.get();
        if total > self.high_water.get() {
            self.high_water.set(total);
        }
        Some(aligned)
    }

    #[inline]
    fn alloc_raw_ptr(&self, size: usize, align: usize) -> *mut u8 {
        assert!(
            align.is_power_of_two() && align <= FRAME_ARENA_BASE_ALIGN,
            "FrameArena alignment must be power-of-two <= {FRAME_ARENA_BASE_ALIGN}, got {align}"
        );
        if let Some(off) = self.try_alloc_raw(size, align) {
            // SAFETY: `off + size <= self.capacity` within the primary slab.
            unsafe { self.ptr.as_ptr().add(off) }
        } else {
            self.alloc_overflow(size, align)
        }
    }

    #[cold]
    #[inline(never)]
    fn alloc_overflow(&self, size: usize, align: usize) -> *mut u8 {
        let mut slabs = self.overflow.borrow_mut();
        if let Some(last) = slabs.last_mut() {
            let before = last.cursor;
            if let Some(ptr) = last.try_alloc(size, align) {
                let delta = last.cursor - before;
                let next_overflow = self.overflow_used.get().saturating_add(delta);
                self.overflow_used.set(next_overflow);
                let total = self.cursor.get().saturating_add(next_overflow);
                if total > self.high_water.get() {
                    self.high_water.set(total);
                }
                return ptr;
            }
        }

        let prev_cap = slabs.last().map_or(self.capacity, |s| s.capacity);
        let min_growth = prev_cap.saturating_mul(2).max(size);
        let mut slab = Slab::new(min_growth);
        let ptr = slab
            .try_alloc(size, align)
            .expect("fresh overflow slab must fit requested allocation");
        let delta = slab.cursor;
        slabs.push(slab);

        let next_overflow = self.overflow_used.get().saturating_add(delta);
        self.overflow_used.set(next_overflow);
        let total = self.cursor.get().saturating_add(next_overflow);
        if total > self.high_water.get() {
            self.high_water.set(total);
        }
        ptr
    }

    /// Bump-allocates a single `Copy` value inside the arena (automatically growing via overflow
    /// slabs if the primary slab is full).
    #[inline]
    pub fn alloc_val<T: Copy>(&self, val: T) -> &T {
        if size_of::<T>() == 0 {
            let _ = val;
            // SAFETY: `NonNull::<T>::dangling()` is properly aligned for `T` and valid for ZST reads.
            return unsafe { &*NonNull::<T>::dangling().as_ptr() };
        }
        let raw = self.alloc_raw_ptr(size_of::<T>(), align_of::<T>());
        // SAFETY: `raw` is `align_of::<T>()`-aligned and uniquely claimed for `size_of::<T>()` bytes.
        unsafe {
            let slot = raw.cast::<T>();
            slot.write(val);
            &*slot
        }
    }

    /// Fallible single-value allocation strictly within the primary slab (never allocates overflow).
    #[inline]
    pub fn try_alloc_val<T: Copy>(&self, val: T) -> Option<&T> {
        if size_of::<T>() == 0 {
            let _ = val;
            // SAFETY: Properly aligned dangling pointer for ZST.
            return Some(unsafe { &*NonNull::<T>::dangling().as_ptr() });
        }
        assert!(
            align_of::<T>() <= FRAME_ARENA_BASE_ALIGN,
            "FrameArena alignment > {FRAME_ARENA_BASE_ALIGN} unsupported"
        );
        let off = self.try_alloc_raw(size_of::<T>(), align_of::<T>())?;
        // SAFETY: `ptr` is 64-byte aligned, `off` is `align_of::<T>()`-aligned, and
        // `off + size_of::<T>() <= capacity` is uniquely claimed by advancing `cursor`.
        unsafe {
            let slot = self.ptr.as_ptr().add(off).cast::<T>();
            slot.write(val);
            Some(&*slot)
        }
    }

    /// Bump-allocates a slice of `len` `Copy` elements initialized via `f(index)`.
    #[inline]
    pub fn alloc_slice_fill_with<T: Copy>(&self, len: usize, mut f: impl FnMut(usize) -> T) -> &[T] {
        if len == 0 {
            return &[];
        }
        if size_of::<T>() == 0 {
            for i in 0..len {
                let _ = f(i);
            }
            // SAFETY: Properly aligned dangling pointer for a slice of `len` ZSTs.
            return unsafe { std::slice::from_raw_parts(NonNull::<T>::dangling().as_ptr(), len) };
        }
        let bytes = len
            .checked_mul(size_of::<T>())
            .expect("FrameArena slice size overflow");
        let raw = self.alloc_raw_ptr(bytes, align_of::<T>());
        // SAFETY: Claimed non-overlapping aligned region of `len * size_of::<T>()` bytes.
        unsafe {
            let base = raw.cast::<T>();
            for i in 0..len {
                base.add(i).write(f(i));
            }
            std::slice::from_raw_parts(base, len)
        }
    }

    /// Bump-allocates a copy of `src` inside the arena.
    #[inline]
    pub fn alloc_slice_copy<T: Copy>(&self, src: &[T]) -> &[T] {
        if src.is_empty() {
            return &[];
        }
        if size_of::<T>() == 0 {
            // SAFETY: Properly aligned dangling pointer for a slice of `src.len()` ZSTs.
            return unsafe {
                std::slice::from_raw_parts(NonNull::<T>::dangling().as_ptr(), src.len())
            };
        }
        let bytes = src
            .len()
            .checked_mul(size_of::<T>())
            .expect("FrameArena slice size overflow");
        let raw = self.alloc_raw_ptr(bytes, align_of::<T>());
        // SAFETY: Claimed non-overlapping aligned region of `src.len() * size_of::<T>()` bytes.
        unsafe {
            let base = raw.cast::<T>();
            std::ptr::copy_nonoverlapping(src.as_ptr(), base, src.len());
            std::slice::from_raw_parts(base, src.len())
        }
    }

    /// Bump-allocates a UTF-8 string directly inside the arena.
    #[inline]
    pub fn alloc_str(&self, s: &str) -> &str {
        let bytes = self.alloc_slice_copy(s.as_bytes());
        // SAFETY: Copied directly from a valid `&str`.
        unsafe { std::str::from_utf8_unchecked(bytes) }
    }
}

impl Drop for FrameArena {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.capacity, FRAME_ARENA_BASE_ALIGN)
            .expect("valid FrameArena layout");
        // SAFETY: Matches the exact `alloc(layout)` in `with_capacity` / `coalesce_after_overflow`.
        unsafe {
            dealloc(self.ptr.as_ptr(), layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C, align(16))]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Align16([u32; 4]);

    #[repr(C, align(64))]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Align64([u64; 8]);

    #[repr(align(16))]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct ZstAlign16;

    #[repr(align(64))]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct ZstAlign64;

    #[test]
    fn alignment_across_1_4_8_16_64_bytes_and_strict_pointer_checks() {
        let mut arena = FrameArena::with_capacity(4096);
        assert_eq!(arena.used_bytes(), 0);
        assert_eq!(arena.capacity_bytes(), 4096);

        let b1 = arena.alloc_val(0xABu8);
        let s1 = arena.alloc_str("xyz");
        let u4 = arena.alloc_val(0x1122_3344u32);
        let b2 = arena.alloc_val(0xCDu8);
        let u8 = arena.alloc_val(0x0102_0304_0506_0708u64);
        let b3 = arena.alloc_val(0xEFu8);
        let a16 = arena.alloc_val(Align16([1, 2, 3, 4]));
        let b4 = arena.alloc_val(0x55u8);
        let a64 = arena.alloc_val(Align64([0xCAFE; 8]));

        assert_eq!((u4 as *const u32 as usize) % align_of::<u32>(), 0);
        assert_eq!((u8 as *const u64 as usize) % align_of::<u64>(), 0);
        assert_eq!((a16 as *const Align16 as usize) % align_of::<Align16>(), 0);
        assert_eq!((a64 as *const Align64 as usize) % align_of::<Align64>(), 0);

        let slice16 = arena.alloc_slice_copy(&[Align16([9; 4]), Align16([7; 4])]);
        assert_eq!((slice16.as_ptr() as usize) % align_of::<Align16>(), 0);
        assert_eq!(slice16[0], Align16([9; 4]));
        assert_eq!(slice16[1], Align16([7; 4]));

        assert_eq!((*b1, s1, *u4, *b2, *u8, *b3, *b4), (0xAB, "xyz", 0x1122_3344, 0xCD, 0x0102_0304_0506_0708, 0xEF, 0x55));
        assert_eq!(a16.0, [1, 2, 3, 4]);
        assert_eq!(a64.0, [0xCAFE; 8]);

        let peak = arena.used_bytes();
        assert!(peak >= 128);
        assert_eq!(arena.high_water_bytes(), peak);

        // Fallible primary-slab queries do not mutate cursor when exceeding capacity.
        assert!(arena.try_alloc_raw(arena.capacity_bytes(), 8).is_none());
        assert_eq!(arena.used_bytes(), peak);

        arena.reset();
        assert_eq!(arena.used_bytes(), 0);
        assert_eq!(arena.high_water_bytes(), peak);
    }

    #[test]
    fn zst_allocations_respect_alignment_and_consume_zero_bytes() {
        let arena = FrameArena::with_capacity(64);
        assert_eq!(arena.used_bytes(), 0);

        let u = arena.alloc_val(());
        let z16 = arena.alloc_val(ZstAlign16);
        let z64 = arena.alloc_val(ZstAlign64);
        assert_eq!((z16 as *const ZstAlign16 as usize) % align_of::<ZstAlign16>(), 0);
        assert_eq!((z64 as *const ZstAlign64 as usize) % align_of::<ZstAlign64>(), 0);
        assert_eq!(*u, ());

        let z_copy = arena.alloc_slice_copy(&[ZstAlign64; 17]);
        assert_eq!(z_copy.len(), 17);
        assert_eq!((z_copy.as_ptr() as usize) % align_of::<ZstAlign64>(), 0);

        let mut side_effects = 0usize;
        let z_fill = arena.alloc_slice_fill_with(11, |i| {
            side_effects += i;
            ZstAlign16
        });
        assert_eq!(z_fill.len(), 11);
        assert_eq!(side_effects, 55);
        assert_eq!((z_fill.as_ptr() as usize) % align_of::<ZstAlign16>(), 0);

        // ZST allocations consume zero slab bytes.
        assert_eq!(arena.used_bytes(), 0);
        assert_eq!(arena.try_alloc_val(ZstAlign64), Some(&ZstAlign64));
    }

    #[test]
    fn capacity_growth_preserves_references_and_coalesces_on_reset() {
        let mut arena = FrameArena::with_capacity(128);
        assert_eq!(arena.primary_capacity_bytes(), 128);

        // Hold references across multiple overflow slab allocations within one frame.
        {
            let first = arena.alloc_str("first-primary-slab-entry");
            let mut refs = Vec::new();
            for i in 0..200u64 {
                refs.push(arena.alloc_val(Align16([i as u32, (i * 2) as u32, (i * 3) as u32, 0xFF])));
            }
            let big = arena.alloc_slice_fill_with(128, |i| i as u64 * 17);
            let tail = arena.alloc_str("tail-after-growth");

            // Every earlier reference remains intact and properly aligned.
            assert_eq!(first, "first-primary-slab-entry");
            assert_eq!(tail, "tail-after-growth");
            for (i, r) in refs.iter().enumerate() {
                assert_eq!((*r as *const Align16 as usize) % align_of::<Align16>(), 0);
                assert_eq!(r.0, [i as u32, (i * 2) as u32, (i * 3) as u32, 0xFF]);
            }
            assert_eq!(big.len(), 128);
            assert_eq!(big[127], 127 * 17);
            assert!(arena.used_bytes() > 4096);
            assert!(arena.capacity_bytes() >= arena.used_bytes());
        }

        let peak = arena.high_water_bytes();
        assert!(peak > 4096);

        // Reset coalesces the primary slab so subsequent frames fit with zero overflow slabs.
        arena.reset();
        assert_eq!(arena.used_bytes(), 0);
        assert!(arena.primary_capacity_bytes() >= peak);
        assert_eq!(arena.capacity_bytes(), arena.primary_capacity_bytes());

        // Multi-frame reuse stays 100% inside the single contiguous primary slab.
        for frame in 0..10u32 {
            let s = arena.alloc_slice_fill_with(256, |i| i as u32 + frame);
            assert_eq!(s[0], frame);
            assert_eq!(s[255], 255 + frame);
            let label = arena.alloc_str("multi-frame-ok");
            assert_eq!(label, "multi-frame-ok");
            arena.reset();
            assert_eq!(arena.used_bytes(), 0);
        }
    }
}
