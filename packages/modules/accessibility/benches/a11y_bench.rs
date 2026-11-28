//! Criterion benchmarks for `nve-accessibility` AccessKit projection & incremental diffing.

#[cfg(not(target_arch = "wasm32"))]
mod native_bench {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::InteractionEvent;
    use nve_accessibility::{A11yGeometryProvider, AccessKitTreeBridge};
    use nve_engine::LayoutRect;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_tables::tables::{Accessibility, AriaRole, Interaction, Transform, Visual};

    struct UniformGeo;
    impl A11yGeometryProvider for UniformGeo {
        fn layout_rect(&self, id: NodeId) -> Option<LayoutRect> {
            Some(LayoutRect {
                x: 0.0,
                y: (id.index() as f32) * 24.0,
                width: 300.0,
                height: 22.0,
            })
        }
    }

    fn build_1000_node_store() -> (NodeStore, ChangeSet, Vec<NodeId>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let nodes = {
            let mut tx = store.tx();
            let mut list = Vec::with_capacity(1000);
            for i in 0..1000usize {
                let n = tx.create();
                tx.edit::<Accessibility>(n)
                    .role(Some(AriaRole::Button))
                    .label(&format!("Item {i}"));
                tx.edit::<Interaction>(n).listen(InteractionEvent::Press);
                list.push(n);
            }
            tx.set_children(NodeId::ROOT, &list);
            tx.commit_into(&mut changes);
            list
        };
        (store, changes, nodes)
    }

    pub fn bench_a11y(c: &mut Criterion) {
        let (mut store, mut changes, nodes) = build_1000_node_store();
        let geo = UniformGeo;

        c.bench_function("a11y_full_tree_1000_nodes", |b| {
            let mut bridge = AccessKitTreeBridge::new();
            bridge.activate();
            b.iter(|| {
                let upd = bridge.build_full_tree_update(black_box(&store), &geo, "Bench");
                black_box(upd.nodes.len())
            });
        });

        let mut bridge = AccessKitTreeBridge::new();
        bridge.activate();
        let _ = bridge.build_full_tree_update(&store, &geo, "Bench");

        let target = nodes[500];
        let mut tick = 0u32;
        c.bench_function("a11y_incremental_single_label_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Accessibility>(target)
                    .label(if tick.is_multiple_of(2) { "Item A" } else { "Item B" });
                tx.commit_into(&mut changes);
                let upd = bridge.sync_changes(&store, &changes, &geo, "Bench", false);
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });

        c.bench_function("a11y_noop_opacity_composite_frame_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Visual>(target)
                    .opacity(0.5 + ((tick & 63) as f32) * 0.005);
                tx.commit_into(&mut changes);
                let upd = bridge.sync_changes(&store, &changes, &geo, "Bench", false);
                black_box(upd.is_some())
            });
        });

        c.bench_function("a11y_incremental_single_transform_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Transform>(target)
                    .translate_x(((tick & 63) as f32) * 0.5);
                tx.commit_into(&mut changes);
                let upd = bridge.sync_changes(&store, &changes, &geo, "Bench", false);
                black_box(upd.map_or(0, |u| u.nodes.len()))
            });
        });
    }

    criterion_group!(benches, bench_a11y);

    pub fn run() {
        benches();
        criterion::Criterion::default()
            .configure_from_args()
            .final_summary();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native_bench::run();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
