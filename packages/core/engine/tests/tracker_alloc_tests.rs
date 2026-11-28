//! `CountingAlloc` zero-allocation steady-state guard plus ancestor hover/active chain,
//! W3C enter/exit ordering, pointer capture survival across `pointer_leave`, `pointer_cancel`,
//! and `:focus-visible` modality tests for [`InteractionTracker`].

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nve_abi::NodeId;
use nve_engine::InteractionTracker;
use nve_tables::interaction::{FocusModality, InteractionState};

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

#[inline(always)]
fn bump() {
    let _ = THREAD_ALLOCS.try_with(|n| n.set(n.get().wrapping_add(1)));
}

// SAFETY: Delegates verbatim to `std::alloc::System` while bumping a thread-local counter.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::alloc_zeroed` preconditions; delegated to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn count_allocs<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = THREAD_ALLOCS.with(Cell::get);
    let out = f();
    (out, THREAD_ALLOCS.with(Cell::get).wrapping_sub(before))
}

#[test]
fn interaction_tracker_ancestor_chains_capture_and_zero_alloc_steady_state() {
    // Tree: root (1) -> card (2) -> btn_a (3) -> label_a (4)
    //                            -> btn_b (5)
    let root = NodeId::new(1, 0);
    let card = NodeId::new(2, 0);
    let btn_a = NodeId::new(3, 0);
    let label_a = NodeId::new(4, 0);
    let btn_b = NodeId::new(5, 0);

    let parent_of = |id: NodeId| -> Option<NodeId> {
        if id == label_a {
            Some(btn_a)
        } else if id == btn_a || id == btn_b {
            Some(card)
        } else if id == card {
            Some(root)
        } else {
            None
        }
    };

    let mut t = InteractionTracker::new();

    // 1. Hovering label_a activates :hover on the entire leaf-to-root ancestor chain.
    let dirty = t.pointer_move_with(Some(label_a), parent_of);
    assert_eq!(dirty, &[root, card, btn_a, label_a]);
    assert_eq!(t.hovered_chain(), &[label_a, btn_a, card, root]);
    assert_eq!(t.entered(), &[root, card, btn_a, label_a]); // top-down
    assert!(t.exited().is_empty());
    for id in [root, card, btn_a, label_a] {
        assert_eq!(t.state(id), InteractionState::HOVER);
    }

    // 2. Moving from label_a -> btn_b exits [label_a, btn_a] bottom-up and enters [btn_b] top-down;
    //    shared ancestors card & root stay continuously hovered.
    let dirty = t.pointer_move_with(Some(btn_b), parent_of);
    assert_eq!(dirty, &[btn_a, label_a, btn_b]);
    assert_eq!(t.exited(), &[label_a, btn_a]); // bottom-up
    assert_eq!(t.entered(), &[btn_b]); // top-down
    assert_eq!(t.state(card), InteractionState::HOVER);
    assert_eq!(t.state(btn_a), InteractionState::NONE);
    assert_eq!(t.state(btn_b), InteractionState::HOVER);

    // 3. Down on label_a, drag to btn_a (still inside btn_a): label_a loses :active, btn_a keeps :active,
    //    releasing on btn_a clicks btn_a (deepest common non-disabled ancestor).
    let _ = t.pointer_down_with(Some(label_a), parent_of);
    assert_eq!(t.active_chain(), &[label_a, btn_a, card, root]);
    let _ = t.pointer_move_with(Some(btn_a), parent_of);
    assert_eq!(t.active_chain(), &[btn_a, card, root]);
    let (clicked, _) = t.pointer_up_with(Some(btn_a), parent_of);
    assert_eq!(clicked, Some(btn_a));
    assert!(t.active_chain().is_empty());

    // 4. Pointer capture survives drag-off AND pointer_leave; pointer_cancel clears capture & active.
    let _ = t.pointer_down_with(Some(btn_a), parent_of);
    t.set_capture(1, btn_a);
    assert_eq!(t.captured(), Some((1, btn_a)));
    let _ = t.pointer_move_with(None, parent_of);
    assert_eq!(t.captured(), Some((1, btn_a)));
    assert_eq!(t.active_chain(), &[btn_a, card, root]);
    let _ = t.pointer_leave_slice();
    assert_eq!(t.captured(), Some((1, btn_a)), "capture must survive pointer_leave");
    assert_eq!(t.active_chain(), &[btn_a, card, root]);
    let _ = t.pointer_cancel_slice();
    assert_eq!(t.captured(), None);
    assert!(t.active_chain().is_empty());
    assert!(t.hovered_chain().is_empty());

    // 5. Focus modality (:focus vs :focus-visible).
    let _ = t.set_focus(Some(btn_a), FocusModality::Pointer);
    assert_eq!(t.state(btn_a), InteractionState::FOCUS);
    let _ = t.set_focus(Some(btn_a), FocusModality::Keyboard);
    assert_eq!(
        t.state(btn_a),
        InteractionState::FOCUS | InteractionState::FOCUS_VISIBLE
    );
    let _ = t.set_focus(None, FocusModality::Pointer);

    // 6. Positive probe proving `CountingAlloc` actually intercepts heap allocations, followed by
    //    the steady-state zero-allocation guard across 200 full interaction cycles.
    let (probe_box, probe_allocs) = count_allocs(|| std::hint::black_box(Box::new(0xDEAD_BEEFu64)));
    assert!(
        probe_allocs >= 1,
        "CountingAlloc positive probe must observe >= 1 allocation, saw {probe_allocs}"
    );
    drop(probe_box);

    let (_, allocs) = count_allocs(|| {
        for _ in 0..200 {
            let _ = t.pointer_move_with(Some(label_a), parent_of);
            let _ = t.pointer_down_with(Some(label_a), parent_of);
            t.set_capture(1, btn_a);
            let _ = t.pointer_move_with(Some(btn_b), parent_of);
            let _ = t.pointer_leave_slice();
            let _ = t.pointer_cancel_slice();
            let _ = t.pointer_down_with(Some(btn_a), parent_of);
            let _ = t.pointer_up_with(Some(btn_a), parent_of);
            let _ = t.set_focus(Some(btn_a), FocusModality::Keyboard);
            let _ = t.set_focus(Some(btn_b), FocusModality::Pointer);
            let _ = t.set_focus(None, FocusModality::Pointer);
        }
    });
    assert_eq!(
        allocs, 0,
        "steady-state InteractionTracker transitions must perform 0 heap allocations, saw {allocs}"
    );
}

#[test]
fn interaction_tracker_breaks_cyclic_parent_chains_without_duplicates() {
    let a = NodeId::new(10, 0);
    let b = NodeId::new(11, 0);
    let cyclic_parent = |id: NodeId| -> Option<NodeId> {
        if id == a {
            Some(b)
        } else if id == b {
            Some(a)
        } else {
            None
        }
    };

    let mut t = InteractionTracker::new();
    let dirty = t.pointer_move_with(Some(a), cyclic_parent);
    assert_eq!(dirty, &[b, a]);
    assert_eq!(t.hovered_chain(), &[a, b]);
    let _ = t.pointer_down_with(Some(a), cyclic_parent);
    assert_eq!(t.active_chain(), &[a, b]);
}
