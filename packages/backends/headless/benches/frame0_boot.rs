//! Criterion benchmark gating Frame-0 `NVE2` keyframe boot + live `TaffyStoreLayout`
//! on a realistic 1,000-node UI tree (`< 2 ms` budget; measures cold replica construction +
//! page-cached file open/read + XXH64/footer validation + NVE2 decode + live Taffy layout).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_headless::HeadlessBackend;
    use nve_components::tables::{
        core_realization_order, core_tables_schema_hash, FlexDirection, Interaction, Layout,
        TextContent, Typography, Visual,
    };
    use nve_engine::{Frame0File, Frame0Meta, StoreBackend, StoreWireReplica, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

    /// Builds a realistic 1,000-node multi-section workspace tree (root + shell + sidebar +
    /// content + 248 rows each with container + title + badge + action button = 1,000 nodes).
    fn build_realistic_1k_tree() -> NodeStore {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        tx.edit::<Layout>(NodeId::ROOT)
            .width(Length::points(1280.0))
            .height(Length::points(800.0))
            .flex_direction(FlexDirection::Row);

        let sidebar = tx.spawn().id();
        tx.edit::<Layout>(sidebar)
            .width(Length::points(240.0))
            .height(Length::percent(100.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Visual>(sidebar)
            .background(Rgba::rgb(15, 23, 42))
            .opacity(1.0);

        let mut nav_items = Vec::with_capacity(5);
        for _ in 0..5 {
            let mut nav = tx.spawn();
            nav.edit::<Layout>()
                .width(Length::percent(100.0))
                .height(Length::points(36.0));
            nav.edit::<Visual>()
                .background(Rgba::rgb(30, 41, 59))
                .border_radius(6.0)
                .opacity(1.0);
            nav.edit::<Typography>()
                .font_size(14.0)
                .color(Rgba::rgb(241, 245, 249));
            nav.edit::<TextContent>().text("Workspace");
            nav.edit::<Interaction>().event_mask(1);
            nav_items.push(nav.id());
        }
        tx.set_children(sidebar, &nav_items);

        let main = tx.spawn().id();
        tx.edit::<Layout>(main)
            .flex_grow(1.0)
            .height(Length::percent(100.0))
            .flex_direction(FlexDirection::Column);
        tx.edit::<Visual>(main)
            .background(Rgba::rgb(17, 24, 39))
            .opacity(1.0);

        // 248 rows x 4 nodes = 992 nodes + sidebar(1) + nav(5) + main(1) + root(1) = 1,000 nodes.
        let mut rows = Vec::with_capacity(248);
        for i in 0..248u32 {
            let row = tx.spawn().id();
            tx.edit::<Layout>(row)
                .width(Length::percent(100.0))
                .height(Length::points(40.0))
                .flex_direction(FlexDirection::Row);
            tx.edit::<Visual>(row)
                .background(Rgba::rgb(30, 41, 59))
                .border_radius(8.0)
                .opacity(1.0);

            let title = tx.spawn().id();
            tx.edit::<Layout>(title)
                .flex_grow(1.0)
                .height(Length::points(24.0));
            tx.edit::<Typography>(title)
                .font_size(14.0)
                .color(Rgba::rgb(248, 250, 252));
            tx.edit::<TextContent>(title).text("Task Item");

            let badge = tx.spawn().id();
            tx.edit::<Layout>(badge)
                .width(Length::points(64.0))
                .height(Length::points(22.0));
            tx.edit::<Visual>(badge)
                .background(Rgba::rgb((i & 255) as u8, 99, 235))
                .border_radius(11.0)
                .opacity(1.0);
            tx.edit::<TextContent>(badge).text("Open");

            let action = tx.spawn().id();
            tx.edit::<Layout>(action)
                .width(Length::points(72.0))
                .height(Length::points(28.0));
            tx.edit::<Visual>(action)
                .background(Rgba::rgb(51, 65, 85))
                .border_radius(6.0)
                .opacity(1.0);
            tx.edit::<Interaction>(action).event_mask(1);
            tx.edit::<TextContent>(action).text("Edit");

            tx.set_children(row, &[title, badge, action]);
            rows.push(row);
        }
        tx.set_children(main, &rows);
        tx.set_children(NodeId::ROOT, &[sidebar, main]);
        tx.commit_into(&mut ChangeSet::default());
        assert_eq!(store.stats().live_nodes, 1_000);
        store
    }

    fn bench_frame0_boot(c: &mut Criterion) {
        let store = build_realistic_1k_tree();
        let meta = Frame0Meta {
            table_schema_hash: core_tables_schema_hash(),
            app_build_id: 0x0001_0002_0003_0004,
            env_hash: 0x0005_0006_0007_0008,
        };
        let path = std::env::temp_dir().join(format!(
            "nve_bench_frame0_1k_{}.nve2",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Frame0File::save(&path, &store, meta).expect("save bench frame0.nve2");
        let raw_bytes = std::fs::read(&path).expect("read bench frame0.nve2");

        let vp = Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 2.0,
        };

        c.bench_function("frame0_boot_1k", |b| {
            let mut scratch = Vec::with_capacity(raw_bytes.len() + 64);
            b.iter(|| {
                let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
                backend.set_viewport(1, vp);
                let mut replica = StoreWireReplica::new(backend, core_realization_order());
                let (footer, _) = Frame0File::boot_into_replica_with_buf(
                    black_box(&path),
                    black_box(meta),
                    &mut scratch,
                    &mut replica,
                )
                .expect("bench frame0 boot");
                black_box((
                    footer.node_count,
                    replica.backend().layout_of(NodeId::ROOT.raw()),
                ))
            });
        });

        c.bench_function("frame0_boot_bytes_1k", |b| {
            b.iter(|| {
                let mut backend = HeadlessBackend::new().with_mutation_tracking(false);
                backend.set_viewport(1, vp);
                let mut replica = StoreWireReplica::new(backend, core_realization_order());
                let (footer, _) = Frame0File::boot_bytes_into_replica(
                    black_box(&raw_bytes),
                    black_box(meta),
                    &mut replica,
                )
                .expect("bench frame0 bytes boot");
                black_box((
                    footer.node_count,
                    replica.backend().layout_of(NodeId::ROOT.raw()),
                ))
            });
        });

        let _ = std::fs::remove_file(&path);
    }

    criterion_group!(benches, bench_frame0_boot);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
