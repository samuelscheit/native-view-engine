//! Thread-local counting global allocator shared by `nve-backend-headless` allocation guards and
//! benches: counts only the measuring thread's `alloc`/`realloc` calls, so concurrently running
//! tests or background threads never leak noise into a measurement.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    // `try_with` tolerates allocations during TLS teardown; a const `Cell` never allocates itself.
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get() + 1));
}

// SAFETY: Delegates verbatim to `std::alloc::System` while bumping a thread-local counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Runs `f` and returns its result with the number of heap allocations this thread made in it.
#[inline]
pub fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let r = f();
    (r, THREAD_ALLOCS.with(Cell::get) - before)
}
