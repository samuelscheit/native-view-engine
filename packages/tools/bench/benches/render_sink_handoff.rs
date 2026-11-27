//! Microbenchmarks comparing direct single-owner `DioxusSurface::commit_to(&mut backend)`
//! (zero wire encode/decode, zero duplicate tree) against cross-boundary `NVE2` wire encode +
//! `StoreWireReplica::submit_nve2_frame` decode across 10-node incremental patches, 100-node and
//! 1,000-node mounts, and rich extended props (CSS Grid, gradients, `:hover` state rules,
//! transitions, `@media` queries, `Transform2D`).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use nve_abi::{
    ColorRgba, EventBatchBuf, FrameId, InteractionEvent, NodeId, SurfaceId, TimestampUs,
};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_bench::DEFAULT_VIEWPORT;
use nve_components::tables::{core_realization_order, Interaction};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_engine::{StoreBackend, StoreWireReplica};
use nve_motion::{EasingCurve, TransitionProperty, TransitionSpec};
use nve_store::NodeStore;

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
                    nve_table: nve_dioxus::table_attr_fn(|tx, node| {
                        nve_motion::write_transitions(
                            tx,
                            node,
                            &[TransitionSpec::new(TransitionProperty::BackgroundColor, 160)
                                .easing(EasingCurve::EaseOut)],
                        );
                    }),
                    view {
                        nve_style: TypedStyle::new()
                            .width_px(20.0)
                            .height_px(20.0)
                            .radius(4.0)
                            .bg(ColorRgba::rgb(56, 189, 248)),
                    }
                    text { "Card #{i}" }
                }
            }
        }
    }
}

fn find_tick_node_id(store: &NodeStore) -> NodeId {
    store
        .live_nodes()
        .find_map(|id| {
            store
                .try_row::<Interaction>(id)?
                .listeners()
                .contains(InteractionEvent::Press)
                .then_some(id)
        })
        .expect("pressable tick node")
}

fn bench_handoff_patch_10_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("store_handoff/patch_10_nodes");
    group.throughput(Throughput::Elements(10));

    // 1. Direct single-owner NodeStore commit -> WebBackend
    let mut direct_surface =
        DioxusSurface::new_with_props(SurfaceId::ROOT, row_list_app, RowListProps { rows: 50 });
    let mut direct_web = WebBackend::new();
    direct_web.set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    direct_surface
        .commit_to(FrameId(1), &mut direct_web)
        .expect("initial direct commit to web");
    let direct_btn = find_tick_node_id(direct_surface.store());
    let mut direct_frame = FrameId(1);

    let mut direct_ev = EventBatchBuf::with_capacity(1, 0);
    group.bench_function("direct_commit_to_web", |b| {
        b.iter(|| {
            direct_frame = direct_frame.next();
            direct_ev.clear();
            direct_ev.push_press(
                SurfaceId::ROOT,
                direct_btn,
                TimestampUs(direct_frame.raw() * 16_000),
                10.0,
                10.0,
            );
            direct_surface.dispatch_events(direct_ev.as_validated());
            black_box(
                direct_surface
                    .commit_to(direct_frame, &mut direct_web)
                    .expect("incremental direct commit to web"),
            )
        });
    });

    // 2. Cross-boundary NVE2 wire encode + StoreWireReplica<WebBackend> decode
    let mut wire_surface =
        DioxusSurface::new_with_props(SurfaceId::ROOT, row_list_app, RowListProps { rows: 50 });
    let mut wire_web = StoreWireReplica::new(WebBackend::new(), core_realization_order());
    wire_web.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
    {
        let init = wire_surface.render_nve2_slice(FrameId(1));
        wire_web
            .submit_nve2_frame(init)
            .expect("initial wire commit to web");
    }
    let wire_btn = find_tick_node_id(wire_web.store());
    let mut wire_frame = FrameId(1);

    let mut wire_ev = EventBatchBuf::with_capacity(1, 0);
    group.bench_function("nve2_wire_encode_decode_web", |b| {
        b.iter(|| {
            wire_frame = wire_frame.next();
            wire_ev.clear();
            wire_ev.push_press(
                SurfaceId::ROOT,
                wire_btn,
                TimestampUs(wire_frame.raw() * 16_000),
                10.0,
                10.0,
            );
            wire_surface.dispatch_events(wire_ev.as_validated());
            let bytes = wire_surface.render_nve2_slice(wire_frame);
            black_box(
                wire_web
                    .submit_nve2_frame(black_box(bytes))
                    .expect("incremental wire commit to web"),
            )
        });
    });

    group.finish();
}

fn bench_handoff_mount_trees(c: &mut Criterion) {
    for &(label, rows) in &[("mount_100_nodes", 50usize), ("mount_1000_nodes", 500usize)] {
        let mut group = c.benchmark_group(format!("store_handoff/{label}"));
        group.throughput(Throughput::Elements((rows * 2) as u64));

        group.bench_function("direct_commit_to_web", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(
                        SurfaceId::ROOT,
                        row_list_app,
                        RowListProps { rows },
                    );
                    let mut web = WebBackend::new();
                    web.set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    (surface, web)
                },
                |(mut surface, mut web)| {
                    black_box(surface.commit_to(FrameId(1), &mut web).expect("mount commit web"))
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function("nve2_wire_encode_decode_web", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(
                        SurfaceId::ROOT,
                        row_list_app,
                        RowListProps { rows },
                    );
                    let mut web =
                        StoreWireReplica::new(WebBackend::new(), core_realization_order());
                    web.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    (surface, web)
                },
                |(mut surface, mut web)| {
                    let bytes = surface.render_nve2_slice(FrameId(1));
                    black_box(
                        web.submit_nve2_frame(black_box(bytes))
                            .expect("mount wire web"),
                    )
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function("direct_commit_to_headless", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(
                        SurfaceId::ROOT,
                        row_list_app,
                        RowListProps { rows },
                    );
                    let mut headless = HeadlessBackend::new().with_mutation_tracking(false);
                    headless.set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    (surface, headless)
                },
                |(mut surface, mut headless)| {
                    black_box(
                        surface
                            .commit_to(FrameId(1), &mut headless)
                            .expect("mount commit headless"),
                    )
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function("nve2_wire_encode_decode_headless", |b| {
            b.iter_batched(
                || {
                    let surface = DioxusSurface::new_with_props(
                        SurfaceId::ROOT,
                        row_list_app,
                        RowListProps { rows },
                    );
                    let mut headless = StoreWireReplica::new(
                        HeadlessBackend::new().with_mutation_tracking(false),
                        core_realization_order(),
                    );
                    headless.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                    (surface, headless)
                },
                |(mut surface, mut headless)| {
                    let bytes = surface.render_nve2_slice(FrameId(1));
                    black_box(
                        headless
                            .submit_nve2_frame(black_box(bytes))
                            .expect("mount wire headless"),
                    )
                },
                BatchSize::SmallInput,
            );
        });

        group.finish();
    }
}

fn bench_handoff_rich_extended(c: &mut Criterion) {
    let mut group = c.benchmark_group("store_handoff/rich_extended_24_cards");
    group.throughput(Throughput::Elements(24));

    group.bench_function("direct_commit_to_web", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(SurfaceId::ROOT, rich_extended_app);
                let mut web = WebBackend::new();
                web.set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                (surface, web)
            },
            |(mut surface, mut web)| {
                black_box(surface.commit_to(FrameId(1), &mut web).expect("rich commit web"))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("nve2_wire_encode_decode_web", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(SurfaceId::ROOT, rich_extended_app);
                let mut web = StoreWireReplica::new(WebBackend::new(), core_realization_order());
                web.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                (surface, web)
            },
            |(mut surface, mut web)| {
                let bytes = surface.render_nve2_slice(FrameId(1));
                black_box(
                    web.submit_nve2_frame(black_box(bytes))
                        .expect("rich wire web"),
                )
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("direct_commit_to_skia", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(SurfaceId::ROOT, rich_extended_app);
                let mut skia = SkiaBackend::new();
                skia.set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                (surface, skia)
            },
            |(mut surface, mut skia)| {
                black_box(surface.commit_to(FrameId(1), &mut skia).expect("rich commit skia"))
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("nve2_wire_encode_decode_skia", |b| {
        b.iter_batched(
            || {
                let surface = DioxusSurface::new(SurfaceId::ROOT, rich_extended_app);
                let mut skia = StoreWireReplica::new(SkiaBackend::new(), core_realization_order());
                skia.backend_mut().set_viewport(SurfaceId::ROOT, DEFAULT_VIEWPORT);
                (surface, skia)
            },
            |(mut surface, mut skia)| {
                let bytes = surface.render_nve2_slice(FrameId(1));
                black_box(
                    skia.submit_nve2_frame(black_box(bytes))
                        .expect("rich wire skia"),
                )
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
