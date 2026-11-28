//! `examples/kitchen-sink` parity between [`YogaLayoutEngine`] (real React Native Yoga C++20) and
//! [`TaffyStoreLayout`]: desktop + mobile viewports, responsive `show_on` hiding, and live layout
//! engine swaps. Skips without `RN_REACT_COMMON_DIR` (fails with `NVE_REQUIRE_YOGA=1`).

#[path = "support/rect_parity.rs"]
mod rect_parity;
#[path = "support/require_yoga.rs"]
mod require_yoga;

use std::collections::HashSet;

use nve_abi::{FrameId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_engine::{StoreBackend, Viewport};
use nve_layout::{DynLayoutEngine, StoreLayoutEngine, TaffyStoreLayout};
use nve_layout_yoga::YogaLayoutEngine;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_tables::tables::{Display, Layout, TextContent};
use rect_parity::{PARITY_TOLERANCE_PX, assert_rect_parity};
use require_yoga::require_yoga_or_skip;

fn collect_subtree(store: &NodeStore, root: NodeId) -> HashSet<NodeId> {
    let mut out = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if out.insert(id) {
            stack.extend_from_slice(store.children(id));
        }
    }
    out
}

fn subtree_contains_text(store: &NodeStore, root: NodeId, needle: &str) -> bool {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if store
            .try_row::<TextContent>(id)
            .is_some_and(|t| t.text().contains(needle))
        {
            return true;
        }
        stack.extend_from_slice(store.children(id));
    }
    false
}

fn live_node_ids(store: &NodeStore) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend_from_slice(store.children(id));
    }
    out
}

#[test]
fn parity_examples_kitchen_sink_nodestore_and_live_swap() {
    if !require_yoga_or_skip() {
        return;
    }

    let mut surface = nve_example_kitchen_sink::create_surface(SurfaceId::ROOT);
    surface.reconcile(FrameId(1));
    let store = surface.store();
    let changes = surface.last_changes();

    // Kitchen Sink mounts its 248px desktop sidebar (`ShowOn(min_width(900))`); lay out at
    // desktop width so all 346 nodes (toolbar, sidebar, badges, scrollview, cards) match reality.
    let viewport = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 2.0,
    };

    let mut taffy_backend = HeadlessBackend::new();
    taffy_backend.set_viewport(SurfaceId::ROOT, viewport);
    taffy_backend.commit(store, changes).unwrap();

    let mut yoga_backend =
        HeadlessBackend::with_layout(YogaLayoutEngine::new().expect("real RN Yoga available"));
    yoga_backend.set_viewport(SurfaceId::ROOT, viewport);
    yoga_backend.commit(store, changes).unwrap();

    let mut node_ids = live_node_ids(store);
    node_ids.sort_unstable();
    assert!(
        node_ids.len() >= 346,
        "expected full kitchen-sink UI tree (>= 346 nodes), found {} nodes",
        node_ids.len()
    );

    // Desktop 1280x800 @ 2x across all >= 346 kitchen-sink nodes (with zero app-code workarounds):
    // Yoga (`YGErrataNone`) omits CSS Flexbox §4.5 `min-width: auto = min-content` on overflowing
    // row items, causing at most a 1.5px shrink-ratio split strictly inside the single
    // `SidebarNavItem` row containing `"Motion, Graphics & Resilience"` + `Badge("4")`.
    // Every node outside that pinned row subtree MUST stay within `<= 1.0px`.
    let motion_text_id = node_ids
        .iter()
        .copied()
        .find(|&id| {
            store
                .try_row::<TextContent>(id)
                .is_some_and(|t| t.text().contains("Motion, Graphics & Resilience"))
        })
        .expect("expected SidebarNavItem text node in kitchen-sink");
    let mut sidebar_row_root = motion_text_id;
    while let Some(parent) = store.parent(sidebar_row_root) {
        sidebar_row_root = parent;
        if subtree_contains_text(store, sidebar_row_root, "4") {
            break;
        }
    }
    let pinned_sidebar_subtree = collect_subtree(store, sidebar_row_root);
    assert!(
        pinned_sidebar_subtree.len() >= 3 && pinned_sidebar_subtree.len() <= 10,
        "expected compact SidebarNavItem subtree, got {} nodes",
        pinned_sidebar_subtree.len()
    );

    let mut desktop_over_1px = 0usize;
    for &id in &node_ids {
        let tr = taffy_backend.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        let max_d = (tr.x - yr.x)
            .abs()
            .max((tr.y - yr.y).abs())
            .max((tr.width - yr.width).abs())
            .max((tr.height - yr.height).abs());
        if max_d > PARITY_TOLERANCE_PX {
            desktop_over_1px += 1;
            assert!(
                pinned_sidebar_subtree.contains(&id),
                "[examples/kitchen-sink desktop] unexpected node #{} outside pinned SidebarNavItem subtree diverged by {max_d}px:\n  taffy: {tr:?}\n  yoga : {yr:?}",
                id.raw()
            );
            assert!(
                max_d <= 2.0,
                "[examples/kitchen-sink desktop] SidebarNavItem node #{} diverged by {max_d}px (> 2px CSS §4.5 bound):\n  taffy: {tr:?}\n  yoga : {yr:?}",
                id.raw()
            );
        }
    }
    assert!(
        desktop_over_1px <= pinned_sidebar_subtree.len(),
        "unexpected desktop divergences: {desktop_over_1px}"
    );

    // Mobile 393x852 @ 3x raw (unfiltered sidebar): verifies zero collapses/NaNs and bounded
    // CSS §9.7 overflow-row differences across x, y, width, and height under tight 71px rows.
    let mobile_vp = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
    };
    let empty = ChangeSet::default();
    taffy_backend.set_viewport(SurfaceId::ROOT, mobile_vp);
    yoga_backend.set_viewport(SurfaceId::ROOT, mobile_vp);
    taffy_backend.commit(store, &empty).unwrap();
    yoga_backend.commit(store, &empty).unwrap();
    let mut mobile_raw_x_over_1px = 0usize;
    let mut mobile_raw_width_over_1px = 0usize;
    let mut mobile_raw_height_over_1px = 0usize;
    for &id in &node_ids {
        let tr = taffy_backend.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert!(
            yr.x.is_finite() && yr.y.is_finite() && yr.width.is_finite() && yr.height.is_finite(),
            "mobile raw node #{} produced non-finite rect: {yr:?}",
            id.raw()
        );
        let dx = (tr.x - yr.x).abs();
        let dy = (tr.y - yr.y).abs();
        let dw = (tr.width - yr.width).abs();
        let dh = (tr.height - yr.height).abs();
        if dx > PARITY_TOLERANCE_PX {
            mobile_raw_x_over_1px += 1;
            assert!(
                dx <= 5.0,
                "mobile raw node #{} x diverged beyond 5px CSS §9.7 bound: taffy={tr:?} yoga={yr:?}",
                id.raw()
            );
        }
        if dw > PARITY_TOLERANCE_PX {
            mobile_raw_width_over_1px += 1;
            assert!(
                dw <= 5.0,
                "mobile raw node #{} width diverged beyond 5px CSS §9.7 bound: taffy={tr:?} yoga={yr:?}",
                id.raw()
            );
        }
        if dh > PARITY_TOLERANCE_PX {
            mobile_raw_height_over_1px += 1;
            assert!(
                dh <= 22.0,
                "mobile raw node #{} height diverged beyond 1 wrapped line: taffy={tr:?} yoga={yr:?}",
                id.raw()
            );
        }
        assert!(
            dy <= 22.0,
            "mobile raw node #{} y diverged beyond 1 wrapped line shift: taffy={tr:?} yoga={yr:?}",
            id.raw()
        );
    }
    assert!(
        mobile_raw_x_over_1px <= 16,
        "unexpected mobile raw x divergences: {mobile_raw_x_over_1px}"
    );
    assert!(
        mobile_raw_width_over_1px <= 10,
        "unexpected mobile raw width divergences: {mobile_raw_width_over_1px}"
    );
    assert!(
        mobile_raw_height_over_1px <= 12,
        "unexpected mobile raw height divergences: {mobile_raw_height_over_1px}"
    );

    // Restore desktop viewport for live engine swap parity checks below.
    taffy_backend.set_viewport(SurfaceId::ROOT, viewport);
    yoga_backend.set_viewport(SurfaceId::ROOT, viewport);
    taffy_backend.commit(store, &empty).unwrap();
    yoga_backend.commit(store, &empty).unwrap();

    // Live swap Taffy -> Yoga -> Box<dyn StoreLayoutEngine> -> Taffy with EMPTY/incremental
    // ChangeSet (verifying full-tree seeding even without caller-synthesized `changes.created`!).
    let mut swapped_to_yoga: HeadlessBackend<YogaLayoutEngine> =
        taffy_backend.swap_layout_engine(YogaLayoutEngine::new().unwrap());
    let mut incremental_root_touch = ChangeSet::default();
    incremental_root_touch.touched.push(NodeId::ROOT);
    incremental_root_touch.effects.push(nve_store::effect::LAYOUT);
    swapped_to_yoga
        .commit(store, &incremental_root_touch)
        .unwrap();
    assert_eq!(swapped_to_yoga.layout_engine().name(), "yoga");
    for &id in &node_ids {
        let sr = swapped_to_yoga.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("kitchen_sink_after_incremental_swap_to_yoga", id, sr, yr);
    }

    let boxed_yoga: DynLayoutEngine = Box::new(YogaLayoutEngine::new().unwrap());
    let mut swapped_to_dyn: HeadlessBackend<DynLayoutEngine> =
        swapped_to_yoga.swap_layout_engine(boxed_yoga);
    swapped_to_dyn.commit(store, &empty).unwrap();
    assert_eq!(swapped_to_dyn.layout_engine().name(), "yoga");
    for &id in &node_ids {
        let dr = swapped_to_dyn.layout_of(id).unwrap();
        let yr = yoga_backend.layout_of(id).unwrap();
        assert_rect_parity("kitchen_sink_after_empty_swap_to_dyn_yoga", id, dr, yr);
    }

    let mut swapped_back_taffy: HeadlessBackend<TaffyStoreLayout> =
        swapped_to_dyn.swap_layout_engine(TaffyStoreLayout::new());
    swapped_back_taffy.commit(store, &empty).unwrap();
    let mut fresh_taffy = HeadlessBackend::new();
    fresh_taffy.set_viewport(SurfaceId::ROOT, viewport);
    fresh_taffy.commit(store, changes).unwrap();
    for &id in &node_ids {
        let tr = swapped_back_taffy.layout_of(id).unwrap();
        let fr = fresh_taffy.layout_of(id).unwrap();
        assert_rect_parity("kitchen_sink_after_swap_back_to_taffy", id, tr, fr);
    }

    let mut mobile_surface = nve_example_kitchen_sink::create_surface(SurfaceId::ROOT);
    mobile_surface.reconcile(nve_abi::FrameId(1));
    let mut hide_changes = ChangeSet::default();
    let desktop_only: Vec<NodeId> = node_ids
        .iter()
        .copied()
        .filter(|&id| {
            mobile_surface
                .node_show_on(id)
                .is_some_and(|mq| mq.min_width_opt().is_some_and(|min_w| (min_w as f32) > mobile_vp.width))
        })
        .collect();
    assert!(!desktop_only.is_empty(), "expected desktop-only ShowOn nodes in kitchen-sink");
    {
        let mut tx = mobile_surface.store_mut().tx();
        for id in desktop_only {
            tx.edit::<Layout>(id).display(Display::None);
        }
        tx.commit_into(&mut hide_changes);
    }
    let m_store = mobile_surface.store();
    let mut m_full = ChangeSet::default();
    m_full.created = live_node_ids(m_store);
    let mut m_taffy = HeadlessBackend::new();
    let mut m_yoga = HeadlessBackend::with_layout(YogaLayoutEngine::new().unwrap());
    m_taffy.set_viewport(SurfaceId::ROOT, mobile_vp);
    m_yoga.set_viewport(SurfaceId::ROOT, mobile_vp);
    m_taffy.commit(m_store, &m_full).unwrap();
    m_yoga.commit(m_store, &m_full).unwrap();
    // Mobile 393x852 @ 3x with responsive `ShowOn(min_width(900))` hidden (real mobile shell):
    // every node outside overflowing flex-shrink rows (CSS §9.7) must match within `<= 1.0px`.
    let mut mobile_resp_over_1px = 0usize;
    for &id in &m_full.created {
        let tr = m_taffy.layout_of(id).unwrap();
        let yr = m_yoga.layout_of(id).unwrap();
        let max_d = (tr.x - yr.x)
            .abs()
            .max((tr.y - yr.y).abs())
            .max((tr.width - yr.width).abs())
            .max((tr.height - yr.height).abs());
        if max_d > PARITY_TOLERANCE_PX {
            mobile_resp_over_1px += 1;
            assert!(
                (tr.y - yr.y).abs() <= PARITY_TOLERANCE_PX
                    && (tr.height - yr.height).abs() <= PARITY_TOLERANCE_PX
                    && max_d <= 6.0,
                "[examples/kitchen-sink mobile responsive] node #{} diverged beyond CSS §9.7 horizontal shrink bound ({max_d}px):\n  taffy: {tr:?}\n  yoga : {yr:?}",
                id.raw()
            );
        }
    }
    assert!(
        mobile_resp_over_1px <= 16,
        "unexpected mobile responsive nodes > 1px: {mobile_resp_over_1px}"
    );
}
