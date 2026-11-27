//! Criterion benchmarks for `nve-layout-yoga` (runs live Yoga C++20 layout when built with
//! `RN_REACT_COMMON_DIR`, otherwise benchmarks the equivalent `TaffyStoreLayout` baseline over
//! the same 128-node `NodeStore` flexbox tree alongside `YogaLayoutEngine::is_available`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_headless::TaffyStoreLayout;
    use nve_components::tables::{Layout, TextContent};
    use nve_engine::{StoreLayoutEngine, Viewport};
    use nve_layout_yoga::YogaLayoutEngine;
    use nve_store::{ChangeSet, Length, NodeId, NodeStore};

    fn build_flex_store(count: usize) -> (NodeStore, ChangeSet, Vec<NodeId>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let ids: Vec<NodeId> = {
            let mut tx = store.tx();
            let nodes: Vec<NodeId> = (0..count)
                .map(|i| {
                    let mut nm = tx.spawn();
                    nm.edit::<Layout>()
                        .width(Length::points(120.0 + (i % 5) as f32 * 16.0))
                        .height(Length::points(36.0));
                    nm.edit::<TextContent>().text("Yoga Flex Item");
                    nm.id()
                })
                .collect();
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
            nodes
        };
        (store, changes, ids)
    }

    fn bench_yoga_or_baseline_layout(c: &mut Criterion) {
        let (mut store, mut changes, ids) = build_flex_store(128);
        let vp = Viewport {
            width: 1280.0,
            height: 720.0,
            scale_factor: 2.0,
        };

        if let Ok(mut yoga) = YogaLayoutEngine::new() {
            yoga.sync_changes(&store, &changes);
            yoga.compute_layout(&store, NodeId::ROOT, vp);
            let mut tick = 0u32;
            c.bench_function("layout_yoga_incremental_128_nodes", |b| {
                b.iter(|| {
                    tick = tick.wrapping_add(1);
                    let mut tx = store.tx();
                    tx.edit::<Layout>(ids[(tick as usize) % ids.len()])
                        .width(Length::points(100.0 + (tick % 64) as f32));
                    tx.commit_into(&mut changes);
                    yoga.sync_changes(&store, &changes);
                    yoga.compute_layout(&store, NodeId::ROOT, vp);
                    black_box(yoga.layout_of(ids[0]))
                });
            });
        } else {
            let mut fallback = TaffyStoreLayout::new();
            fallback.sync_changes(&store, &changes);
            fallback.compute_layout(&store, NodeId::ROOT, vp);
            let mut tick = 0u32;
            c.bench_function("layout_yoga_unavailable_taffy_baseline_128_nodes", |b| {
                b.iter(|| {
                    let avail = black_box(YogaLayoutEngine::is_available());
                    tick = tick.wrapping_add(1);
                    let mut tx = store.tx();
                    tx.edit::<Layout>(ids[(tick as usize) % ids.len()])
                        .width(Length::points(100.0 + (tick % 64) as f32));
                    tx.commit_into(&mut changes);
                    fallback.sync_changes(&store, &changes);
                    fallback.compute_layout(&store, NodeId::ROOT, vp);
                    black_box((avail, fallback.layout_of(ids[0])))
                });
            });
        }
    }

    criterion_group!(benches, bench_yoga_or_baseline_layout);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
