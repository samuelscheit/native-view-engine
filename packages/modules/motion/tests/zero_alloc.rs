//! `CountingAlloc` guard: steady-state `MotionClock` ticks and lookups allocate nothing.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::NodeId;
use nve_motion::{MotionClock, MotionKey, SpringSpec, TransitionProperty, TransitionSpec};

struct CountingAlloc;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: delegates to `System`, only counting calls on the current thread.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        // SAFETY: forwarded verbatim from the caller's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim from the caller's contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        // SAFETY: forwarded verbatim from the caller's contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn allocs_during<R>(f: impl FnOnce() -> R) -> (usize, R) {
    let before = ALLOCS.with(Cell::get);
    let out = f();
    (ALLOCS.with(Cell::get) - before, out)
}

#[test]
fn steady_state_ticks_and_retargets_allocate_nothing() {
    let (probe, _) = allocs_during(|| std::hint::black_box(Vec::<u8>::with_capacity(8)));
    assert!(probe >= 1, "the counting allocator observes allocations");

    let mut clock = MotionClock::new();
    for i in 1..=64u32 {
        let node = NodeId::new(i, 0);
        clock.animate_spring((node, TransitionProperty::Opacity), SpringSpec::snappy(), 0.0, 1.0);
        clock.animate_transition((node, TransitionProperty::Transform), TransitionSpec::all(10_000), 0.0, 1.0);
    }
    let key = MotionKey::from((NodeId::new(7, 0), TransitionProperty::Opacity));
    let (allocs, completions) = allocs_during(|| {
        let mut completions = 0;
        for _ in 0..120 {
            clock.tick_with(1.0 / 120.0, |_| completions += 1);
            // Retargeting an existing key reuses its slot.
            clock.animate_spring(key, SpringSpec::snappy(), 0.0, 0.5);
            std::hint::black_box(clock.value(&key));
        }
        completions
    });
    assert!(completions > 0, "springs settled during the measured ticks");
    assert_eq!(allocs, 0, "steady-state MotionClock ticks, lookups and retargets allocate");
}
