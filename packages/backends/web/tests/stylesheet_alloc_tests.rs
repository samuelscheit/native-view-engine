//! Heap-allocation guard for `StyleSheetCache::resolve_node`: registering a NEW structural rule
//! costs a fixed number of allocations over resolving an already-known one, independent of how
//! many rules the cache holds (one shared class allocation, declarations moved in).

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::alloc::System;

use nve_backend_web::StyleSheetCache;
use nve_components::tables::Visual;
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

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

const PROBES: u32 = 8;

fn rgb(i: u32) -> Rgba {
    Rgba::rgb((i % 251) as u8, (i / 251 % 251) as u8, 7)
}

fn allocations(f: impl FnOnce()) -> usize {
    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    f();
    ALLOC_COUNT.load(Ordering::SeqCst) - before
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
