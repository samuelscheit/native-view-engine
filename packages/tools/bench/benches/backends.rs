use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use nve_abi::{ColorRgba, FrameId, SurfaceId};
use nve_backend_headless::{HeadlessBackend, TaffyStoreLayout};
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_bench::{
    build_card_scene_cmd, build_row_tree_cmd, RowFeedFixture, DEFAULT_VIEWPORT,
};
use nve_tables::tables::core_realization_order;
use nve_engine::{StoreBackend, StoreLayoutEngine, StoreWireReplica};
use nve_layout_yoga::YogaLayoutEngine;

fn bench_headless_backend(c: &mut Criterion) {
    let mut layout_group = c.benchmark_group("backend_headless/taffy_layout");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, FrameId(1));
        layout_group.throughput(Throughput::Elements((rows * 2) as u64));
        layout_group.bench_with_input(BenchmarkId::new("initial_rows", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let mut replica =
                        StoreWireReplica::new(HeadlessBackend::new(), core_realization_order());
                    replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    replica
                },
                |mut replica| {
                    black_box(
                        replica
                            .submit_nve2_frame(black_box(bytes))
                            .expect("submit headless bench frame"),
                    )
                },
                BatchSize::SmallInput,
            );
        });
    }
    layout_group.finish();

    let mut hit_group = c.benchmark_group("backend_headless/hit_test");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, FrameId(1));
        let mut replica = StoreWireReplica::new(HeadlessBackend::new(), core_realization_order());
        replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
        replica
            .submit_nve2_frame(&init_bytes)
            .expect("submit hit-test bench frame");

        hit_group.bench_with_input(BenchmarkId::new("rows", rows), &rows, |b, _| {
            let mut y = 10.0f32;
            b.iter(|| {
                y = (y + 37.0) % 800.0;
                black_box(
                    replica
                        .backend()
                        .hit_test(replica.store(), black_box(120.0), black_box(y)),
                )
            });
        });
    }
    hit_group.finish();
}

fn bench_skia_backend(c: &mut Criterion) {
    let mut group = c.benchmark_group("backend_skia");

    // 1. Card scene: display-list build + software rasterization
    let card_cmd = build_card_scene_cmd(FrameId(1));
    group.bench_function("display_list_card", |b| {
        b.iter_batched(
            || {
                let mut replica =
                    StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
                replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                replica
            },
            |mut replica| {
                let stats = replica
                    .submit_nve2_frame(black_box(&card_cmd))
                    .expect("submit skia card frame");
                black_box((stats, replica.backend().display_list().ops.len()))
            },
            BatchSize::SmallInput,
        );
    });

    let mut skia_card = StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
    skia_card.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    skia_card
        .submit_nve2_frame(&card_cmd)
        .expect("submit skia card fixture");
    let mut card_buf = PixelBuffer::new(393, 852);
    group.bench_function("rasterize_card_393x852", |b| {
        b.iter(|| {
            skia_card
                .backend()
                .rasterize(&mut card_buf, ColorRgba::BLACK);
            black_box(card_buf.pixels()[0])
        });
    });

    // 2. 1,000-node tree (500 rows * 2 nodes): display-list build + software rasterization
    let tree_cmd = build_row_tree_cmd(500, FrameId(1));
    group.bench_function("display_list_1000_nodes", |b| {
        b.iter_batched(
            || {
                let mut replica =
                    StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
                replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                replica
            },
            |mut replica| {
                let stats = replica
                    .submit_nve2_frame(black_box(&tree_cmd))
                    .expect("submit skia tree frame");
                black_box((stats, replica.backend().display_list().ops.len()))
            },
            BatchSize::SmallInput,
        );
    });

    let mut skia_tree = StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
    skia_tree.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    skia_tree
        .submit_nve2_frame(&tree_cmd)
        .expect("submit skia tree fixture");
    let mut tree_buf = PixelBuffer::new(393, 852);
    group.bench_function("rasterize_1000_nodes_393x852", |b| {
        b.iter(|| {
            skia_tree
                .backend()
                .rasterize(&mut tree_buf, ColorRgba::BLACK);
            black_box(tree_buf.pixels()[0])
        });
    });

    group.finish();
}

fn bench_web_backend(c: &mut Criterion) {
    let mut group = c.benchmark_group("backend_web");

    let mut feed_100 = RowFeedFixture::new(100);
    let mut frame_buf = Vec::with_capacity(16_384);
    feed_100.encode_initial(&mut frame_buf);
    let mut web_100 = StoreWireReplica::new(WebBackend::new(), core_realization_order());
    web_100.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    web_100
        .submit_nve2_frame(&frame_buf)
        .expect("submit initial web_100 frame");

    let mut frame_id = FrameId(2);
    group.bench_function("incremental_dom_patch_10_nodes", |b| {
        b.iter(|| {
            frame_id = frame_id.next();
            feed_100.encode_patch(frame_id, 10, &mut frame_buf);
            black_box(
                web_100
                    .submit_nve2_frame(black_box(&frame_buf))
                    .expect("submit incremental web patch"),
            )
        });
    });

    group.bench_function("render_html_100_rows", |b| {
        b.iter(|| black_box(web_100.backend().render_html(web_100.store())));
    });

    let init_500 = build_row_tree_cmd(500, FrameId(1));
    let mut web_500 = StoreWireReplica::new(WebBackend::new(), core_realization_order());
    web_500.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    web_500
        .submit_nve2_frame(&init_500)
        .expect("submit initial web_500 frame");

    group.bench_function("render_html_500_rows", |b| {
        b.iter(|| black_box(web_500.backend().render_html(web_500.store())));
    });

    group.finish();
}

fn bench_taffy_vs_yoga(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_engine/taffy_vs_yoga");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, FrameId(1));
        group.throughput(Throughput::Elements((rows * 2) as u64));

        group.bench_with_input(BenchmarkId::new("taffy_static", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let mut replica =
                        StoreWireReplica::new(HeadlessBackend::new(), core_realization_order());
                    replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    replica
                },
                |mut replica| {
                    black_box(
                        replica
                            .submit_nve2_frame(black_box(bytes))
                            .expect("submit taffy_static frame"),
                    )
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("taffy_dyn_box", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let boxed: Box<dyn StoreLayoutEngine> = Box::new(TaffyStoreLayout::new());
                    let mut replica = StoreWireReplica::new(
                        HeadlessBackend::with_layout(boxed),
                        core_realization_order(),
                    );
                    replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    replica
                },
                |mut replica| {
                    black_box(
                        replica
                            .submit_nve2_frame(black_box(bytes))
                            .expect("submit taffy_dyn_box frame"),
                    )
                },
                BatchSize::SmallInput,
            );
        });

        if YogaLayoutEngine::is_available() {
            group.bench_with_input(
                BenchmarkId::new("yoga_rn_static", rows),
                &init_bytes,
                |b, bytes| {
                    b.iter_batched(
                        || {
                            let yoga = YogaLayoutEngine::new().expect("RN Yoga available");
                            let mut replica = StoreWireReplica::new(
                                HeadlessBackend::with_layout(yoga),
                                core_realization_order(),
                            );
                            replica.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                            replica
                        },
                        |mut replica| {
                            black_box(
                                replica
                                    .submit_nve2_frame(black_box(bytes))
                                    .expect("submit yoga bench frame"),
                            )
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
    }
    group.finish();
}

criterion_group!(
    backend_benches,
    bench_headless_backend,
    bench_taffy_vs_yoga,
    bench_skia_backend,
    bench_web_backend
);
criterion_main!(backend_benches);
