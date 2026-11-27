//! End-to-end Frame-0 `NVE2` + live `TaffyStoreLayout` boot verification:
//! 1. 100% table & layout parity between live rendered `NodeStore` and Frame-0 booted replica.
//! 2. Responsive viewport recomputation (identical `frame0.nve2` booted at mobile vs desktop).
//! 3. `CountingAlloc` O(1)-allocation scaling guard comparing 1,000 vs 5,000 nodes after pre-sizing.

#![cfg(not(target_arch = "wasm32"))]

#[path = "support/counting_alloc.rs"]
mod counting_alloc;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nve_backend_headless::HeadlessBackend;
use nve_components::tables::{
    FlexDirection, Interaction, Layout, TextContent, Typography, Visual,
};
use nve_engine::{Frame0File, Frame0Meta, StoreBackend, StoreWireReplica, Viewport};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, RealizationOrder, Rgba};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_frame0_path(tag: &str) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nve_headless_frame0_{}_{}_{}.nve2",
        std::process::id(),
        seq,
        tag
    ))
}

fn build_responsive_app_tree() -> (NodeStore, ChangeSet, NodeId, NodeId, NodeId) {
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    let (shell, sidebar, main_pane) = {
        let mut tx = store.tx();
        let shell = tx.spawn().id();
        tx.edit::<Layout>(shell)
            .width(Length::percent(100.0))
            .height(Length::percent(100.0))
            .flex_direction(FlexDirection::Row);
        tx.edit::<Visual>(shell)
            .background(Rgba::rgb(15, 23, 42))
            .opacity(1.0);

        let sidebar = tx.spawn().id();
        tx.edit::<Layout>(sidebar)
            .width(Length::percent(25.0))
            .height(Length::percent(100.0));
        tx.edit::<Visual>(sidebar)
            .background(Rgba::rgb(30, 41, 59))
            .opacity(1.0);

        let main_pane = tx.spawn().id();
        tx.edit::<Layout>(main_pane)
            .flex_grow(1.0)
            .height(Length::percent(100.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Visual>(main_pane)
            .background(Rgba::rgb(17, 24, 39))
            .opacity(1.0);

        let mut rows = Vec::with_capacity(24);
        for i in 0..24u32 {
            let mut row = tx.spawn();
            row.edit::<Layout>()
                .width(Length::percent(100.0))
                .height(Length::points(36.0));
            row.edit::<Visual>()
                .background(Rgba::rgb(31, 41, 55))
                .border_radius(6.0)
                .opacity(1.0);
            row.edit::<Typography>()
                .font_size(14.0)
                .color(Rgba::rgb(241, 245, 249));
            row.edit::<TextContent>().text("Workspace Row");
            if i % 3 == 0 {
                row.edit::<Interaction>().event_mask(1);
            }
            rows.push(row.id());
        }
        tx.set_children(main_pane, &rows);
        tx.set_children(shell, &[sidebar, main_pane]);
        tx.set_children(NodeId::ROOT, &[shell]);
        tx.commit_into(&mut changes);
        (shell, sidebar, main_pane)
    };
    (store, changes, shell, sidebar, main_pane)
}

fn build_flat_n_nodes(n: usize) -> NodeStore {
    let mut store = NodeStore::new();
    let mut tx = store.tx();
    tx.edit::<Layout>(NodeId::ROOT)
        .width(Length::points(1280.0))
        .height(Length::points(800.0))
        .flex_direction(FlexDirection::Column);
    let mut kids = Vec::with_capacity(n);
    for i in 0..n {
        let mut nm = tx.spawn();
        nm.edit::<Layout>()
            .width(Length::points(240.0))
            .height(Length::points(28.0));
        nm.edit::<Visual>()
            .background(Rgba::rgb((i & 255) as u8, 41, 59))
            .border_radius(4.0)
            .opacity(1.0);
        if i % 2 == 0 {
            nm.edit::<Typography>()
                .font_size(13.0)
                .color(Rgba::rgb(226, 232, 240));
            nm.edit::<TextContent>().text("Row Item");
        }
        if i % 8 == 0 {
            nm.edit::<Interaction>().event_mask(1);
        }
        kids.push(nm.id());
    }
    tx.set_children(NodeId::ROOT, &kids);
    tx.commit_into(&mut ChangeSet::default());
    store
}

#[test]
fn frame0_boot_table_layout_parity_and_responsive_viewports() {
    let (live_store, live_changes, shell, sidebar, main_pane) = build_responsive_app_tree();
    let desktop = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 2.0,
    };
    let mobile = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
    };

    let mut live_backend = HeadlessBackend::new();
    live_backend.set_viewport(1, desktop);
    live_backend
        .commit(&live_store, &live_changes)
        .expect("live desktop commit");

    let meta = Frame0Meta {
        table_schema_hash: nve_store::registered_tables_schema_hash(),
        app_build_id: 0x0102_0304_0506_0708,
        env_hash: 0x1112_1314_1516_1718,
    };
    let path = temp_frame0_path("parity");
    let _ = std::fs::remove_file(&path);
    Frame0File::save(&path, &live_store, meta).expect("save frame0.nve2");

    // 1. Boot at identical desktop viewport -> 100% table + layout rect parity.
    let mut desktop_backend = HeadlessBackend::new();
    desktop_backend.set_viewport(1, desktop);
    let mut desktop_replica = StoreWireReplica::new(desktop_backend, RealizationOrder::default());
    let (footer, _) =
        Frame0File::boot_into_replica(&path, meta, &mut desktop_replica).expect("boot desktop");
    assert_eq!(footer.node_count as usize, live_store.stats().live_nodes);
    assert_eq!(
        desktop_replica.store().stats().live_nodes,
        live_store.stats().live_nodes
    );

    let booted = desktop_replica.store();
    for id in [NodeId::ROOT, shell, sidebar, main_pane] {
        assert_eq!(booted.children(id), live_store.children(id));
        assert_eq!(
            desktop_replica.backend().layout_of(id.raw()),
            live_backend.layout_of(id.raw()),
            "rect mismatch for node {:?}",
            id
        );
    }
    for &row_id in live_store.children(main_pane) {
        assert_eq!(
            booted.try_row::<TextContent>(row_id).map(|t| t.text()),
            live_store.try_row::<TextContent>(row_id).map(|t| t.text())
        );
        assert_eq!(
            booted.try_row::<Visual>(row_id).map(|v| v.background()),
            live_store.try_row::<Visual>(row_id).map(|v| v.background())
        );
        assert_eq!(
            booted.try_row::<Typography>(row_id).map(|t| t.font_size()),
            live_store
                .try_row::<Typography>(row_id)
                .map(|t| t.font_size())
        );
        assert_eq!(
            booted.try_row::<Interaction>(row_id).map(|i| i.event_mask()),
            live_store
                .try_row::<Interaction>(row_id)
                .map(|i| i.event_mask())
        );
        assert_eq!(
            desktop_replica.backend().layout_of(row_id.raw()),
            live_backend.layout_of(row_id.raw())
        );
    }

    // 2. Boot the EXACT SAME frame0.nve2 file at mobile viewport -> fresh responsive layout!
    let mut mobile_backend = HeadlessBackend::new();
    mobile_backend.set_viewport(1, mobile);
    let mut mobile_replica = StoreWireReplica::new(mobile_backend, RealizationOrder::default());
    Frame0File::boot_into_replica(&path, meta, &mut mobile_replica).expect("boot mobile");

    let desktop_sidebar = desktop_replica
        .backend()
        .layout_of(sidebar.raw())
        .expect("desktop sidebar rect");
    let desktop_main = desktop_replica
        .backend()
        .layout_of(main_pane.raw())
        .expect("desktop main rect");
    let mobile_sidebar = mobile_replica
        .backend()
        .layout_of(sidebar.raw())
        .expect("mobile sidebar rect");
    let mobile_main = mobile_replica
        .backend()
        .layout_of(main_pane.raw())
        .expect("mobile main rect");

    assert!((desktop_sidebar.width - 320.0).abs() <= 1.0);
    assert!((desktop_main.width - 960.0).abs() <= 1.0);
    assert!((mobile_sidebar.width - 98.0).abs() <= 1.5);
    assert!((mobile_main.width - 295.0).abs() <= 1.5);
    assert!((mobile_main.height - 852.0).abs() <= 1.0);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn frame0_boot_counting_alloc_o1_guard_1k_vs_5k() {
    counting_alloc::assert_counts_only_this_thread();

    let store_1k = build_flat_n_nodes(1_000);
    let store_5k = build_flat_n_nodes(5_000);
    let meta = Frame0Meta {
        table_schema_hash: nve_store::registered_tables_schema_hash(),
        app_build_id: 7,
        env_hash: 11,
    };
    let path_1k = temp_frame0_path("alloc_1k");
    let path_5k = temp_frame0_path("alloc_5k");
    let _ = std::fs::remove_file(&path_1k);
    let _ = std::fs::remove_file(&path_5k);
    Frame0File::save(&path_1k, &store_1k, meta).expect("save 1k");
    Frame0File::save(&path_5k, &store_5k, meta).expect("save 5k");

    let measure_boot = |path: &PathBuf| -> (usize, usize) {
        let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
        backend.set_viewport(
            1,
            Viewport {
                width: 1280.0,
                height: 800.0,
                scale_factor: 2.0,
            },
        );
        let mut replica = StoreWireReplica::new(backend, RealizationOrder::default());
        let mut scratch = Vec::with_capacity(512 * 1024);
        let (live, allocs) = counting_alloc::count_allocs(|| {
            let (footer, _) =
                Frame0File::boot_into_replica_with_buf(path, meta, &mut scratch, &mut replica)
                    .expect("boot frame0");
            assert!(replica.backend().layout_of(NodeId::ROOT.raw()).is_some());
            footer.node_count as usize
        });
        (live, allocs)
    };

    let (live_1k, allocs_1k) = measure_boot(&path_1k);
    let (live_5k, allocs_5k) = measure_boot(&path_5k);
    let _ = std::fs::remove_file(&path_1k);
    let _ = std::fs::remove_file(&path_5k);
    eprintln!("[frame0_boot_tests] Full Frame-0 boot (read + validate + NVE2 decode + Taffy layout) allocs: 1k={allocs_1k}, 5k={allocs_5k}");

    assert_eq!(live_1k, 1_001);
    assert_eq!(live_5k, 5_001);
    // Scaling 5x in node count (1k -> 5k) must stay strictly O(1) in heap allocations after
    // pre-sizing (never growing proportionally to node count).
    assert!(
        allocs_5k <= allocs_1k + 16,
        "Frame-0 full boot (read + validate + NVE2 decode + Taffy layout) must be O(1) allocs: 1k={allocs_1k}, 5k={allocs_5k}"
    );
}
