//! Criterion benchmarks for `nve-backend-headless`: 256-node `HeadlessBackend::commit`
//! (Taffy flexbox layout + `NveSceneView` publication) and coordinate `hit_test`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_headless::HeadlessBackend;
    use nve_components::tables::{Layout, TextContent, Visual};
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

    fn bench_headless_commit_and_hit_test(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let ids: Vec<NodeId> = {
            let mut tx = store.tx();
            let nodes: Vec<NodeId> = (0..256)
                .map(|i| {
                    let mut nm = tx.spawn();
                    nm.edit::<Layout>()
                        .width(Length::points(240.0))
                        .height(Length::points(32.0));
                    nm.edit::<Visual>()
                        .background(Rgba::rgb(30, 41, 59))
                        .opacity(1.0);
                    if i % 4 == 0 {
                        nm.edit::<TextContent>().text("Headless Item");
                    }
                    nm.id()
                })
                .collect();
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
            nodes
        };

        let mut backend = HeadlessBackend::new();
        backend.set_viewport(
            1,
            Viewport {
                width: 1280.0,
                height: 720.0,
                scale_factor: 2.0,
            },
        );
        backend
            .commit(&store, &changes)
            .expect("initial headless bench commit");

        let mut tick = 0u32;
        c.bench_function("headless_incremental_visual_commit_256_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                for &id in &ids[..16] {
                    tx.edit::<Visual>(id)
                        .opacity(((tick & 255) as f32) / 255.0);
                }
                tx.commit_into(&mut changes);
                let stats = backend
                    .commit(&store, &changes)
                    .expect("incremental headless commit");
                black_box(stats.mutation_count)
            });
        });

        c.bench_function("headless_hit_test_256_nodes", |b| {
            b.iter(|| black_box(backend.hit_test(&store, black_box(120.0), black_box(320.0))));
        });
    }

    criterion_group!(benches, bench_headless_commit_and_hit_test);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
