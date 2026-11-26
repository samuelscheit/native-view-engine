//! Track 1B-1 microbenchmarks comparing direct monomorphized `DioxusSurface::flush_into<S: RenderSink>`
//! (zero encode/decode, zero vtable) against cross-boundary `BatchSink` encode + `submit_frame` decode
//! across 10-node incremental patches, 100-node and 1,000-node mounts, and rich extended props
//! (gradients, SVG, `:hover` state rules, transitions, `LayoutExt`, `Transform2D`).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use nve_abi::{ColorRgba, EventMask, EventPayload, NativeEvent, NativeEventKind};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_bench::DEFAULT_VIEWPORT;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::RenderBackend;

#[derive(Clone, PartialEq)]
struct RowListProps {
    rows: usize,
}

fn row_list_app(props: RowListProps) -> Element {
    let mut tick = use_signal(|| 0u32);
    let rows = props.rows;
    let t = tick();
    let row_style = TypedStyle::new()
        .width_pct(100.0)
        .height_px(40.0)
        .flex_row()
        .gap(8.0)
        .padding_xy(12.0, 6.0)
        .bg(ColorRgba::rgb(30, 41, 59))
        .radius(6.0);
    let txt_style = TypedStyle::new().color(ColorRgba::WHITE).font_size(14.0);

    rsx! {
        view {
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(4.0),
            pressable {
                test_id: "bench-tick",
                onpress: move |_| tick += 1,
                text { "Tick {t}" }
            }
            for i in 0..rows {
                view {
                    key: "{i}",
                    nve_style: row_style.clone(),
                    text {
                        nve_style: txt_style.clone(),
                        if i < 10 {
                            "Row #{i} · frame {t}"
                        } else {
                            "Row #{i} · static"
                        }
                    }
                }
            }
        }
    }
}

fn rich_extended_app() -> Element {
    let icon = SvgPicture::from_paths(24.0, 24.0, &["M4 12 L20 12 M14 6 L20 12 L14 18"]);
    let grid_style = TypedStyle::new()
        .display_grid()
        .grid_cols([GridTrack::repeat_minmax_zero_fr(3)])
        .column_gap(12.0)
        .row_gap(12.0);

    let card_style = TypedStyle::new()
        .width_px(240.0)
        .height_px(120.0)
        .radius(12.0)
        .cursor(CursorKind::Pointer)
        .transform(Transform2D::translate(0.0, 2.0))
        .linear_gradient(
            135.0,
            [
                GradientStop::new(0.0, ColorRgba::rgb(37, 99, 235)),
                GradientStop::new(1.0, ColorRgba::rgb(124, 58, 237)),
            ],
        )
        .transition(
            TransitionSpec::new(TransitionProperty::BackgroundColor, 160)
                .easing(EasingCurve::EaseOut),
        )
        .on_hover_with(|p| {
            p.scale(1.02).linear_gradient(
                90.0,
                [
                    GradientStop::new(0.0, ColorRgba::rgb(14, 165, 233)),
                    GradientStop::new(1.0, ColorRgba::rgb(99, 102, 241)),
                ],
            )
        })
        .at_with(MediaQuery::min_width(768), |p| p.padding(16.0));

    rsx! {
        view {
            nve_style: grid_style,
            for i in 0..24usize {
                pressable {
                    key: "{i}",
                    nve_style: card_style.clone(),
                    svg {
                        nve_style: TypedStyle::new().width_px(20.0).height_px(20.0),
                        picture: SvgPictureAttr::from(icon.clone()),
                    }
                    text { "Card #{i}" }
                }
            }
        }
    }
}

fn find_tick_node_id_web(web: &WebBackend) -> u32 {
    web.iter_nodes()
        .find_map(|(id, el)| ((el.event_mask & EventMask::PRESS) != 0).then_some(id))
        .expect("pressable tick node")
}

fn bench_handoff_patch_10_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_sink/patch_10_nodes");
    group.throughput(Throughput::Elements(10));

    // 1. Direct flush_into -> WebBackend
    let mut direct_surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows: 50 });
    let mut direct_web = WebBackend::new();
    direct_web.set_viewport(1, DEFAULT_VIEWPORT);
    direct_surface.flush_into(1, &mut direct_web).unwrap();
    let direct_btn = find_tick_node_id_web(&direct_web);
    let mut direct_frame = 1u64;

    group.bench_function("direct_flush_into_web", |b| {
        b.iter(|| {
            direct_frame += 1;
            direct_surface.dispatch_events(&[NativeEvent {
                surface_id: 1,
                node_id: direct_btn,
                timestamp_us: direct_frame * 16_000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 10.0, y: 10.0 },
            }]);
            black_box(direct_surface.flush_into(direct_frame, &mut direct_web).unwrap())
        });
    });

    // 2. Cross-boundary BatchSink encode + WebBackend::submit_frame decode
    let mut batch_surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows: 50 });
    let mut batch_web = WebBackend::new();
    batch_web.set_viewport(1, DEFAULT_VIEWPORT);
    {
        let init = batch_surface.render_frame_slice(1);
        batch_web.submit_frame(init).unwrap();
    }
    let batch_btn = find_tick_node_id_web(&batch_web);
    let mut batch_frame = 1u64;

    group.bench_function("batch_sink_encode_decode_web", |b| {
        b.iter(|| {
            batch_frame += 1;
            batch_surface.dispatch_events(&[NativeEvent {
                surface_id: 1,
                node_id: batch_btn,
                timestamp_us: batch_frame * 16_000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point { x: 10.0, y: 10.0 },
            }]);
            let bytes = batch_surface.render_frame_slice(batch_frame);
            black_box(batch_web.submit_frame(black_box(bytes)).unwrap())
        });
    });

    group.finish();
}

fn bench_handoff_mount_trees(c: &mut Criterion) {
    for &(label, rows) in &[("mount_100_nodes", 50usize), ("mount_1000_nodes", 500usize)] {
        let mut group = c.benchmark_group(format!("render_sink/{label}"));
        group.throughput(Throughput::Elements((rows * 2) as u64));

        group.bench_function("direct_flush_into_web", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows });
                    let mut web = WebBackend::new();
                    web.set_viewport(1, DEFAULT_VIEWPORT);
                    (surface, web)
                },
                |(mut surface, mut web)| black_box(surface.flush_into(1, &mut web).unwrap()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function("batch_sink_encode_decode_web", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows });
                    let mut web = WebBackend::new();
                    web.set_viewport(1, DEFAULT_VIEWPORT);
                    (surface, web)
                },
                |(mut surface, mut web)| {
                    let bytes = surface.render_frame_slice(1);
                    black_box(web.submit_frame(black_box(bytes)).unwrap())
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function("direct_flush_into_headless", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows });
                    let mut headless = HeadlessBackend::new().with_mutation_tracking(false);
                    headless.set_viewport(1, DEFAULT_VIEWPORT);
                    (surface, headless)
                },
                |(mut surface, mut headless)| {
                    black_box(surface.flush_into(1, &mut headless).unwrap())
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function("batch_sink_encode_decode_headless", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(1, row_list_app, RowListProps { rows });
                    let mut headless = HeadlessBackend::new().with_mutation_tracking(false);
                    headless.set_viewport(1, DEFAULT_VIEWPORT);
                    (surface, headless)
                },
                |(mut surface, mut headless)| {
                    let bytes = surface.render_frame_slice(1);
                    black_box(headless.submit_frame(black_box(bytes)).unwrap())
                },
                BatchSize::SmallInput,
            );
        });

        group.finish();
    }
}

fn bench_handoff_rich_extended(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_sink/rich_extended_24_cards");
    group.throughput(Throughput::Elements(24));

    group.bench_function("direct_flush_into_web", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(1, rich_extended_app);
                let mut web = WebBackend::new();
                web.set_viewport(1, DEFAULT_VIEWPORT);
                (surface, web)
            },
            |(mut surface, mut web)| black_box(surface.flush_into(1, &mut web).unwrap()),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("batch_sink_encode_decode_web", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(1, rich_extended_app);
                let mut web = WebBackend::new();
                web.set_viewport(1, DEFAULT_VIEWPORT);
                (surface, web)
            },
            |(mut surface, mut web)| {
                let bytes = surface.render_frame_slice(1);
                black_box(web.submit_frame(black_box(bytes)).unwrap())
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("direct_flush_into_skia", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(1, rich_extended_app);
                let mut skia = SkiaBackend::new();
                skia.set_viewport(1, DEFAULT_VIEWPORT);
                (surface, skia)
            },
            |(mut surface, mut skia)| black_box(surface.flush_into(1, &mut skia).unwrap()),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("batch_sink_encode_decode_skia", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(1, rich_extended_app);
                let mut skia = SkiaBackend::new();
                skia.set_viewport(1, DEFAULT_VIEWPORT);
                (surface, skia)
            },
            |(mut surface, mut skia)| {
                let bytes = surface.render_frame_slice(1);
                black_box(skia.submit_frame(black_box(bytes)).unwrap())
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    handoff_benches,
    bench_handoff_patch_10_nodes,
    bench_handoff_mount_trees,
    bench_handoff_rich_extended
);
criterion_main!(handoff_benches);
