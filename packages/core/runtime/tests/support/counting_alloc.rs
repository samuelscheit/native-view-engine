//! Thread-local `CountingAlloc` global allocator for zero-allocation proofs: `mod` it into a test
//! binary (it installs itself) and wrap the measured work in [`count_allocs`].

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get().wrapping_add(1)));
}

// SAFETY: Delegates verbatim to `std::alloc::System` while counting allocations on this thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Forwards unmodified layout to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: AllocLayout) -> *mut u8 {
        bump();
        // SAFETY: Forwards unmodified layout to `System::alloc_zeroed`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Forwards matching pointer and layout to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: Forwards unmodified arguments to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Runs `f` and returns its result along with the exact number of heap allocations made by the
/// current thread (unwind-safe: uses a monotonic counter delta rather than a toggle flag).
#[inline]
pub fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (out, after.wrapping_sub(before))
}
