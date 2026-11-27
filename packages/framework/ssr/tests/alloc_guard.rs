//! Heap-allocation guard verifying bounded allocation budgets on `nve-ssr` hot helpers
//! (`generate_csp_nonce` and `SsrLocalPool` metadata accessors) and on streamed slot deltas
//! (`SsrStyleTracker` cost independent of how many rules were already sent).

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::alloc::System;

use nve_backend_web::WebBackend;
use nve_components::tables::Visual;
use nve_ssr::{generate_csp_nonce, SsrPageOptions, StreamPlan};
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

/// The counter is process-global: tests measuring it must not overlap.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    let delta = web.render_ssr_subtree_delta(&store, parent, &mut shell.style_tracker);
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(delta.rule_hashes.len(), 1, "only the new rule is streamed: {delta:?}");
    after - before
}

#[test]
fn slot_delta_allocations_do_not_scale_with_previously_sent_rules() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let small = delta_allocations_after(10);
    let large = delta_allocations_after(1_000);
    // Equal up to amortized container growth (one `BTreeMap` node split / `Vec` doubling).
    assert!(
        large <= small + 3,
        "delta after 1000 sent rules allocated {large}, after 10 allocated {small}: \
         per-chunk cost must not re-seed every sent hash"
    );
}

#[test]
fn csp_nonce_and_options_have_strictly_bounded_allocations() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Warm up OS RNG once before measuring.
    let _ = generate_csp_nonce().unwrap();

    let before = ALLOC_COUNT.load(Ordering::SeqCst);
    let nonce = generate_csp_nonce().unwrap();
    let after = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(nonce.len(), 24);
    assert!(
        after - before <= 1,
        "generate_csp_nonce should perform at most 1 String allocation, did {}",
        after - before
    );

    let before_plan = ALLOC_COUNT.load(Ordering::SeqCst);
    let _plan = StreamPlan::default();
    let _page = SsrPageOptions::new("", WebBackend::new);
    let after_plan = ALLOC_COUNT.load(Ordering::SeqCst);
    assert_eq!(
        after_plan - before_plan,
        0,
        "StreamPlan::default() and SsrPageOptions::new(\"\", WebBackend::new) must perform 0 heap allocations"
    );
}
