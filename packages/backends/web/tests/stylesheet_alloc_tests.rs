//! Heap-allocation guard for `StyleSheetCache::resolve_node`: registering a NEW structural rule
//! costs a fixed number of allocations over resolving an already-known one, independent of how
//! many rules the cache holds (one shared class allocation, declarations moved in), and an
//! already-known cursor class registers with zero allocations.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;

use nve_backend_web::StyleSheetCache;
use nve_backend_web::state_css::format_cursor_rule;
use nve_components::CursorKind;
use nve_components::tables::{Interaction, Visual};
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

struct CountingAlloc;

// Per-thread allocation counter (const-initialized so `try_with` never allocates or re-enters).
// Isolates each test from the `libtest` runner and sibling test threads.
thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

// SAFETY: every method forwards unchanged arguments to `System`, which upholds the `GlobalAlloc` contract.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged from our caller to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged from our caller to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` come from a prior `System` allocation per the caller contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

const PROBES: u32 = 8;

fn rgb(i: u32) -> Rgba {
    Rgba::rgb((i % 251) as u8, (i / 251 % 251) as u8, 7)
}

fn allocations(f: impl FnOnce()) -> usize {
    let before = thread_alloc_count();
    f();
    thread_alloc_count().wrapping_sub(before)
}

#[test]
fn counting_alloc_observes_allocations_on_this_thread_only() {
    let own = allocations(|| drop(std::hint::black_box(vec![1u8; 64])));
    assert!(
        own >= 1,
        "CountingAlloc must observe allocations on the current thread"
    );

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = ready_tx.send(());
        for _ in 0..64 {
            drop(std::hint::black_box(vec![3u8; 128]));
        }
        let _ = done_tx.send(());
    });
    ready_rx.recv().unwrap();
    let start = thread_alloc_count();
    // Wait without allocating on this thread.
    while done_rx.try_recv().is_err() {
        std::thread::yield_now();
    }
    let end = thread_alloc_count();
    worker.join().unwrap();
    assert_eq!(
        end.wrapping_sub(start),
        0,
        "background allocations must not pollute this thread"
    );
}

/// Per-probe `(new rule, known twin)` allocation counts after `prior` distinct rules.
fn registration_costs(prior: u32) -> Vec<(usize, usize)> {
    let mut store = NodeStore::new();
    let (prior_nodes, probes) = {
        let mut tx = store.tx();
        let styled = |tx: &mut nve_store::Transaction<'_>, color: Rgba| {
            let n = tx.create();
            tx.edit::<Visual>(n).background(color);
            n
        };
        let prior_nodes: Vec<NodeId> = (0..prior).map(|i| styled(&mut tx, rgb(i))).collect();
        let probes: Vec<(NodeId, NodeId)> = (0..PROBES)
            .map(|k| {
                let color = Rgba::rgb(3, k as u8, 250);
                (styled(&mut tx, color), styled(&mut tx, color))
            })
            .collect();
        tx.commit_into(&mut ChangeSet::default());
        (prior_nodes, probes)
    };
    let mut cache = StyleSheetCache::new();
    for &n in &prior_nodes {
        let _ = cache.resolve_node(&store, n);
    }
    assert_eq!(cache.rule_count(), prior as usize);
    probes
        .iter()
        .map(|&(fresh, twin)| {
            let new_rule = allocations(|| drop(cache.resolve_node(&store, fresh)));
            let known = allocations(|| drop(cache.resolve_node(&store, twin)));
            (new_rule, known)
        })
        .collect()
}

#[test]
fn registering_a_new_structural_rule_costs_fixed_allocations_independent_of_cache_size() {
    let small = registration_costs(10);
    let large = registration_costs(1_000);
    let overhead = |costs: &[(usize, usize)]| -> (usize, usize) {
        let per: Vec<usize> = costs.iter().map(|&(new, known)| new - known).collect();
        (*per.iter().min().unwrap(), *per.iter().max().unwrap())
    };
    let (small_min, small_max) = overhead(&small);
    let (large_min, large_max) = overhead(&large);
    // Known twins resolve identically regardless of cache size.
    assert_eq!(
        small.iter().map(|c| c.1).collect::<Vec<_>>(),
        large.iter().map(|c| c.1).collect::<Vec<_>>()
    );
    // Steady state: exactly ONE shared `Rc<str>` class allocation (declarations are moved in,
    // never re-cloned); amortized `Vec`/set growth may add one more on a doubling boundary.
    assert_eq!(
        (small_min, large_min),
        (1, 1),
        "small={small:?} large={large:?}"
    );
    assert!(
        small_max <= 2 && large_max <= 2,
        "small={small:?} large={large:?}"
    );
}

#[test]
fn known_cursor_class_registers_with_zero_allocations() {
    let mut store = NodeStore::new();
    let (first, twin) = {
        let mut tx = store.tx();
        let mut pointer = || {
            let n = tx.create();
            tx.edit::<Interaction>(n).cursor(CursorKind::Pointer);
            n
        };
        let pair = (pointer(), pointer());
        tx.commit_into(&mut ChangeSet::default());
        pair
    };
    let mut cache = StyleSheetCache::new();
    drop(cache.resolve_node(&store, first));
    let rules = cache.rule_count();
    let Some((cls, _)) = format_cursor_rule(CursorKind::Pointer) else {
        unreachable!("Pointer has a cursor rule");
    };
    assert!(cache.knows_structural(cls));

    // A known twin costs exactly what building its class list costs: zero registration allocs.
    let class_list = allocations(|| {
        let mut classes = String::new();
        classes.push(' ');
        classes.push_str(cls);
        drop(std::hint::black_box(classes));
    });
    let twin_cost = allocations(|| drop(cache.resolve_node(&store, twin)));
    assert_eq!(
        twin_cost, class_list,
        "known cursor hit must not allocate its declarations"
    );
    assert_eq!(cache.rule_count(), rules);
}
