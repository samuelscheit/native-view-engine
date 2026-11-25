use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use nve_abi::ColorRgba;
use nve_backend_headless::{HeadlessBackend, TaffyLayoutEngine};
use nve_backend_skia::{PixelBuffer, SkiaBackend};
use nve_backend_web::WebBackend;
use nve_bench::{
    build_card_scene_cmd, build_row_patch_cmd, build_row_tree_cmd, DEFAULT_VIEWPORT,
};
use nve_engine::{DynLayoutEngine, RenderBackend};
use nve_layout_yoga::YogaLayoutEngine;

fn bench_headless_backend(c: &mut Criterion) {
    let mut layout_group = c.benchmark_group("backend_headless/taffy_layout");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, 1);
        layout_group.throughput(Throughput::Elements((rows * 2) as u64));
        layout_group.bench_with_input(BenchmarkId::new("initial_rows", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let mut backend = HeadlessBackend::new();
                    backend.set_viewport(1, DEFAULT_VIEWPORT);
                    backend
                },
                |mut backend| black_box(backend.submit_frame(black_box(bytes)).unwrap()),
                BatchSize::SmallInput,
            );
        });
    }
    layout_group.finish();

    let mut hit_group = c.benchmark_group("backend_headless/hit_test");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, 1);
        let mut backend = HeadlessBackend::new();
        backend.set_viewport(1, DEFAULT_VIEWPORT);
        backend.submit_frame(&init_bytes).unwrap();

        hit_group.bench_with_input(BenchmarkId::new("rows", rows), &rows, |b, _| {
            let mut y = 10.0f32;
            b.iter(|| {
                y = (y + 37.0) % 800.0;
                black_box(backend.hit_test(black_box(120.0), black_box(y), true))
            });
        });
    }
    hit_group.finish();
}

fn bench_skia_backend(c: &mut Criterion) {
    let mut group = c.benchmark_group("backend_skia");

    // 1. Card scene: display-list build + software rasterization
    let card_cmd = build_card_scene_cmd(1);
    group.bench_function("display_list_card", |b| {
        b.iter_batched(
            || {
                let mut skia = SkiaBackend::new();
                skia.set_viewport(1, DEFAULT_VIEWPORT);
                skia
            },
            |mut skia| {
                let stats = skia.submit_frame(black_box(&card_cmd)).unwrap();
                black_box((stats, skia.display_list().ops.len()))
            },
            BatchSize::SmallInput,
        );
    });

    let mut skia_card = SkiaBackend::new();
    skia_card.set_viewport(1, DEFAULT_VIEWPORT);
    skia_card.submit_frame(&card_cmd).unwrap();
    let mut card_buf = PixelBuffer::new(393, 852);
    group.bench_function("rasterize_card_393x852", |b| {
        b.iter(|| {
            skia_card.rasterize_at_scale(&mut card_buf, 1.0, ColorRgba::BLACK);
            black_box(card_buf.pixels()[0])
        });
    });

    // 2. 1,000-node tree (500 rows * 2 nodes): display-list build + software rasterization
    let tree_cmd = build_row_tree_cmd(500, 1);
    group.bench_function("display_list_1000_nodes", |b| {
        b.iter_batched(
            || {
                let mut skia = SkiaBackend::new();
                skia.set_viewport(1, DEFAULT_VIEWPORT);
                skia
            },
            |mut skia| {
                let stats = skia.submit_frame(black_box(&tree_cmd)).unwrap();
                black_box((stats, skia.display_list().ops.len()))
            },
            BatchSize::SmallInput,
        );
    });

    let mut skia_tree = SkiaBackend::new();
    skia_tree.set_viewport(1, DEFAULT_VIEWPORT);
    skia_tree.submit_frame(&tree_cmd).unwrap();
    let mut tree_buf = PixelBuffer::new(393, 852);
    group.bench_function("rasterize_1000_nodes_393x852", |b| {
        b.iter(|| {
            skia_tree.rasterize_at_scale(&mut tree_buf, 1.0, ColorRgba::BLACK);
            black_box(tree_buf.pixels()[0])
        });
    });

    group.finish();
}

fn bench_web_backend(c: &mut Criterion) {
    let mut group = c.benchmark_group("backend_web");

    let init_100 = build_row_tree_cmd(100, 1);
    let mut web_100 = WebBackend::new();
    web_100.set_viewport(1, DEFAULT_VIEWPORT);
    web_100.submit_frame(&init_100).unwrap();

    let mut frame_id = 2u64;
    group.bench_function("incremental_dom_patch_10_nodes", |b| {
        b.iter(|| {
            frame_id += 1;
            let patch = build_row_patch_cmd(frame_id, 100, 10);
            black_box(web_100.submit_frame(black_box(&patch)).unwrap())
        });
    });

    group.bench_function("render_html_100_rows", |b| {
        b.iter(|| black_box(web_100.render_html()));
    });

    let init_500 = build_row_tree_cmd(500, 1);
    let mut web_500 = WebBackend::new();
    web_500.set_viewport(1, DEFAULT_VIEWPORT);
    web_500.submit_frame(&init_500).unwrap();

    group.bench_function("render_html_500_rows", |b| {
        b.iter(|| black_box(web_500.render_html()));
    });

    group.finish();
}

fn bench_taffy_vs_yoga(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_engine/taffy_vs_yoga");
    for &rows in &[100usize, 1_000] {
        let init_bytes = build_row_tree_cmd(rows, 1);
        group.throughput(Throughput::Elements((rows * 2) as u64));

        group.bench_with_input(BenchmarkId::new("taffy_static", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let mut backend = HeadlessBackend::new();
                    backend.set_viewport(1, DEFAULT_VIEWPORT);
                    backend
                },
                |mut backend| black_box(backend.submit_frame(black_box(bytes)).unwrap()),
                BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("taffy_dyn_box", rows), &init_bytes, |b, bytes| {
            b.iter_batched(
                || {
                    let boxed: DynLayoutEngine = Box::new(TaffyLayoutEngine::new());
                    let mut backend = HeadlessBackend::with_layout(boxed);
                    backend.set_viewport(1, DEFAULT_VIEWPORT);
                    backend
                },
                |mut backend| black_box(backend.submit_frame(black_box(bytes)).unwrap()),
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
                            let mut backend = HeadlessBackend::with_layout(yoga);
                            backend.set_viewport(1, DEFAULT_VIEWPORT);
                            backend
                        },
                        |mut backend| black_box(backend.submit_frame(black_box(bytes)).unwrap()),
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
