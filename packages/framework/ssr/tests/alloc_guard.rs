//! Heap-allocation guard verifying bounded allocation budgets on `nve-ssr` hot helpers
//! (`generate_csp_nonce` and `SsrLocalPool` metadata accessors).

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::alloc::System;

use nve_backend_web::WebBackend;
use nve_ssr::{generate_csp_nonce, SsrPageOptions, StreamPlan};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards unchanged arguments to `System`, which upholds the `GlobalAlloc` contract.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `layout` is forwarded unchanged from our caller to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn csp_nonce_and_options_have_strictly_bounded_allocations() {
    // Warm up OS RNG once before measuring.
    let _ = generate_csp_nonce().unwrap();

    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    let nonce = generate_csp_nonce().unwrap();
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(nonce.len(), 24);
    assert!(
        after - before <= 1,
        "generate_csp_nonce should perform at most 1 String allocation, did {}",
        after - before
    );

    let before_plan = ALLOC_COUNT.load(Ordering::SeqCst);
    let _plan = StreamPlan::default();
    let _page = SsrPageOptions::new("", WebBackend::new);
    let after_plan = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(
        after_plan - before_plan,
        0,
        "StreamPlan::default() and SsrPageOptions::new(\"\", WebBackend::new) must perform 0 heap allocations"
    );
}
