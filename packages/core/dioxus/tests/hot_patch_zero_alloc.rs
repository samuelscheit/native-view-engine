//! Zero-allocation `CountingAlloc` integration tests for Tier-1 binary `RowPatch` hot reload:
//! - Patches an un-tagged (`test_id: None`) `rsx!` element via `(template_key, node_path)` with
//!   0 heap allocations, leaving identical sibling nodes untouched.
//! - Patches every live instance of a `for`-loop template AND a sibling following the `for` loop
//!   with 0 heap allocations, including after loop shrinkage / node eviction.

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nve_abi::{FrameId, SurfaceId};
use nve_dioxus::prelude::*;
use nve_engine::{hot_patch_template_key, HotPatchMsg};
use nve_store::{FlatResource, Length, NodeId, NodeStore, Rgba, RowPatch};
use nve_tables::tables::{Layout, TextContent, Visual};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `std::alloc::System` while incrementing atomic counters.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn measure_allocs<R>(f: impl FnOnce() -> R) -> (R, usize, usize) {
    let c0 = ALLOC_COUNT.load(Ordering::SeqCst);
    let b0 = ALLOC_BYTES.load(Ordering::SeqCst);
    let out = f();
    let dc = ALLOC_COUNT.load(Ordering::SeqCst) - c0;
    let db = ALLOC_BYTES.load(Ordering::SeqCst) - b0;
    (out, dc, db)
}

fn encode_patch(rp: &RowPatch) -> Vec<u32> {
    let mut words = Vec::new();
    assert!(rp.write_words(&mut words));
    words
}

// Template ordinal 0: untagged card with two identical sibling children (zero `test_id`s).
fn untagged_card_app() -> Element {
    rsx! {
        view {
            view {
                padding: 8.0,
                opacity: 1.0,
                text { "Untagged Alpha" }
            }
            view {
                padding: 8.0,
                opacity: 1.0,
                text { "Untagged Beta" }
            }
        }
    }
}

#[derive(Clone)]
struct LoopState {
    count: Signal<usize>,
}

// Template ordinal 1: outer `rsx!`; ordinal 2: `for`-loop body template (zero `test_id`s).
fn loop_and_after_app() -> Element {
    let state = consume_context::<LoopState>();
    let n = (state.count)();
    rsx! {
        view {
            for idx in 0..n {
                view {
                    key: "{idx}",
                    padding: 4.0,
                    opacity: 1.0,
                    text { "Item {idx}" }
                }
            }
            view {
                padding: 6.0,
                opacity: 1.0,
                text { "Footer" }
            }
        }
    }
}

fn first_text_in(store: &NodeStore, root: NodeId) -> Option<String> {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(t) = store.try_row::<TextContent>(id)
            && !t.text().is_empty()
        {
            return Some(t.text().to_owned());
        }
        for &c in store.children(id).iter().rev() {
            stack.push(c);
        }
    }
    None
}

#[test]
fn hot_patch_untagged_element_and_for_loop_instances_with_zero_allocs() {
    // 1. Un-tagged (`test_id: None`) `rsx!` element patched via `(template_key, node_path)` + `RowPatchView`.
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, untagged_card_app);
    surface.reconcile(FrameId(1));

    let root_view = surface.store().children(NodeId::ROOT)[0];
    let alpha = surface.store().children(root_view)[0];
    let beta = surface.store().children(root_view)[1];
    assert_eq!(first_text_in(surface.store(), alpha).as_deref(), Some("Untagged Alpha"));
    assert_eq!(first_text_in(surface.store(), beta).as_deref(), Some("Untagged Beta"));

    let card_key = hot_patch_template_key(file!(), 0);
    let mut rp_a = RowPatch::new();
    rp_a.set(Layout::PADDING_TOP, Length::points(24.0))
        .set(Layout::PADDING_RIGHT, Length::points(24.0))
        .set(Layout::PADDING_BOTTOM, Length::points(24.0))
        .set(Layout::PADDING_LEFT, Length::points(24.0))
        .set(Visual::OPACITY, 0.65)
        .set(Visual::BACKGROUND, Rgba::rgb(37, 99, 235));
    let patch_a = HotPatchMsg::from_bytes(
        &HotPatchMsg::TemplatePatch {
            generation: 2,
            template_key: card_key,
            node_path: vec![0, 0],
            target_test_id: None,
            row_patch_words: encode_patch(&rp_a),
            text_updates: vec![],
            text_replacements: vec![],
        }
        .to_bytes()
        .expect("encode hot-patch v2"),
    )
    .expect("decode hot-patch v2");

    let mut rp_b = RowPatch::new();
    rp_b.set(Layout::PADDING_TOP, Length::points(18.0))
        .set(Layout::PADDING_RIGHT, Length::points(18.0))
        .set(Layout::PADDING_BOTTOM, Length::points(18.0))
        .set(Layout::PADDING_LEFT, Length::points(18.0))
        .set(Visual::OPACITY, 0.85)
        .set(Visual::BACKGROUND, Rgba::rgb(16, 185, 129));
    let patch_b = HotPatchMsg::TemplatePatch {
        generation: 3,
        template_key: card_key,
        node_path: vec![0, 0],
        target_test_id: None,
        row_patch_words: encode_patch(&rp_b),
        text_updates: vec![],
        text_replacements: vec![],
    };

    // Warm once so ChangeSet / scratch capacities settle, then assert strictly 0 heap allocations.
    assert!(surface.apply_hot_patch_now(&patch_a));
    let alpha_l = surface.store().try_row::<Layout>(alpha).unwrap();
    let alpha_v = surface.store().try_row::<Visual>(alpha).unwrap();
    assert_eq!(alpha_l.padding_top(), Length::points(24.0));
    assert!((alpha_v.opacity() - 0.65).abs() < 1e-5);
    assert_eq!(alpha_v.background(), Rgba::rgb(37, 99, 235));
    // Identical sibling remains completely untouched.
    assert_eq!(
        surface.store().try_row::<Layout>(beta).unwrap().padding_top(),
        Length::points(8.0)
    );
    assert!((surface.store().try_row::<Visual>(beta).unwrap().opacity() - 1.0).abs() < 1e-5);

    let (ok, allocs, bytes) = measure_allocs(|| surface.apply_hot_patch_now(&patch_b));
    assert!(ok);
    assert_eq!(
        (allocs, bytes),
        (0, 0),
        "steady-state apply_hot_patch_now on untagged element must perform 0 allocations"
    );
    assert_eq!(
        surface.store().try_row::<Layout>(alpha).unwrap().padding_top(),
        Length::points(18.0)
    );
    assert_eq!(
        surface.store().try_row::<Visual>(alpha).unwrap().background(),
        Rgba::rgb(16, 185, 129)
    );
    assert_eq!(
        surface.store().try_row::<Layout>(beta).unwrap().padding_top(),
        Length::points(8.0)
    );

    // 2. Multi-instance `for`-loop template + sibling after `for` loop (still zero `test_id`s).
    let mut loop_surface = DioxusSurface::new(SurfaceId::ROOT, loop_and_after_app);
    let count_sig = loop_surface.in_root_scope(|| {
        let sig = Signal::new(4usize);
        provide_context(LoopState { count: sig });
        sig
    });
    loop_surface.reconcile(FrameId(1));

    let outer = loop_surface.store().children(NodeId::ROOT)[0];
    let outer_children = loop_surface.store().children(outer).to_vec();
    assert_eq!(outer_children.len(), 5, "4 loop items + 1 footer sibling");
    let footer = outer_children[4];
    assert_eq!(first_text_in(loop_surface.store(), footer).as_deref(), Some("Footer"));

    let outer_key = hot_patch_template_key(file!(), 1);
    let loop_key = hot_patch_template_key(file!(), 2);

    let mut loop_rp1 = RowPatch::new();
    loop_rp1
        .set(Layout::PADDING_TOP, Length::points(20.0))
        .set(Visual::OPACITY, 0.75);
    let loop_patch1 = HotPatchMsg::TemplatePatch {
        generation: 4,
        template_key: loop_key,
        node_path: vec![0],
        target_test_id: None,
        row_patch_words: encode_patch(&loop_rp1),
        text_updates: vec![],
        text_replacements: vec![],
    };

    let mut loop_rp2 = RowPatch::new();
    loop_rp2
        .set(Layout::PADDING_TOP, Length::points(14.0))
        .set(Visual::OPACITY, 0.55);
    let loop_patch2 = HotPatchMsg::TemplatePatch {
        generation: 5,
        template_key: loop_key,
        node_path: vec![0],
        target_test_id: None,
        row_patch_words: encode_patch(&loop_rp2),
        text_updates: vec![],
        text_replacements: vec![],
    };

    let mut footer_rp = RowPatch::new();
    footer_rp.set(Layout::PADDING_TOP, Length::points(32.0));
    let footer_patch = HotPatchMsg::TemplatePatch {
        generation: 6,
        template_key: outer_key,
        node_path: vec![0, 1],
        target_test_id: None,
        row_patch_words: encode_patch(&footer_rp),
        text_updates: vec![],
        text_replacements: vec![],
    };

    // Warm once, verify all 4 loop instances patched while footer stays at 6px.
    assert!(loop_surface.apply_hot_patch_now(&loop_patch1));
    for &item in &outer_children[..4] {
        assert_eq!(
            loop_surface.store().try_row::<Layout>(item).unwrap().padding_top(),
            Length::points(20.0)
        );
        assert!((loop_surface.store().try_row::<Visual>(item).unwrap().opacity() - 0.75).abs() < 1e-5);
    }
    assert_eq!(
        loop_surface.store().try_row::<Layout>(footer).unwrap().padding_top(),
        Length::points(6.0)
    );

    // Steady-state 0-alloc patch across all 4 `for`-loop instances.
    let (ok, allocs, bytes) = measure_allocs(|| loop_surface.apply_hot_patch_now(&loop_patch2));
    assert!(ok);
    assert_eq!(
        (allocs, bytes),
        (0, 0),
        "steady-state apply_hot_patch_now across for-loop instances must perform 0 allocations"
    );
    for &item in &outer_children[..4] {
        assert_eq!(
            loop_surface.store().try_row::<Layout>(item).unwrap().padding_top(),
            Length::points(14.0)
        );
    }

    // Patching the sibling AFTER the `for` loop targets `footer` directly with 0 allocs despite
    // `for` expanding to 4 children in NodeStore!
    let (ok, allocs, bytes) = measure_allocs(|| loop_surface.apply_hot_patch_now(&footer_patch));
    assert!(ok);
    assert_eq!((allocs, bytes), (0, 0));
    assert_eq!(
        loop_surface.store().try_row::<Layout>(footer).unwrap().padding_top(),
        Length::points(32.0)
    );
    for &item in &outer_children[..4] {
        assert_eq!(
            loop_surface.store().try_row::<Layout>(item).unwrap().padding_top(),
            Length::points(14.0)
        );
    }

    // Shrink the `for` loop from 4 -> 2 items: unmounted anchors are evicted cleanly and
    // subsequent patches touch only the 2 surviving items with 0 allocations.
    loop_surface.in_root_scope(|| {
        let mut s = count_sig;
        s.set(2);
    });
    loop_surface.reconcile(FrameId(2));
    let remaining = loop_surface.store().children(outer).to_vec();
    assert_eq!(remaining.len(), 3, "2 surviving loop items + 1 footer");

    let (ok, allocs, bytes) = measure_allocs(|| loop_surface.apply_hot_patch_now(&loop_patch1));
    assert!(ok);
    assert_eq!((allocs, bytes), (0, 0));
    for &item in &remaining[..2] {
        assert_eq!(
            loop_surface.store().try_row::<Layout>(item).unwrap().padding_top(),
            Length::points(20.0)
        );
    }
}
