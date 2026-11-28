//! Criterion benchmarks for Slice 4C retained `CompositorScene` 120 Hz fast paths (scroll,
//! sticky scroll, opacity+transform commit, single-node paint, full layout sync, deep >64-depth
//! replay, and DisplayList materialization / software replay) on a 1,000-node tree.

#[cfg(not(target_arch = "wasm32"))]
mod native_bench {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{ColorRgba, SurfaceId};
    use nve_engine::{StoreBackend, Viewport};
    use nve_tables::tables::{
        FlexDirection, Layout, Overflow, Position, Scroll, Transform, Visual,
    };
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

    use nve_backend_skia::raster::{rasterize_display_list, PixelBuffer};
    use nve_backend_skia::SkiaBackend;

    fn build_1000_node_tree(
        with_sticky: bool,
    ) -> (NodeStore, ChangeSet, SkiaBackend, NodeId, Vec<NodeId>) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut backend = SkiaBackend::new();
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 320.0,
                height: 240.0,
                scale_factor: 1.0,
            },
        );

        let (scroll, cards) = {
            let mut tx = store.tx();
            tx.edit::<Layout>(NodeId::ROOT)
                .width(Length::points(320.0))
                .height(Length::points(240.0));
            let scroll = tx.create();
            tx.edit::<Layout>(scroll)
                .width(Length::points(320.0))
                .height(Length::points(240.0))
                .flex_direction(FlexDirection::Column);
            tx.edit::<Scroll>(scroll);
            tx.edit::<Visual>(scroll).overflow_y(Overflow::Scroll);

            let mut children = Vec::with_capacity(1001);
            if with_sticky {
                let sticky = tx.create();
                tx.edit::<Layout>(sticky)
                    .position(Position::Sticky)
                    .inset_top(Length::points(0.0))
                    .width(Length::points(300.0))
                    .height(Length::points(22.0))
                    .flex_shrink(0.0);
                tx.edit::<Visual>(sticky)
                    .background(Rgba::rgb(30, 58, 138))
                    .z_index(10);
                children.push(sticky);
            }

            let mut cards = Vec::with_capacity(1000);
            for i in 0..1000usize {
                let c = tx.create();
                tx.edit::<Layout>(c)
                    .width(Length::points(300.0))
                    .height(Length::points(20.0))
                    .flex_shrink(0.0);
                tx.edit::<Visual>(c)
                    .background(Rgba::rgb((i & 255) as u8, 90, 150))
                    .border_radius(4.0);
                children.push(c);
                cards.push(c);
            }
            tx.set_children(scroll, &children);
            tx.set_children(NodeId::ROOT, &[scroll]);
            tx.commit_into(&mut changes);
            (scroll, cards)
        };
        backend.commit(&store, &changes).expect("initial 1k commit");
        assert!(
            backend
                .scroll_content_size(&store, scroll)
                .is_some_and(|(_, ch)| ch > 10_000.0),
            "1k scroll benchmark must have real scrollable overflow (> 10,000px)"
        );
        (store, changes, backend, scroll, cards)
    }

    fn build_deep_80_tree() -> (NodeStore, ChangeSet, SkiaBackend) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let mut backend = SkiaBackend::new();
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 320.0,
                height: 240.0,
                scale_factor: 1.0,
            },
        );
        {
            let mut tx = store.tx();
            tx.edit::<Layout>(NodeId::ROOT)
                .width(Length::points(320.0))
                .height(Length::points(240.0));
            let mut parent = NodeId::ROOT;
            for depth in 0..80usize {
                let node = tx.create();
                tx.edit::<Layout>(node)
                    .width(Length::points((300 - depth * 2).max(16) as f32))
                    .height(Length::points((220 - depth * 2).max(16) as f32));
                if depth.is_multiple_of(10) {
                    tx.edit::<Visual>(node)
                        .background(Rgba::rgb(40 + depth as u8, 80, 140))
                        .opacity(0.95);
                }
                tx.set_children(parent, &[node]);
                parent = node;
            }
            tx.commit_into(&mut changes);
        }
        backend.commit(&store, &changes).expect("deep 80 commit");
        (store, changes, backend)
    }

    pub fn bench_compositor(c: &mut Criterion) {
        let (mut store, mut changes, mut backend, scroll, cards) = build_1000_node_tree(false);

        // 1. O(1) 120 Hz scroll fast path vs scroll + flat DisplayList materialization on 1,000 nodes.
        let mut sy = 0.0f32;
        c.bench_function("compositor_scroll_1000_nodes_fast_path", |b| {
            b.iter(|| {
                sy = (sy + 16.0) % 10_000.0;
                black_box(backend.set_scroll_offset(&store, scroll, 0.0, black_box(sy)))
            });
        });

        c.bench_function("display_list_scroll_1000_nodes_materialize", |b| {
            b.iter(|| {
                sy = (sy + 16.0) % 10_000.0;
                backend.set_scroll_offset(&store, scroll, 0.0, black_box(sy));
                black_box(backend.materialize_display_list().ops.len())
            });
        });

        // 1b. 120 Hz scroll with sticky header on 1,000 nodes.
        let (sticky_store, _sticky_changes, mut sticky_backend, sticky_scroll, _sticky_cards) =
            build_1000_node_tree(true);
        let mut sticky_sy = 0.0f32;
        c.bench_function("compositor_scroll_1000_nodes_with_sticky_fast_path", |b| {
            b.iter(|| {
                sticky_sy = (sticky_sy + 16.0) % 10_000.0;
                black_box(sticky_backend.set_scroll_offset(
                    &sticky_store,
                    sticky_scroll,
                    0.0,
                    black_box(sticky_sy),
                ))
            });
        });

        // 2. Single-node COMPOSITE-only opacity+transform commit on 1,000 nodes.
        let target = cards[0];
        let mut tick = 0u32;
        c.bench_function("compositor_opacity_transform_commit_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Visual>(target)
                    .opacity(0.5 + ((tick & 63) as f32) * 0.005);
                tx.edit::<Transform>(target)
                    .translate_x(((tick & 63) as f32) * 0.5);
                tx.commit_into(&mut changes);
                backend
                    .commit(&store, &changes)
                    .expect("composite commit");
                black_box(backend.scene().stats().composite_fast_paths)
            });
        });

        // 3. Single-node PAINT-only background edit on 1,000 nodes.
        c.bench_function("compositor_single_node_paint_commit_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Visual>(target)
                    .background(Rgba::rgb((tick & 255) as u8, 100, 180));
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("paint commit");
                black_box(backend.scene().live_op_count())
            });
        });

        // 4. Full LAYOUT sync commit on 1,000 nodes (width toggle on 1 card, preserving unchanged ops).
        c.bench_function("compositor_layout_commit_1000_nodes", |b| {
            b.iter(|| {
                tick = tick.wrapping_add(1);
                let mut tx = store.tx();
                tx.edit::<Layout>(target)
                    .width(Length::points(if tick.is_multiple_of(2) { 296.0 } else { 300.0 }));
                tx.commit_into(&mut changes);
                backend.commit(&store, &changes).expect("layout commit");
                black_box(backend.scene().stats().layout_tree_syncs)
            });
        });

        // Reset opacity/transform to opaque identity for clean replay comparison.
        {
            let mut tx = store.tx();
            tx.edit::<Visual>(target).opacity(1.0);
            tx.edit::<Transform>(target).translate_x(0.0);
            tx.commit_into(&mut changes);
            backend.commit(&store, &changes).expect("reset target");
        }
        backend.set_scroll_offset(&store, scroll, 0.0, 200.0);
        let dl = backend.materialize_display_list().clone();
        let mut fb = PixelBuffer::new(320, 240);

        c.bench_function("compositor_software_replay_1000_nodes_culled", |b| {
            b.iter(|| {
                backend.rasterize(black_box(&mut fb), ColorRgba::BLACK);
                black_box(fb.pixels().len())
            });
        });

        c.bench_function("display_list_software_replay_1000_nodes", |b| {
            b.iter(|| {
                rasterize_display_list(black_box(&dl), &mut fb, 1.0, ColorRgba::BLACK);
                black_box(fb.pixels().len())
            });
        });

        // 5. >64-depth nested tree replay (exercises ReplayStack overflow Vec path cleanly).
        let (_deep_store, _deep_changes, deep_backend) = build_deep_80_tree();
        c.bench_function("compositor_deep_80_depth_software_replay", |b| {
            b.iter(|| {
                deep_backend.rasterize(black_box(&mut fb), ColorRgba::BLACK);
                black_box(fb.pixels().len())
            });
        });
    }

    criterion_group!(benches, bench_compositor);

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
