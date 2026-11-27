//! Thread-local `CountingAlloc` zero-allocation guard for steady-state `DioxusSurface::dispatch_events`.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{EventBatchBuf, EventFlags, FrameId, SurfaceId, TimestampUs};
use nve_components::tables::DebugId;
use nve_dioxus::prelude::*;
use nve_store::{NodeId, NodeStore};

struct CountingAlloc;

thread_local! {
    static TRACK_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

static GLOBAL_PROBE_HITS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `System` allocator while counting allocations only on the active thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        TRACK_THREAD.with(|active| {
            if active.get() {
                THREAD_ALLOCS.with(|c| c.set(c.get() + 1));
                GLOBAL_PROBE_HITS.fetch_add(1, Ordering::Relaxed);
            }
        });
        // SAFETY: Forwards exact layout to `System::alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Forwards exact pointer and layout to `System::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        TRACK_THREAD.with(|active| {
            if active.get() {
                THREAD_ALLOCS.with(|c| c.set(c.get() + 1));
                GLOBAL_PROBE_HITS.fetch_add(1, Ordering::Relaxed);
            }
        });
        // SAFETY: Forwards exact arguments to `System::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    TRACK_THREAD.with(|a| a.set(true));
    let out = f();
    TRACK_THREAD.with(|a| a.set(false));
    let count = THREAD_ALLOCS.with(Cell::get);
    (count, out)
}

#[test]
fn counting_alloc_positive_probe_detects_heap_allocation() {
    let (allocs, len) = measure_allocs(|| {
        let mut v = Vec::with_capacity(64);
        v.push(42u8);
        std::hint::black_box(v).len()
    });
    assert_eq!(len, 1);
    assert!(allocs >= 1, "CountingAlloc must observe intentional heap allocation");
}

#[component]
fn ZeroAllocEventApp() -> Element {
    rsx! {
        view {
            test_id: "root-box",
            onpress: move |ev| {
                std::hint::black_box((ev.x, ev.y));
            },
            onkeydown: move |ev| {
                std::hint::black_box(ev.key.len());
            },
            textinput {
                test_id: "input-box",
                onchange: move |ev| {
                    std::hint::black_box(ev.value.len());
                },
            }
        }
    }
}

fn find_by_test_id(store: &NodeStore, tid: &str) -> Option<NodeId> {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if id != NodeId::ROOT
            && let Some(dbg) = store.try_row::<DebugId>(id)
            && dbg.test_id() == tid
        {
            return Some(id);
        }
        stack.extend_from_slice(store.children(id));
    }
    None
}

#[test]
fn steady_state_dioxus_dispatch_events_reuses_slots_without_growing_allocs() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, ZeroAllocEventApp);
    surface.reconcile(FrameId(1));

    let root = find_by_test_id(surface.store(), "root-box").expect("root-box mounted");
    let input = find_by_test_id(surface.store(), "input-box").expect("input-box mounted");

    let mut batch = EventBatchBuf::with_capacity(8, 256);
    // Warm EventSlots buffers (String capacities + Rc boxes) once.
    batch.push_press(SurfaceId::ROOT, root, TimestampUs(10), 12.0, 24.0);
    batch.push_key_down(
        SurfaceId::ROOT,
        root,
        TimestampUs(20),
        "ArrowRight",
        "ArrowRight",
        EventFlags::MOD_SHIFT,
        39,
    );
    batch.push_text_change(SurfaceId::ROOT, input, TimestampUs(30), "hello steady state");
    assert!(surface.dispatch_events(batch.as_validated()));

    // Upstream dioxus-core 0.7.10 allocates exactly 2 boxes per bubbling event that NVE cannot
    // reuse: `Event::new` → `Rc<RefCell<EventMetadata>>` and `handle_bubbling_event` → the
    // listener `vec![]` (docs/UPSTREAM.md). All three events below bubble.
    const DIOXUS_ALLOCS_PER_BUBBLING_EVENT: usize = 2;
    const EVENTS_PER_BATCH: usize = 3;
    let mut run_batch = |key: &str, text: &str| {
        batch.clear();
        batch.push_press(SurfaceId::ROOT, root, TimestampUs(100), 1.0, 2.0);
        batch.push_key_down(SurfaceId::ROOT, root, TimestampUs(110), key, key, EventFlags::EMPTY, 13);
        batch.push_text_change(SurfaceId::ROOT, input, TimestampUs(120), text);
        surface.dispatch_events(batch.as_validated())
    };

    // NVE-side work (EventSlots `Rc` boxes + `String` capacities) adds 0 allocations.
    let (allocs_1, handled_1) = measure_allocs(|| run_batch("Enter", "hello"));
    assert!(handled_1);
    assert_eq!(
        allocs_1,
        EVENTS_PER_BATCH * DIOXUS_ALLOCS_PER_BUBBLING_EVENT,
        "steady-state dispatch_events must add 0 NVE-side allocations on top of dioxus-core's per-event boxes"
    );

    // Payload size within warmed capacity does not change the count (String reuse).
    let (allocs_long, _) = measure_allocs(|| run_batch("ArrowRight", "hello steady state"));
    assert_eq!(allocs_long, allocs_1, "payload strings must reuse warmed capacity");

    // Cost is linear in events, never growing with dispatch history.
    let (allocs_20, _) = measure_allocs(|| {
        for _ in 0..20 {
            run_batch("Enter", "hello");
        }
    });
    assert_eq!(allocs_20, 20 * allocs_1, "per-batch allocations must not grow across dispatches");
}
