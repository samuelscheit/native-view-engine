//! Thread-local counting global allocator shared by `nve-backend-headless` allocation guards and
//! benches: counts only the measuring thread's `alloc`/`realloc` calls, so concurrently running
//! tests or background threads never leak noise into a measurement.
//!
//! **Per-thread scope**: work a measured closure hands to another thread (spawned threads, thread
//! pools) is never counted, so guards must keep the measured work on the calling thread.
//! [`assert_counts_only_this_thread`] proves both directions of that scope.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    // `try_with` tolerates allocations during TLS teardown; a const `Cell` never allocates itself.
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get().wrapping_add(1)));
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
    (r, THREAD_ALLOCS.with(Cell::get).wrapping_sub(before))
}

/// Positive probe (this thread's allocations count) plus isolation probe: a worker spawned
/// *before* the measured window allocates at least 1,000 times *inside* it, gated by atomics
/// (no lazily-allocating `Barrier`/`Mutex`), and none of those allocations may be counted here.
pub fn assert_counts_only_this_thread() {
    use std::hint::{black_box, spin_loop};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let (_, own) = count_allocs(|| black_box(Vec::<u64>::with_capacity(32)));
    assert!(
        own >= 1,
        "allocator hook must count this thread's allocations"
    );

    const WORKER_ALLOCS: usize = 1_000;
    let go = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_allocs = Arc::new(AtomicUsize::new(0));
    let worker = {
        let (go, stop, worker_allocs) = (go.clone(), stop.clone(), worker_allocs.clone());
        std::thread::spawn(move || {
            while !go.load(Ordering::Acquire) {
                spin_loop();
            }
            while !stop.load(Ordering::Acquire) {
                black_box(vec![0u8; 256]);
                worker_allocs.fetch_add(1, Ordering::Release);
            }
        })
    };
    let (_, during) = count_allocs(|| {
        go.store(true, Ordering::Release);
        while worker_allocs.load(Ordering::Acquire) < WORKER_ALLOCS {
            spin_loop();
        }
    });
    stop.store(true, Ordering::Release);
    assert!(worker.join().is_ok(), "allocation probe worker panicked");
    assert!(worker_allocs.load(Ordering::Acquire) >= WORKER_ALLOCS);
    assert_eq!(
        during, 0,
        "another thread's allocations leaked into this thread's count"
    );
}
