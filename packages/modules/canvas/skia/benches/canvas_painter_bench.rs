//! Criterion end-to-end benchmarks painting onto a reused `256x256` `PixelBuffer` via
//! `CanvasSkiaSystem::paint_node` across N ∈ {10, 100, 1_000} compact primitives.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nve_abi::{ColorRgba, EventBatchBuf};
use nve_backend_skia::{PixelBuffer, SkiaSystemPaintCtx, SkiaTableSystem};
use nve_canvas::{Canvas, CanvasPainter, DirectCanvasRegistry, DrawList, DrawListRecorder};
use nve_canvas_skia::CanvasSkiaSystem;
use nve_engine::LayoutRect;
use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

fn emit_compact_primitives(p: &mut dyn CanvasPainter, count: usize) {
    let poly = [
        (4.0f32, 6.0),
        (9.0, 10.0),
        (14.0, 6.0),
        (19.0, 12.0),
        (24.0, 8.0),
        (29.0, 14.0),
        (34.0, 10.0),
        (39.0, 16.0),
    ];
    p.clear(ColorRgba::rgb(12, 16, 24));
    for i in 0..count {
        let x = ((i * 11) % 240) as f32;
        let y = ((i * 7) % 240) as f32;
        let color = ColorRgba::rgb((i & 0xFF) as u8, ((i * 5) & 0xFF) as u8, 220);
        match i % 5 {
            0 => p.fill_rect(x, y, 6.0, 5.0, color),
            1 => p.fill_rounded_rect(x, y, 6.0, 6.0, 2.0, color),
            2 => p.fill_circle(x + 3.0, y + 3.0, 2.5, color),
            3 => p.stroke_line(x, y, x + 6.0, y + 4.0, 1.5, color),
            _ => p.stroke_polyline(&poly, 1.5, color),
        }
    }
}

fn bench_skia_canvas_painter(c: &mut Criterion) {
    let mut group = c.benchmark_group("canvas_skia_painter");
    let rect = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: 256.0,
        height: 256.0,
    };

    for &n in &[10usize, 100, 1_000] {
        // 1. Direct paint (`DirectCanvasRegistry::paint` -> `SkiaCanvasPainter`)
        group.bench_with_input(BenchmarkId::new("skia_direct_paint", n), &n, |b, &n| {
            let reg = DirectCanvasRegistry::new();
            let mut store = NodeStore::new();
            let mut changes = ChangeSet::default();
            let node = {
                let mut tx = store.tx();
                let node = tx.create();
                tx.set_children(NodeId::ROOT, &[node]);
                reg.bind(&mut tx, node, move |p, _size| {
                    emit_compact_primitives(p, n);
                });
                tx.commit_into(&mut changes);
                node
            };
            let sys = CanvasSkiaSystem::with_direct(reg);
            let mut buf = PixelBuffer::new(256, 256);
            let mut events = EventBatchBuf::new();

            b.iter(|| {
                let mut ctx = SkiaSystemPaintCtx {
                    buffer: &mut buf,
                    rect,
                    clip_rect: None,
                    scale_factor: 1.0,
                    opacity: 1.0,
                    inherited_text_color: Rgba::WHITE,
                    events: &mut events,
                };
                sys.paint_node(&mut ctx, black_box(&store), node);
                black_box(buf.pixels()[0])
            });
        });

        // 1b. Apples-to-apples direct frame (`invalidate` + `commit_into` + `paint_node`)
        group.bench_with_input(
            BenchmarkId::new("skia_direct_invalidate_and_paint", n),
            &n,
            |b, &n| {
                let reg = DirectCanvasRegistry::new();
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    reg.bind(&mut tx, node, move |p, _size| {
                        emit_compact_primitives(p, n);
                    });
                    tx.commit_into(&mut changes);
                    node
                };
                let sys = CanvasSkiaSystem::with_direct(reg.clone());
                let mut buf = PixelBuffer::new(256, 256);
                let mut events = EventBatchBuf::new();

                b.iter(|| {
                    {
                        let mut tx = store.tx();
                        let g = reg.invalidate(&mut tx, node).expect("bound node");
                        black_box(g);
                        tx.commit_into(&mut changes);
                    }
                    let mut ctx = SkiaSystemPaintCtx {
                        buffer: &mut buf,
                        rect,
                        clip_rect: None,
                        scale_factor: 1.0,
                        opacity: 1.0,
                        inherited_text_color: Rgba::WHITE,
                        events: &mut events,
                    };
                    sys.paint_node(&mut ctx, black_box(&store), node);
                    black_box(buf.pixels()[0])
                });
            },
        );

        // 2. Recorded encode + commit + replay (`DrawListRecorder` -> `draw_list_ephemeral` -> `paint_node`)
        group.bench_with_input(
            BenchmarkId::new("skia_recorded_encode_and_replay", n),
            &n,
            |b, &n| {
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    tx.commit_into(&mut changes);
                    node
                };
                let sys = CanvasSkiaSystem::new();
                let mut buf = PixelBuffer::new(256, 256);
                let mut events = EventBatchBuf::new();
                let mut generation = 1u32;

                b.iter(|| {
                    let mut rec = DrawListRecorder::with_capacity(n + 1);
                    emit_compact_primitives(&mut rec, black_box(n));
                    let dl = rec.finish();
                    generation = generation.wrapping_add(1).max(1);
                    {
                        let mut tx = store.tx();
                        tx.edit::<Canvas>(node)
                            .draw_list_ephemeral(dl)
                            .generation(generation);
                        tx.commit_into(&mut changes);
                    }
                    let mut ctx = SkiaSystemPaintCtx {
                        buffer: &mut buf,
                        rect,
                        clip_rect: None,
                        scale_factor: 1.0,
                        opacity: 1.0,
                        inherited_text_color: Rgba::WHITE,
                        events: &mut events,
                    };
                    sys.paint_node(&mut ctx, black_box(&store), node);
                    black_box(buf.pixels()[0])
                });
            },
        );

        // 2b. Zero-alloc recorded path (`draw_list_ephemeral_with` + `DrawList::record_into_words`)
        group.bench_with_input(
            BenchmarkId::new("skia_recorded_ephemeral_with_and_replay", n),
            &n,
            |b, &n| {
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    tx.commit_into(&mut changes);
                    node
                };
                let sys = CanvasSkiaSystem::new();
                let mut buf = PixelBuffer::new(256, 256);
                let mut events = EventBatchBuf::new();
                let mut generation = 1u32;

                b.iter(|| {
                    generation = generation.wrapping_add(1).max(1);
                    {
                        let mut tx = store.tx();
                        tx.edit::<Canvas>(node)
                            .draw_list_ephemeral_with(|out| {
                                DrawList::record_into_words(out, |rec| {
                                    emit_compact_primitives(rec, black_box(n));
                                });
                                true
                            })
                            .generation(generation);
                        tx.commit_into(&mut changes);
                    }
                    let mut ctx = SkiaSystemPaintCtx {
                        buffer: &mut buf,
                        rect,
                        clip_rect: None,
                        scale_factor: 1.0,
                        opacity: 1.0,
                        inherited_text_color: Rgba::WHITE,
                        events: &mut events,
                    };
                    sys.paint_node(&mut ctx, black_box(&store), node);
                    black_box(buf.pixels()[0])
                });
            },
        );

        // 3. Recorded replay only (`paint_node` against pre-committed `Res<DrawList>`)
        group.bench_with_input(
            BenchmarkId::new("skia_recorded_replay_only", n),
            &n,
            |b, &n| {
                let dl = DrawList::record_with_capacity(n + 1, |rec| {
                    emit_compact_primitives(rec, n);
                });
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    tx.edit::<Canvas>(node).draw_list_ephemeral(dl).generation(1);
                    tx.commit_into(&mut changes);
                    node
                };
                let sys = CanvasSkiaSystem::new();
                let mut buf = PixelBuffer::new(256, 256);
                let mut events = EventBatchBuf::new();

                b.iter(|| {
                    let mut ctx = SkiaSystemPaintCtx {
                        buffer: &mut buf,
                        rect,
                        clip_rect: None,
                        scale_factor: 1.0,
                        opacity: 1.0,
                        inherited_text_color: Rgba::WHITE,
                        events: &mut events,
                    };
                    sys.paint_node(&mut ctx, black_box(&store), node);
                    black_box(buf.pixels()[0])
                });
            },
        );
    }

    group.finish();
}

#[cfg(feature = "skia")]
fn bench_skia_native_gpu_vs_cached(c: &mut Criterion) {
    use nve_abi::SurfaceId;
    use nve_backend_skia::{CpuSkiaSurface, SkiaBackend};
    use nve_canvas_skia::SkiaNativeCanvasPainter;
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::Length;
    use nve_tables::tables::Layout;

    let mut group = c.benchmark_group("canvas_skia_native");
    for &n in &[10usize, 100, 1_000] {
        // 1. Raw `SkiaNativeCanvasPainter` direct draw on `CpuSkiaSurface::canvas()`.
        group.bench_with_input(BenchmarkId::new("skia_native_sink_direct", n), &n, |b, &n| {
            let mut surface = CpuSkiaSurface::new(256, 256).expect("CpuSkiaSurface");
            b.iter(|| {
                {
                    let (mut painter, _) =
                        SkiaNativeCanvasPainter::new(surface.canvas(), 256.0, 256.0, 1.0);
                    emit_compact_primitives(&mut painter, black_box(n));
                }
                black_box(surface.canvas().save_count())
            });
        });

        // 2. Full animating frame: Direct `invalidate` + `SkiaBackend::commit` + native `SkiaNativeSink`
        //    (0 CPU `PixelBuffer` rasterizations / 0 texture uploads).
        group.bench_with_input(
            BenchmarkId::new("skia_native_direct_frame", n),
            &n,
            |b, &n| {
                let reg = DirectCanvasRegistry::new();
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    tx.edit::<Layout>(node)
                        .width(Length::points(256.0))
                        .height(Length::points(256.0));
                    reg.bind(&mut tx, node, move |p, _| emit_compact_primitives(p, n));
                    tx.commit_into(&mut changes);
                    node
                };
                let mut backend = SkiaBackend::new();
                backend.register_table_system(CanvasSkiaSystem::with_direct(reg.clone()));
                backend.set_viewport(
                    SurfaceId::ROOT,
                    Viewport {
                        width: 256.0,
                        height: 256.0,
                        scale_factor: 1.0,
                    },
                );
                backend.commit(&store, &changes).expect("init commit");
                let mut surface = CpuSkiaSurface::new(256, 256).expect("CpuSkiaSurface");

                b.iter(|| {
                    {
                        let mut tx = store.tx();
                        let g = reg.invalidate(&mut tx, node).expect("bound");
                        black_box(g);
                        tx.commit_into(&mut changes);
                    }
                    backend.commit(&store, &changes).expect("commit");
                    let (scene, table_reg, ev) = backend.scene_paint_parts_mut();
                    surface
                        .render_scene_cached_with_plugins(
                            scene,
                            1.0,
                            ColorRgba::BLACK,
                            Some((&store, table_reg, ev)),
                        )
                        .expect("render");
                    black_box(surface.cache_stats().plugin_rasterizations)
                });
            },
        );

        // 3. Full animating frame: Recorded `record_into_words` + `SkiaBackend::commit` +
        //    CPU `PixelBuffer` rasterization + `SkImage` upload via `SkiaRendererCaches`.
        group.bench_with_input(
            BenchmarkId::new("skia_recorded_upload_frame", n),
            &n,
            |b, &n| {
                let mut store = NodeStore::new();
                let mut changes = ChangeSet::default();
                let node = {
                    let mut tx = store.tx();
                    let node = tx.create();
                    tx.set_children(NodeId::ROOT, &[node]);
                    tx.edit::<Layout>(node)
                        .width(Length::points(256.0))
                        .height(Length::points(256.0));
                    tx.edit::<Canvas>(node)
                        .draw_list_ephemeral_with(|out| {
                            DrawList::record_into_words(out, |rec| emit_compact_primitives(rec, n));
                            true
                        })
                        .generation(1);
                    tx.commit_into(&mut changes);
                    node
                };
                let mut backend = SkiaBackend::new();
                backend.register_table_system(CanvasSkiaSystem::new());
                backend.set_viewport(
                    SurfaceId::ROOT,
                    Viewport {
                        width: 256.0,
                        height: 256.0,
                        scale_factor: 1.0,
                    },
                );
                backend.commit(&store, &changes).expect("init commit");
                let mut surface = CpuSkiaSurface::new(256, 256).expect("CpuSkiaSurface");
                let mut generation = 2u32;

                b.iter(|| {
                    generation = generation.wrapping_add(1).max(1);
                    {
                        let mut tx = store.tx();
                        tx.edit::<Canvas>(node)
                            .draw_list_ephemeral_with(|out| {
                                DrawList::record_into_words(out, |rec| {
                                    emit_compact_primitives(rec, black_box(n));
                                });
                                true
                            })
                            .generation(generation);
                        tx.commit_into(&mut changes);
                    }
                    backend.commit(&store, &changes).expect("commit");
                    let (scene, table_reg, ev) = backend.scene_paint_parts_mut();
                    surface
                        .render_scene_cached_with_plugins(
                            scene,
                            1.0,
                            ColorRgba::BLACK,
                            Some((&store, table_reg, ev)),
                        )
                        .expect("render");
                    black_box(surface.cache_stats().plugin_rasterizations)
                });
            },
        );
    }
    group.finish();
}

#[cfg(feature = "skia")]
criterion_group!(
    benches,
    bench_skia_canvas_painter,
    bench_skia_native_gpu_vs_cached
);
#[cfg(not(feature = "skia"))]
criterion_group!(benches, bench_skia_canvas_painter);
criterion_main!(benches);
