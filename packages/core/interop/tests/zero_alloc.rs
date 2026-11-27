//! Thread-local `CountingAlloc` zero-allocation steady-state guards for `nve-interop`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use nve_abi::{SurfaceId, TimestampUs};
use nve_interop::{
    FrameworkKind, GestureCapabilities, InteropRuntime, PointerPhase, PointerSample,
    SurfaceEnvironment, SurfaceOrchestrator,
};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

// SAFETY: Delegates directly to `System` while incrementing a thread-local counter via `try_with`
// so concurrent test threads never race or panic during TLS teardown.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Forwards exact `layout` to the global `System` allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forwards exact `(ptr, layout)` previously allocated by `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().wrapping_add(1)));
        // SAFETY: Forwards exact realloc parameters to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (usize, R) {
    THREAD_ALLOCS.with(|c| c.set(0));
    let out = f();
    let count = THREAD_ALLOCS.with(Cell::get);
    (count, out)
}

#[test]
fn surface_orchestrator_lookups_and_pointer_arbitration_zero_alloc_steady_state() {
    let mut runtime = InteropRuntime::new();
    let mut ids = Vec::with_capacity(8);
    let root = runtime
        .surfaces
        .register_root(FrameworkKind::Flutter, "Root", SurfaceEnvironment::default())
        .unwrap();
    ids.push(root);
    let mut parent = root;
    for depth in 1..8 {
        let child = runtime
            .surfaces
            .register_child(parent, FrameworkKind::Nve, format!("Child{depth}"))
            .unwrap();
        ids.push(child);
        parent = child;
    }
    let leaf = *ids.last().unwrap();

    for (i, &sid) in ids.iter().enumerate() {
        let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
        if i > 2 {
            caps.scroll_edges.at_bottom = true;
        }
        runtime.gestures.set_capabilities(sid, caps);
    }

    // Warm one full Down -> Move -> Up pointer cycle so gesture state tables are initialized.
    for (phase, dy, ts) in [
        (PointerPhase::Down, 0.0, 100),
        (PointerPhase::Move, -30.0, 200),
        (PointerPhase::Up, 0.0, 300),
    ] {
        let _ = runtime
            .arbitrate_pointer(
                leaf,
                PointerSample {
                    pointer_id: 1,
                    phase,
                    x: 20.0,
                    y: 100.0,
                    delta_x: 0.0,
                    delta_y: dy,
                    timestamp_us: TimestampUs(ts),
                },
            )
            .unwrap();
    }

    let mut chain_buf = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];

    // 1. All hot SurfaceOrchestrator reads + stack ancestor walk must perform 0 allocations.
    let (read_allocs, checksum) = measure_allocs(|| {
        let mut acc = 0usize;
        for _ in 0..200 {
            for &sid in &ids {
                acc += runtime.surfaces.get(black_box(sid)).unwrap().children.len();
                acc += runtime.surfaces.children(black_box(sid)).unwrap().len();
            }
            acc += runtime.surfaces.roots().len();
            acc += runtime.surfaces.active_count();
            let len = runtime
                .surfaces
                .ancestor_chain_into(black_box(leaf), &mut chain_buf)
                .unwrap()
                .unwrap();
            acc += len;
        }
        acc
    });
    black_box(checksum);
    assert_eq!(
        read_allocs, 0,
        "SurfaceOrchestrator hot reads must perform 0 heap allocations (saw {read_allocs})"
    );

    // 2. InteropRuntime::arbitrate_pointer steady-state (when no cancelled surfaces spill) must
    // perform 0 allocations — ancestor_chain_into stays strictly on the stack!
    let (arb_allocs, winner) = measure_allocs(|| {
        let mut last = None;
        for step in 0..100u64 {
            let down = runtime
                .arbitrate_pointer(
                    black_box(root),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Down,
                        x: 10.0,
                        y: 10.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(step * 1000 + 1),
                    },
                )
                .unwrap();
            let mv = runtime
                .arbitrate_pointer(
                    black_box(root),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Move,
                        x: 10.0,
                        y: 40.0,
                        delta_x: 0.0,
                        delta_y: 30.0,
                        timestamp_us: TimestampUs(step * 1000 + 2),
                    },
                )
                .unwrap();
            let up = runtime
                .arbitrate_pointer(
                    black_box(root),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Up,
                        x: 10.0,
                        y: 40.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(step * 1000 + 3),
                    },
                )
                .unwrap();
            last = down.winner.or(mv.winner).or(up.winner);
        }
        last
    });
    black_box(winner);
    assert_eq!(
        arb_allocs, 0,
        "InteropRuntime::arbitrate_pointer must perform 0 heap allocations on ancestor lookup (saw {arb_allocs})"
    );
}
