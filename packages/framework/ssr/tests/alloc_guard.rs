//! Heap-allocation guard verifying bounded allocation budgets on `nve-ssr` hot helpers
//! (`generate_csp_nonce` and `SsrLocalPool` metadata accessors) and on streamed slot deltas
//! (`SsrStyleTracker` cost independent of how many rules were already sent).

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;

use nve_backend_web::WebBackend;
use nve_components::tables::Visual;
use nve_ssr::{generate_csp_nonce, SsrPageOptions, StreamPlan};
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

struct CountingAlloc;

// Per-thread allocation counter (const-initialized so `try_with` never allocates or re-enters).
// Isolates each test from `libtest` runner and sibling test threads.
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

fn rgb(i: u32) -> Rgba {
    Rgba::rgb((i % 251) as u8, (i / 251 % 251) as u8, 7)
}

/// Heap allocations of one `render_ssr_subtree_delta` that streams a 2-node slot with ONE new
/// rule after a shell that already sent `prior` distinct rules.
fn delta_allocations_after(prior: u32) -> usize {
    let web = WebBackend::new();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let parent = {
        let mut tx = store.tx();
        let mut kids: Vec<NodeId> = (0..prior)
            .map(|i| {
                let n = tx.create();
                tx.edit::<Visual>(n).background(rgb(i));
                n
            })
            .collect();
        let parent = tx.create();
        let sent = tx.create();
        tx.edit::<Visual>(sent).background(rgb(0));
        tx.set_children(parent, &[sent]);
        kids.push(parent);
        tx.set_children(NodeId::ROOT, &kids);
        tx.commit_into(&mut changes);
        parent
    };
    let mut shell = web.render_ssr_streaming_shell(&store).expect("shell renders");
    assert_eq!(shell.ssr.rule_hashes.len(), prior as usize, "one rule per prior node");
    {
        let mut tx = store.tx();
        let fresh = tx.create();
        tx.edit::<Visual>(fresh).background(Rgba::rgb(1, 2, 250));
        tx.append_child(parent, fresh);
        tx.commit_into(&mut changes);
    }
    let before = thread_alloc_count();
    let delta = web.render_ssr_subtree_delta(&store, parent, &mut shell.style_tracker);
    let after = thread_alloc_count();
    assert_eq!(delta.rule_hashes.len(), 1, "only the new rule is streamed: {delta:?}");
    after.wrapping_sub(before)
}

#[test]
fn counting_alloc_isolates_current_thread_and_observes_allocations() {
    let before = thread_alloc_count();
    let probe: Vec<u8> = std::hint::black_box(vec![1u8; 64]);
    let after_probe = thread_alloc_count();
    drop(probe);
    assert!(
        after_probe.wrapping_sub(before) >= 1,
        "CountingAlloc must observe allocations on the current thread"
    );

    let before_bg = thread_alloc_count();
    let handle = std::thread::spawn(|| {
        let bg: Vec<u8> = std::hint::black_box(vec![7u8; 256]);
        bg.len()
    });
    assert_eq!(handle.join().unwrap(), 256);
    let _ = before_bg; // spawn/join itself allocates on this thread; steady-state isolation below:

    let (tx, rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        tx.send(()).unwrap();
        for _ in 0..64 {
            let v: Vec<u8> = std::hint::black_box(vec![3u8; 128]);
            drop(v);
        }
        done_tx.send(()).unwrap();
    });
    rx.recv().unwrap();
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
        "background thread allocations must not pollute this thread's CountingAlloc counter"
    );
}

#[test]
fn slot_delta_allocations_do_not_scale_with_previously_sent_rules() {
    let small = delta_allocations_after(10);
    let large = delta_allocations_after(1_000);
    // Equal up to amortized container growth (`Vec` / hash-set doubling).
    assert!(
        large <= small + 3,
        "delta after 1000 sent rules allocated {large}, after 10 allocated {small}: \
         per-chunk cost must not re-seed every sent hash"
    );
}

#[test]
fn csp_nonce_and_options_have_strictly_bounded_allocations() {
    // Warm up OS RNG once before measuring.
    let _ = generate_csp_nonce().unwrap();

    let before = thread_alloc_count();
    let nonce = generate_csp_nonce().unwrap();
    let after = thread_alloc_count();
    assert_eq!(nonce.len(), 24);
    assert!(
        after.wrapping_sub(before) <= 1,
        "generate_csp_nonce should perform at most 1 String allocation, did {}",
        after.wrapping_sub(before)
    );

    let before_plan = thread_alloc_count();
    let _plan = StreamPlan::default();
    let _page = SsrPageOptions::new("", WebBackend::new);
    let after_plan = thread_alloc_count();
    assert_eq!(
        after_plan.wrapping_sub(before_plan),
        0,
        "StreamPlan::default() and SsrPageOptions::new(\"\", WebBackend::new) must perform 0 heap allocations"
    );
}
