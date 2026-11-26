//! Zero-allocation performance guards for `VirtualListState`, `RowHeights`, and
//! `FenwickHeightTree` using a custom `#[global_allocator]` (`CountingAlloc`).
//!
//! All checks execute sequentially inside a single `#[test]` entry point so no concurrent libtest
//! worker threads can introduce spurious allocations into the global counter.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nve_ui::{RowHeights, VirtualListState};

struct CountingAlloc;

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static REALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

fn count_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    REALLOC_COUNT.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
    let out = f();
    TRACKING.store(false, Ordering::SeqCst);
    let total = ALLOC_COUNT.load(Ordering::SeqCst) + REALLOC_COUNT.load(Ordering::SeqCst);
    (total, out)
}

#[test]
fn virtual_list_zero_allocation_guards() {
    // 0. Sanity check: verify CountingAlloc genuinely observes heap allocations.
    let (sanity_allocs, _) = count_allocs(|| {
        let v = black_box(Vec::<u32>::with_capacity(64));
        drop(v);
    });
    assert!(
        sanity_allocs >= 1,
        "CountingAlloc must observe Vec allocation, got {sanity_allocs}"
    );

    const N: usize = 100_000;
    let var_heights = RowHeights::variable_from_fn(N, |i| 28 + ((i * 13) % 52) as u32);
    let total_px = var_heights.total_height();
    let mut state = VirtualListState::new(var_heights.clone(), 600.0, 3);

    // 1. 0 heap allocations for RowHeights::slice() & VirtualListState::slice() across
    //    start, mid, end, fling sweep, overscroll, negative, NaN, Inf, and large overscan (>32).
    let (slice_allocs, checksum) = count_allocs(|| {
        let mut acc = 0usize;
        let edge_offsets = [
            0.0f32,
            0.5,
            123.4,
            total_px * 0.5,
            total_px - 600.0,
            total_px - 1.0,
            total_px,
            total_px + 500_000.0,
            -500.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for &off in &edge_offsets {
            for &overscan in &[0usize, 3, 32, 64] {
                let s = var_heights.slice(black_box(off), 600.0, overscan);
                acc = acc.wrapping_add(s.visible_count);
            }
            state.set_scroll_offset(off);
            let s = state.slice();
            acc = acc.wrapping_add(s.start ^ s.end);
        }

        // Fling sweep across 500 scroll positions spanning the entire 100,000-row document.
        for step in 0..500usize {
            let off = (step as f32 / 499.0) * total_px;
            state.set_scroll_offset(off);
            let s = state.slice();
            acc = acc.wrapping_add(s.first_visible + s.end_visible);
        }
        black_box(acc)
    });
    assert!(checksum > 0);
    assert_eq!(
        slice_allocs, 0,
        "100k Variable slice() must perform 0 heap allocations, got {slice_allocs}"
    );

    // 2. Uniform no-op & OOB stay Uniform with 0 allocs; after one-time uniform->variable
    //    promotion, uniquely owned set_row_height() performs 0 heap allocations.
    let mut promo = RowHeights::uniform(N, 44.0);
    let (uniform_noop_allocs, _) = count_allocs(|| {
        assert!(promo.set_row_height(50_000, 44));
        assert!(!promo.set_row_height(N, 88));
        assert!(!promo.is_variable());
    });
    assert_eq!(
        uniform_noop_allocs, 0,
        "Uniform no-op/OOB set_row_height must perform 0 allocations, got {uniform_noop_allocs}"
    );

    // One-time O(N) promotion.
    assert!(promo.set_row_height(50_000, 92));
    assert!(promo.is_variable());

    // Post-promotion: 1,000 live O(log N) updates while uniquely owned => strictly 0 allocations.
    let (update_allocs, rev_changes) = count_allocs(|| {
        let mut changes = 0usize;
        for step in 0..1_000usize {
            let idx = (step * 97) % N;
            let new_h = 24 + ((step * 17) % 90) as u32;
            let before = promo.revision();
            assert!(promo.set_row_height(idx, new_h));
            if promo.revision() != before {
                changes += 1;
            }
        }
        // Also verify OOB & no-op after promotion allocate 0 bytes.
        assert!(!promo.set_row_height(N, 50));
        let same_h = promo.row_height(0) as u32;
        assert!(promo.set_row_height(0, same_h));
        changes
    });
    assert!(rev_changes > 900);
    assert_eq!(
        update_allocs, 0,
        "Post-promotion uniquely owned set_row_height() must perform 0 allocations, got {update_allocs}"
    );

    // 3. O(1) zero-allocation render-path sync: Clone + PartialEq + revision() + borrowed slice
    //    on a 100,000-row Variable table (no O(N) compare, no 1.2 MB Fenwick clone!).
    let mut render_state = VirtualListState::new(var_heights.clone(), 600.0, 3);
    let (render_sync_allocs, sum_h) = count_allocs(|| {
        let mut acc_h = 0.0f32;
        for frame in 0..10_000usize {
            let incoming = black_box(var_heights.clone());
            // Dioxus props diffing (O(1) PartialEq).
            assert_eq!(black_box(&render_state.heights), black_box(&incoming));
            // VirtualList component height-revision sync (O(1)).
            if render_state.heights.revision() != incoming.revision()
                || render_state.heights != incoming
            {
                render_state.heights = incoming;
            }
            if frame % 100 == 0 {
                render_state.set_scroll_offset((frame as f32) * 180.0);
                let sl = render_state.slice();
                for idx in sl.start..sl.end {
                    acc_h += render_state.heights.row_height(idx);
                }
            }
        }
        black_box(acc_h)
    });
    assert!(sum_h > 0.0);
    assert_eq!(
        render_sync_allocs, 0,
        "100k render-path Clone/PartialEq/revision sync must perform 0 allocations, got {render_sync_allocs}"
    );

    // 4. Shared Variable no-op set_row_height() avoids COW clone (0 allocations, revision intact).
    let mut shared = var_heights.clone();
    let shared_rev = shared.revision();
    let existing_h = shared.row_height(12_345) as u32;
    let (shared_noop_allocs, _) = count_allocs(|| {
        assert!(shared.set_row_height(12_345, existing_h));
        assert!(!shared.set_row_height(N, 99));
    });
    assert_eq!(shared.revision(), shared_rev);
    assert_eq!(
        shared_noop_allocs, 0,
        "Shared Variable no-op/OOB set_row_height must not COW-clone, got {shared_noop_allocs}"
    );
}
