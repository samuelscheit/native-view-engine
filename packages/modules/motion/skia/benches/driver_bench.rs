//! Criterion benchmarks for `MotionDriver`: per-frame `advance` over many animating nodes and
//! the commit-time sync cost when one node out of many changes.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, BatchSize, Criterion};
    use nve_abi::ColorRgba;
    use nve_backend_skia::{NodeTargets, SkiaFrameHook};
    use nve_motion::{write_transitions, EasingCurve, TransitionSpec};
    use nve_motion_skia::MotionDriver;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_tables::snapshots::VisualSnapshot;
    use nve_tables::transform::Transform2D;

    const NODES: u32 = 512;

    fn targets(bg: u8) -> impl Fn(NodeId) -> NodeTargets {
        move |_| NodeTargets {
            visual: VisualSnapshot { background: ColorRgba::rgb(bg, 0, 0), ..VisualSnapshot::default() },
            transform: Transform2D::IDENTITY,
        }
    }

    fn setup() -> (NodeStore, MotionDriver, Vec<NodeId>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut tx = store.tx();
        let nodes: Vec<NodeId> = (0..NODES).map(|_| tx.create()).collect();
        tx.set_children(NodeId::ROOT, &nodes);
        for &n in &nodes {
            write_transitions(&mut tx, n, &[TransitionSpec::all(10_000).easing(EasingCurve::Linear)]);
        }
        tx.commit_into(&mut changes);
        let mut driver = MotionDriver::new();
        driver.register_tables();
        driver.after_commit(&store, &changes, &targets(0));
        (store, driver, nodes)
    }

    fn bench_driver(c: &mut Criterion) {
        let (store, mut animating, nodes) = setup();
        animating.sync_nodes(&store, &nodes, &targets(255));
        c.bench_function("motion_driver_advance_512_animating_nodes", |b| {
            b.iter_batched_ref(
                || animating.clone(),
                |d| black_box(d.advance(black_box(1.0 / 120.0))),
                BatchSize::SmallInput,
            );
        });

        let (store, idle, nodes) = setup();
        let mut one = ChangeSet::default();
        one.touched.push(nodes[NODES as usize / 2]);
        one.effects.push(0);
        c.bench_function("motion_driver_commit_sync_1_of_512_changed", |b| {
            b.iter_batched_ref(
                || idle.clone(),
                |d| d.after_commit(&store, black_box(&one), &targets(255)),
                BatchSize::SmallInput,
            );
        });
    }

    criterion_group!(benches, bench_driver);
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(native::benches);
