//! Zero-allocation performance guards for `RowHeights`, `VirtualListState`, and real mounted
//! `VirtualList` (`DioxusSurface` + `HeadlessBackend`) using a custom `#[global_allocator]`
//! (`CountingAlloc`) tracking both allocation count and allocated bytes.
//!
//! All checks execute sequentially inside a single `#[test]` entry point so no concurrent libtest
//! worker threads can introduce spurious allocations into the global counters.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::UiHarness;
use nve_abi::{EventBatchBuf, FrameId, SurfaceId, TimestampUs};
use nve_dioxus::prelude::*;
use nve_ui::prelude::*;

struct CountingAlloc;

static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static REALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to `std::alloc::System` while recording allocation metrics in lock-free atomics.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[derive(Debug, Clone, Copy)]
struct AllocStats {
    allocs: usize,
    bytes: usize,
}

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (AllocStats, R) {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    REALLOC_COUNT.store(0, Ordering::SeqCst);
    ALLOC_BYTES.store(0, Ordering::SeqCst);
    TRACKING.store(true, Ordering::SeqCst);
    let out = f();
    TRACKING.store(false, Ordering::SeqCst);
    let stats = AllocStats {
        allocs: ALLOC_COUNT.load(Ordering::SeqCst) + REALLOC_COUNT.load(Ordering::SeqCst),
        bytes: ALLOC_BYTES.load(Ordering::SeqCst),
    };
    (stats, out)
}

static LAST_LIVE_MUTATE_ALLOCS: AtomicUsize = AtomicUsize::new(usize::MAX);
static LAST_LIVE_MUTATE_BYTES: AtomicUsize = AtomicUsize::new(usize::MAX);
static LAST_LIVE_NOTIFY_BYTES: AtomicUsize = AtomicUsize::new(usize::MAX);

#[component]
fn Live100RowVirtualListApp() -> Element {
    let heights = use_signal(|| {
        RowHeights::variable_from_fn(100, |i| 32 + ((i % 4) as u32) * 8)
    });

    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                VirtualList {
                    test_id: "live-vlist-100",
                    heights: heights,
                    viewport_height: 300.0,
                    overscan: 2,
                    initial_selected: Some(0),
                    render_row: move |(idx, _sel)| rsx! { text { "Row #{idx}" } },
                }
            }
        }
    }
}

#[component]
fn Live100kVirtualListApp() -> Element {
    let mut heights = use_signal(|| {
        RowHeights::variable_from_fn(100_000, |i| 32 + ((i % 4) as u32) * 8)
    });

    rsx! {
        ThemeProvider {
            theme: Theme::LIGHT,
            Col {
                Button {
                    test_id: "mutate-row-50k",
                    on_press: move |_| {
                        // Measure Dioxus signal write notification separately from the
                        // RowHeights::set_row_height call itself so we prove:
                        // (a) `guard.set_row_height(...)` allocates strictly (0, 0) — zero COW!
                        // (b) `heights.write()` subscriber wakeup stays tiny O(1) (< 4 KiB).
                        let (notify_stats, mut guard) = measure_allocs(|| heights.write());
                        let (mutate_stats, ok) = measure_allocs(|| {
                            guard.set_row_height(50_000, 76)
                        });
                        drop(guard);
                        assert!(ok);
                        LAST_LIVE_MUTATE_ALLOCS.store(mutate_stats.allocs, Ordering::SeqCst);
                        LAST_LIVE_MUTATE_BYTES.store(mutate_stats.bytes, Ordering::SeqCst);
                        LAST_LIVE_NOTIFY_BYTES.store(notify_stats.bytes, Ordering::SeqCst);
                    },
                    "Mutate50k"
                }
                Button {
                    test_id: "shrink-to-5",
                    on_press: move |_| {
                        heights.set(RowHeights::uniform(5, 40));
                    },
                    "Shrink"
                }
                VirtualList {
                    test_id: "live-vlist",
                    heights: heights,
                    viewport_height: 300.0,
                    overscan: 2,
                    initial_selected: Some(0),
                    render_row: move |(idx, _sel)| rsx! { text { "Row #{idx}" } },
                }
            }
        }
    }
}

#[test]
fn virtual_list_zero_allocation_guards() {
    // 0. Sanity check: verify CountingAlloc genuinely observes heap allocations and byte counts.
    let (sanity, _) = measure_allocs(|| {
        let v = black_box(Vec::<u32>::with_capacity(64));
        drop(v);
    });
    assert!(
        sanity.allocs >= 1 && sanity.bytes >= 64 * std::mem::size_of::<u32>(),
        "CountingAlloc must observe Vec allocation & bytes, got {sanity:?}"
    );

    const N: usize = 100_000;
    let var_heights = RowHeights::variable_from_fn(N, |i| 28 + ((i * 13) % 52) as u32);
    let total_px = var_heights.total_height() as f64;
    let mut state = VirtualListState::new(var_heights.clone(), 600.0, 3);

    // 1. 0 heap allocations & 0 bytes for RowHeights::slice(), VirtualListState::slice(),
    //    scroll_to_index(), handle_key(), and VirtualListState::clone() on 100k variable rows.
    let (slice_stats, checksum) = measure_allocs(|| {
        let mut acc = 0usize;
        let edge_offsets = [
            0.0f64,
            0.5,
            123.4,
            total_px * 0.5,
            total_px - 600.0,
            total_px - 1.0,
            total_px,
            total_px + 500_000.0,
            -500.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        for &off in &edge_offsets {
            for &overscan in &[0usize, 3, 32, 64, usize::MAX] {
                let s = var_heights.slice(black_box(off), 600.0, overscan);
                acc = acc.wrapping_add(s.visible_count);
            }
            state.set_scroll_offset(off);
            let s = state.slice();
            acc = acc.wrapping_add(s.start ^ s.end);
        }

        // Fling sweep across 500 scroll positions spanning the entire 100,000-row document.
        for step in 0..500usize {
            let off = (step as f64 / 499.0) * total_px;
            state.set_scroll_offset(off);
            let s = state.slice();
            acc = acc.wrapping_add(s.first_visible + s.end_visible);
        }

        // scroll_to_index (all 4 alignments), handle_key (all keys), and state.clone() -> 0 allocs.
        for &(idx, align) in &[
            (0usize, ScrollAlign::Start),
            (25_000, ScrollAlign::Center),
            (75_000, ScrollAlign::End),
            (50_000, ScrollAlign::Nearest),
        ] {
            let off = state.scroll_to_index(idx, align);
            acc = acc.wrapping_add(off as usize);
        }
        for key in ["ArrowDown", "PageDown", "PageUp", "ArrowUp", "End", "Home", "Tab"] {
            if let Some(sel) = state.handle_key(key) {
                acc = acc.wrapping_add(sel);
            }
        }
        let cloned_state = black_box(state.clone());
        acc = acc.wrapping_add(cloned_state.heights.len());
        black_box(acc)
    });
    assert!(checksum > 0);
    assert_eq!(
        (slice_stats.allocs, slice_stats.bytes),
        (0, 0),
        "100k Variable slice/scroll_to_index/handle_key/clone must allocate 0 bytes, got {slice_stats:?}"
    );

    // 2. Uniform no-op & OOB stay Uniform with 0 allocs; after one-time uniform->variable
    //    promotion, uniquely owned set_row_height() performs 0 heap allocations with exact
    //    expected revision bump count.
    let mut promo = RowHeights::uniform(N, 44);
    let (uniform_noop, _) = measure_allocs(|| {
        assert!(promo.set_row_height(50_000, 44));
        assert!(!promo.set_row_height(N, 88));
        assert!(!promo.is_variable());
    });
    assert_eq!(
        (uniform_noop.allocs, uniform_noop.bytes),
        (0, 0),
        "Uniform no-op/OOB set_row_height must allocate 0 bytes, got {uniform_noop:?}"
    );

    // One-time O(N) promotion.
    assert!(promo.set_row_height(50_000, 92));
    assert!(promo.is_variable());

    let (update_stats, (actual_rev_changes, expected_rev_changes)) = measure_allocs(|| {
        let mut actual_changes = 0usize;
        let mut expected_changes = 0usize;
        for step in 0..1_000usize {
            let idx = (step * 97) % N;
            let new_h = 24 + ((step * 17) % 90) as u32;
            let old_h = promo.row_height(idx);
            let before = promo.revision();
            assert!(promo.set_row_height(idx, new_h));
            if new_h != old_h {
                expected_changes += 1;
            }
            if promo.revision() != before {
                actual_changes += 1;
            }
        }
        assert!(!promo.set_row_height(N, 50));
        let same_h = promo.row_height(0);
        let rev_before_noop = promo.revision();
        assert!(promo.set_row_height(0, same_h));
        assert_eq!(promo.revision(), rev_before_noop);
        (actual_changes, expected_changes)
    });
    assert_eq!(
        actual_rev_changes, expected_rev_changes,
        "revision must bump iff row height actually changed"
    );
    assert!(expected_rev_changes > 0);
    assert_eq!(
        (update_stats.allocs, update_stats.bytes),
        (0, 0),
        "Post-promotion uniquely owned set_row_height() must allocate 0 bytes, got {update_stats:?}"
    );

    // 3. O(1) zero-allocation render-path sync for BOTH Variable (100k) and Uniform (100k).
    let uniform_100k = RowHeights::uniform(N, 44);
    let mut render_var = VirtualListState::new(var_heights.clone(), 600.0, 3);
    let render_uni = VirtualListState::new(uniform_100k.clone(), 600.0, 3);
    let (render_sync_stats, sum_h) = measure_allocs(|| {
        let mut acc_h = 0u64;
        for frame in 0..10_000usize {
            let inc_v = black_box(var_heights.clone());
            assert_eq!(black_box(&render_var.heights), black_box(&inc_v));
            assert_eq!(render_var.heights.revision(), inc_v.revision());

            let inc_u = black_box(RowHeights::uniform(N, 44));
            assert_eq!(black_box(&render_uni.heights), black_box(&inc_u));
            assert_eq!(render_uni.heights.revision(), inc_u.revision());

            if frame % 100 == 0 {
                render_var.set_scroll_offset((frame as f64) * 180.0);
                let sl = render_var.slice();
                for idx in sl.start..sl.end {
                    acc_h += u64::from(render_var.heights.row_height(idx));
                }
            }
        }
        black_box(acc_h)
    });
    assert!(sum_h > 0);
    assert_eq!(
        (render_sync_stats.allocs, render_sync_stats.bytes),
        (0, 0),
        "100k Variable + Uniform render-path sync must allocate 0 bytes, got {render_sync_stats:?}"
    );

    // 4. Shared Variable no-op & OOB set_row_height() avoids COW clone (0 allocs, 0 bytes).
    let mut shared = var_heights.clone();
    let shared_rev = shared.revision();
    let existing_h = shared.row_height(12_345);
    let (shared_noop, _) = measure_allocs(|| {
        assert!(shared.set_row_height(12_345, existing_h));
        assert!(!shared.set_row_height(N, 99));
    });
    assert_eq!(shared.revision(), shared_rev);
    assert_eq!(
        (shared_noop.allocs, shared_noop.bytes),
        (0, 0),
        "Shared Variable no-op/OOB set_row_height must not COW-clone, got {shared_noop:?}"
    );

    // 5. Real mounted 100k headless VirtualList (DioxusSurface + HeadlessBackend):
    //    - Parent `heights.write().set_row_height(50_000, 76)` allocates 0 bytes (no 1.2 MB COW!).
    //    - Scrolling to row #50,000 + re-rendering allocates strictly O(viewport) bytes (< 64 KiB)
    //      and renders row #50,000 with its exact live-updated 76px height.
    //    - Shrinking `heights` to 5 rows clamps both scroll_offset and selected_index cleanly.
    let mut harness = UiHarness::new(Live100kVirtualListApp);
    assert!(harness.maybe_find("live-vlist-row-0").is_some());

    // Live mutation on parent Signal<RowHeights> while VirtualList is mounted.
    harness.press("mutate-row-50k");
    assert_eq!(
        (
            LAST_LIVE_MUTATE_ALLOCS.load(Ordering::SeqCst),
            LAST_LIVE_MUTATE_BYTES.load(Ordering::SeqCst),
        ),
        (0, 0),
        "Parent guard.set_row_height() while VirtualList is mounted must allocate 0 bytes (no COW clone!)"
    );
    let notify_bytes = LAST_LIVE_NOTIFY_BYTES.load(Ordering::SeqCst);
    assert!(
        notify_bytes < 4 * 1024,
        "Dioxus Signal::write() notification must stay tiny O(1) (< 4 KiB), got {notify_bytes} B"
    );

    // Measure pure DioxusSurface scroll dispatch + commit_to at N=100 vs N=100,000
    // (1,000x larger dataset!) to prove strictly O(viewport) allocation independent of N.
    let mut harness_100 = UiHarness::new(Live100RowVirtualListApp);
    let vlist_100_id = harness_100.find("live-vlist-100");
    let mut scroll_100_buf = EventBatchBuf::with_capacity(1, 0);
    scroll_100_buf.push_scroll(
        SurfaceId::ROOT,
        vlist_100_id,
        TimestampUs(16_000),
        0.0,
        1_800.0,
        800.0,
        4_400.0,
    );
    let (scroll_100_stats, _) = measure_allocs(|| {
        assert!(harness_100.surface.dispatch_events(scroll_100_buf.as_validated()));
        harness_100
            .surface
            .commit_to(FrameId(2), &mut harness_100.backend)
            .expect("commit 100 scroll frame");
    });

    let vlist_100k_id = harness.find("live-vlist");
    let mut scroll_100k_buf = EventBatchBuf::with_capacity(1, 0);
    scroll_100k_buf.push_scroll(
        SurfaceId::ROOT,
        vlist_100k_id,
        TimestampUs(32_000),
        0.0,
        2_200_000.0,
        800.0,
        5_000_000.0,
    );
    let (scroll_100k_stats, _) = measure_allocs(|| {
        assert!(harness.surface.dispatch_events(scroll_100k_buf.as_validated()));
        harness
            .surface
            .commit_to(FrameId(10), &mut harness.backend)
            .expect("commit 100k scroll frame");
    });

    // 100k-row DioxusSurface scroll+render must match 100-row O(viewport) footprint within 16 KiB
    // and stay well under 256 KiB (proving zero 1.2 MB Fenwick tree clone!).
    assert!(
        scroll_100k_stats.bytes <= scroll_100_stats.bytes + 16 * 1024
            && scroll_100k_stats.bytes < 256 * 1024,
        "100k VirtualList scroll+render must stay O(viewport) & N-independent: N=100k allocated {} B ({} allocs) vs N=100 allocated {} B ({} allocs)",
        scroll_100k_stats.bytes,
        scroll_100k_stats.allocs,
        scroll_100_stats.bytes,
        scroll_100_stats.allocs,
    );
    eprintln!(
        "[CountingAlloc live VirtualList] parent set_row_height=(0 allocs, 0 B) | Dioxus signal notify={} B | scroll+render N=100: {} B ({} allocs) vs N=100k: {} B ({} allocs)",
        notify_bytes,
        scroll_100_stats.bytes,
        scroll_100_stats.allocs,
        scroll_100k_stats.bytes,
        scroll_100k_stats.allocs,
    );
    let row_50k_id = harness.find("live-vlist-row-50000");
    let row_50k_h = harness.backend.layout_of(row_50k_id).unwrap().height;
    assert_eq!(
        row_50k_h, 76.0,
        "live mutated row #50,000 must render at updated 76px height, got {row_50k_h}"
    );

    // Select row #50,000 then shrink list to 5 rows: both scroll_offset and selected_index clamp.
    harness.press("live-vlist-row-50000");
    assert_eq!(
        harness.raw_str(harness.find("live-vlist-row-50000"), "ariaSelected"),
        Some("true")
    );
    harness.press("shrink-to-5");
    assert!(harness.maybe_find("live-vlist-row-50000").is_none());
    assert!(harness.maybe_find("live-vlist-row-0").is_some());
    assert_eq!(
        harness.raw_str(harness.find("live-vlist-row-4"), "ariaSelected"),
        Some("true"),
        "selected_index must clamp to len - 1 (4) when heights shrinks"
    );
}
