//! Zero-allocation steady-state clean-frame and paint-only verification for [`YogaLayoutEngine`]
//! with positive + cross-thread `CountingAlloc` isolation probes.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::hint::{black_box, spin_loop};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nve_components::tables::{Layout, TextContent, Typography, Visual};
use nve_engine::Viewport;
use nve_layout::StoreLayoutEngine;
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

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

#[inline]
fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    let after = THREAD_ALLOCS.with(Cell::get);
    (out, after.wrapping_sub(before))
}

fn require_yoga_or_skip() -> bool {
    if YogaLayoutEngine::is_available() {
        return true;
    }
    assert!(
        std::env::var_os("NVE_REQUIRE_YOGA").is_none(),
        "NVE_REQUIRE_YOGA is set but real React Native Yoga was not compiled in (set RN_REACT_COMMON_DIR)"
    );
    false
}

#[test]
fn counting_alloc_positive_and_thread_isolation_probe() {
    let deadline = Instant::now() + Duration::from_secs(10);
    let (_, own) = count_allocs(|| black_box(Vec::<u64>::with_capacity(32)));
    assert!(own >= 1, "CountingAlloc must observe measuring-thread allocations");

    const WORKER_ALLOCS: usize = 1_000;
    let go = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_allocs = Arc::new(AtomicUsize::new(0));
    let worker = {
        let (go, stop, worker_allocs) = (
            Arc::clone(&go),
            Arc::clone(&stop),
            Arc::clone(&worker_allocs),
        );
        std::thread::spawn(move || {
            while !go.load(Ordering::Acquire) {
                if Instant::now() > deadline {
                    return;
                }
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
        while worker_allocs.load(Ordering::Acquire) < WORKER_ALLOCS && Instant::now() < deadline {
            spin_loop();
        }
    });
    stop.store(true, Ordering::Release);
    assert!(worker.join().is_ok());
    assert!(worker_allocs.load(Ordering::Acquire) >= WORKER_ALLOCS);
    assert_eq!(during, 0, "cross-thread allocations must not leak into this thread's count");
}

#[test]
fn yoga_steady_state_clean_and_paint_only_frames_are_zero_alloc() {
    if !require_yoga_or_skip() {
        return;
    }
    let vp = Viewport {
        width: 400.0,
        height: 300.0,
        scale_factor: 2.0,
    };
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (card, label) = {
        let mut tx = store.tx();
        let card = tx.create();
        tx.edit::<Layout>(card)
            .width(Length::points(240.0))
            .height(Length::points(80.0));
        tx.edit::<Visual>(card).opacity(1.0);
        let label = tx.create();
        tx.edit::<TextContent>(label).text("Zero-alloc Yoga frame");
        tx.edit::<Typography>(label).color(Rgba::BLACK).font_size(16.0);
        tx.set_children(card, &[label]);
        tx.set_children(NodeId::ROOT, &[card]);
        tx.commit_into(&mut changes);
        (card, label)
    };

    let mut yoga = YogaLayoutEngine::new().expect("real RN Yoga available");
    yoga.sync_changes(&store, &changes);
    assert!(yoga.compute_layout(&store, NodeId::ROOT, vp));

    // Warm one paint-only commit so `ChangeSet` scratch buffers reach steady capacity.
    let mut paint_changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        tx.edit::<Visual>(card).opacity(0.75);
        tx.edit::<Typography>(label).color(Rgba::WHITE);
        tx.commit_into(&mut paint_changes);
    }
    yoga.sync_changes(&store, &paint_changes);
    assert!(!yoga.compute_layout(&store, NodeId::ROOT, vp));

    let empty = ChangeSet::default();
    let (_, clean_allocs) = count_allocs(|| {
        for _ in 0..32 {
            yoga.sync_changes(&store, &empty);
            assert!(!yoga.compute_layout(&store, NodeId::ROOT, vp));
        }
    });
    assert_eq!(clean_allocs, 0, "clean Yoga frames must perform 0 heap allocations");

    let (_, paint_allocs) = count_allocs(|| {
        for i in 0..32u32 {
            {
                let mut tx = store.tx();
                tx.edit::<Visual>(card)
                    .opacity(if i.is_multiple_of(2) { 0.5 } else { 0.9 });
                tx.edit::<Typography>(label)
                    .color(if i.is_multiple_of(2) { Rgba::BLACK } else { Rgba::WHITE });
                tx.commit_into(&mut paint_changes);
            }
            yoga.sync_changes(&store, &paint_changes);
            assert!(!yoga.compute_layout(&store, NodeId::ROOT, vp));
        }
    });
    assert_eq!(
        paint_allocs, 0,
        "paint-only Visual::opacity + Typography::color Yoga frames must perform 0 heap allocations"
    );
}
