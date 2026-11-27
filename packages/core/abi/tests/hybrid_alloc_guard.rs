//! Allocation guard for the hybrid trampoline plumbing: buffers handed to a null out-pointer are
//! released, never leaked, and failed or empty allocations allocate nothing.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::hybrid::{
    NveHybridOwnedBytes, NveHybridStatus, fail, nve_hybrid_alloc_bytes, nve_hybrid_free_bytes,
    store_bytes,
};

/// Counts this thread's live heap bytes, so parallel tests never add noise.
struct LiveBytes;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn track(delta: isize) {
    let _ = LIVE.try_with(|n| n.set(n.get() + delta));
}

fn size(layout: Layout) -> isize {
    isize::try_from(layout.size()).unwrap_or(isize::MAX)
}

// SAFETY: Delegates verbatim to `System` while tracking this thread's live byte count.
unsafe impl GlobalAlloc for LiveBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track(size(layout));
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        track(size(layout));
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        track(-size(layout));
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track(isize::try_from(new_size).unwrap_or(isize::MAX) - size(layout));
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: LiveBytes = LiveBytes;

/// Live bytes `f` leaves behind on this thread.
fn leaked(f: impl FnOnce()) -> isize {
    let before = LIVE.with(Cell::get);
    f();
    LIVE.with(Cell::get) - before
}

#[test]
fn null_out_pointers_release_owned_buffers() {
    assert_eq!(
        leaked(|| {
            // SAFETY: A null out-pointer is allowed; the message is dropped, not handed out.
            let status = unsafe {
                fail(
                    std::ptr::null_mut(),
                    NveHybridStatus::InvalidArg,
                    "x".repeat(4096),
                )
            };
            assert_eq!(status, NveHybridStatus::InvalidArg as i32);
        }),
        0
    );
    assert_eq!(
        leaked(|| {
            // SAFETY: A null out-pointer is allowed; the buffer comes from `from_vec`.
            unsafe {
                store_bytes(
                    std::ptr::null_mut(),
                    NveHybridOwnedBytes::from_vec(vec![7; 4096]),
                );
            }
        }),
        0
    );
}

#[test]
fn empty_and_impossible_buffers_allocate_nothing() {
    assert_eq!(
        leaked(|| {
            for buf in [
                NveHybridOwnedBytes::from_vec(Vec::with_capacity(256)),
                nve_hybrid_alloc_bytes(0),
                nve_hybrid_alloc_bytes(usize::MAX),
            ] {
                assert!(buf.ptr.is_null());
                // SAFETY: `EMPTY` is always releasable.
                unsafe { nve_hybrid_free_bytes(buf) };
            }
        }),
        0
    );
    assert_eq!(
        leaked(|| {
            let buf = nve_hybrid_alloc_bytes(1024);
            // SAFETY: Allocated by `nve_hybrid_alloc_bytes`, released once.
            unsafe { nve_hybrid_free_bytes(buf) };
        }),
        0
    );
}
