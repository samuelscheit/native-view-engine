//! `CountingAlloc` guard: steady-state `MotionClock` ticks and lookups allocate nothing.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::NodeId;
use nve_engine::FrameTickHook;
use nve_motion::{
    MotionClock, MotionKey, MotionRuntimeTickHook, SpringSpec, TransitionProperty, TransitionSpec,
};
use nve_store::{ChangeSet, Length, NodeStore};
use nve_tables::tables::{register_core_tables, Layout as NveLayout, Visual};

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

#[test]
fn motion_runtime_tick_hook_idle_and_active_are_zero_alloc() {
    register_core_tables();
    let (probe, _) = allocs_during(|| std::hint::black_box(Vec::<u8>::with_capacity(8)));
    assert!(probe >= 1, "CountingAlloc must observe allocations");

    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let node = {
        let mut tx = store.tx();
        let n = tx.create();
        tx.set_children(NodeId::ROOT, &[n]);
        tx.edit::<Visual>(n).opacity(0.0).border_radius(0.0);
        tx.edit::<NveLayout>(n)
            .width(Length::points(50.0))
            .height(Length::points(40.0))
            .padding_top(Length::points(4.0))
            .padding_right(Length::points(4.0))
            .padding_bottom(Length::points(4.0))
            .padding_left(Length::points(4.0))
            .row_gap(Length::points(2.0))
            .column_gap(Length::points(2.0));
        tx.commit_into(&mut changes);
        n
    };

    let mut hook = MotionRuntimeTickHook::new();
    let clock = hook.clock();

    // Warm one empty transaction so `ChangeSet` and `NodeStore` scratch vectors have steady capacity.
    {
        let mut tx = store.tx();
        assert!(!hook.tick(&mut tx, 1.0 / 60.0));
        tx.commit_into(&mut changes);
    }

    // 1. Idle `MotionRuntimeTickHook` = 0 allocs.
    let (idle_allocs, _) = allocs_during(|| {
        for _ in 0..32 {
            let mut tx = store.tx();
            assert!(!hook.tick(&mut tx, 1.0 / 60.0));
            tx.commit_into(&mut changes);
            assert!(changes.is_empty());
        }
    });
    assert_eq!(idle_allocs, 0, "idle MotionRuntimeTickHook must perform 0 heap allocations");

    // 2. Active `MotionRuntimeTickHook` driving all 6 supported properties = 0 allocs!
    {
        let mut c = clock.lock().expect("clock lock");
        c.animate_spring((node, TransitionProperty::Opacity), SpringSpec::snappy(), 0.0, 1.0);
        c.animate_spring((node, TransitionProperty::BorderRadius), SpringSpec::snappy(), 0.0, 16.0);
        c.animate_spring((node, TransitionProperty::Width), SpringSpec::snappy(), 50.0, 200.0);
        c.animate_spring((node, TransitionProperty::Height), SpringSpec::snappy(), 40.0, 120.0);
        c.animate_spring((node, TransitionProperty::Padding), SpringSpec::snappy(), 4.0, 16.0);
        c.animate_spring((node, TransitionProperty::Gap), SpringSpec::snappy(), 2.0, 12.0);
    }
    // Warm one active commit so `ChangeSet` dirty buffers reach steady capacity.
    {
        let mut tx = store.tx();
        assert!(hook.tick(&mut tx, 1.0 / 60.0));
        tx.commit_into(&mut changes);
        assert!(!changes.is_empty());
    }

    let (active_allocs, steps) = allocs_during(|| {
        let mut steps = 0usize;
        for _ in 0..32 {
            let mut tx = store.tx();
            if hook.tick(&mut tx, 1.0 / 60.0) {
                steps += 1;
            }
            tx.commit_into(&mut changes);
        }
        steps
    });
    assert!(steps > 0, "active MotionRuntimeTickHook must advance live tracks");
    assert_eq!(active_allocs, 0, "active MotionRuntimeTickHook must perform 0 heap allocations");
}
