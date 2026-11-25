use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use nve_interop::{
    ChannelPayload, FrameworkKind, GestureCapabilities, InteropRuntime, IntrinsicSize,
    LayoutProposal, MeasurableSurface, NveChannelBus, PayloadEncoding, PointerPhase, PointerSample,
    ScrollEdgeState, SizeConstraints, SurfaceEnvironment,
};

struct BenchTextSurface {
    chars: usize,
    char_width: f32,
    line_height: f32,
}

impl MeasurableSurface for BenchTextSurface {
    fn measure(&self, constraints: SizeConstraints) -> IntrinsicSize {
        let single_line_w = self.chars as f32 * self.char_width;
        let width = if constraints.has_bounded_width() {
            single_line_w.min(constraints.max_w).max(constraints.min_w)
        } else {
            single_line_w.max(constraints.min_w)
        };
        let chars_per_line = (width / self.char_width).floor().max(1.0) as usize;
        let lines = self.chars.div_ceil(chars_per_line).max(1);
        IntrinsicSize::new(width, lines as f32 * self.line_height)
    }
}

fn bench_channel_bus(c: &mut Criterion) {
    let mut group = c.benchmark_group("interop/channel_bus");
    let payload = ChannelPayload::from_slice(&[0xAB; 64]);

    for &subs in &[1usize, 8, 64] {
        let mut bus = NveChannelBus::new();
        let ch_id = bus.ensure_channel("bench.telemetry").unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        for i in 0..subs {
            let c_ref = Arc::clone(&counter);
            bus.subscribe_by_id(ch_id, Some(i as u32 + 1), false, move |msg| {
                c_ref.fetch_add(msg.as_bytes().len(), Ordering::Relaxed);
            })
            .unwrap();
        }

        group.throughput(Throughput::Elements(subs as u64));
        group.bench_with_input(BenchmarkId::new("publish_subscribers", subs), &subs, |b, _| {
            b.iter(|| {
                let delivered = bus
                    .publish_by_id(
                        ch_id,
                        Some(1),
                        PayloadEncoding::Binary,
                        payload.clone(),
                        false,
                    )
                    .unwrap();
                black_box(delivered)
            });
        });
    }

    group.finish();
}

fn bench_gesture_arbitration(c: &mut Criterion) {
    let mut group = c.benchmark_group("interop/gesture_arbitration");

    let mut runtime = InteropRuntime::new();
    let outer = runtime
        .surfaces
        .register_root(FrameworkKind::Flutter, "OuterList", SurfaceEnvironment::default())
        .unwrap();
    let middle = runtime
        .surfaces
        .register_child(outer, FrameworkKind::Nve, "MiddleCarousel")
        .unwrap();
    let inner = runtime
        .surfaces
        .register_child(middle, FrameworkKind::ReactNativeFabric, "InnerFeed")
        .unwrap();

    runtime
        .gestures
        .set_capabilities(outer, GestureCapabilities::vertical_scroll(FrameworkKind::Flutter));
    runtime
        .gestures
        .set_capabilities(middle, GestureCapabilities::horizontal_scroll(FrameworkKind::Nve));
    runtime.gestures.set_capabilities(
        inner,
        GestureCapabilities::vertical_scroll(FrameworkKind::ReactNativeFabric),
    );

    let chain = runtime.surfaces.ancestor_chain(inner).unwrap();

    group.bench_function("nested_scroll_sequence", |b| {
        let mut ptr_id = 1u32;
        b.iter(|| {
            ptr_id = ptr_id.wrapping_add(1);
            runtime
                .gestures
                .update_scroll_edges(inner, ScrollEdgeState::default());
            let d1 = runtime.gestures.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr_id,
                    phase: PointerPhase::Down,
                    x: 100.0,
                    y: 200.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 1_000,
                },
            );
            let d2 = runtime.gestures.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr_id,
                    phase: PointerPhase::Move,
                    x: 100.0,
                    y: 165.0,
                    delta_x: 0.0,
                    delta_y: -35.0,
                    timestamp_us: 2_000,
                },
            );
            runtime.gestures.update_scroll_edges(
                inner,
                ScrollEdgeState {
                    at_bottom: true,
                    ..Default::default()
                },
            );
            let d3 = runtime.gestures.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr_id,
                    phase: PointerPhase::Move,
                    x: 100.0,
                    y: 130.0,
                    delta_x: 0.0,
                    delta_y: -35.0,
                    timestamp_us: 3_000,
                },
            );
            let d4 = runtime.gestures.arbitrate(
                &chain,
                PointerSample {
                    pointer_id: ptr_id,
                    phase: PointerPhase::Up,
                    x: 100.0,
                    y: 130.0,
                    delta_x: 0.0,
                    delta_y: 0.0,
                    timestamp_us: 4_000,
                },
            );
            black_box((d1.winner, d2.winner, d3.winner, d4.reason))
        });
    });

    group.finish();
}

fn bench_intrinsic_layout_negotiation(c: &mut Criterion) {
    let mut group = c.benchmark_group("interop/intrinsic_layout");

    let mut runtime = InteropRuntime::new();
    let surf = runtime
        .surfaces
        .register_root(FrameworkKind::Nve, "TextCard", SurfaceEnvironment::default())
        .unwrap();
    let surface = BenchTextSurface {
        chars: 140,
        char_width: 8.5,
        line_height: 20.0,
    };

    let proposal = LayoutProposal::new(SizeConstraints::loose(320.0, 600.0), 3.0);
    // Warm cache entry
    runtime
        .layout
        .negotiate_surface(surf, &surface, proposal)
        .unwrap();

    group.bench_function("cached_hit", |b| {
        b.iter(|| {
            black_box(
                runtime
                    .layout
                    .negotiate_surface(surf, &surface, black_box(proposal))
                    .unwrap(),
            )
        });
    });

    group.bench_function("uncached_measure_and_snap", |b| {
        let mut width = 200.0f32;
        b.iter(|| {
            width = 200.0 + ((width - 199.0) % 240.0);
            runtime.layout.invalidate_surface(surf);
            let p = LayoutProposal::new(SizeConstraints::loose(width, 600.0), 3.0);
            black_box(runtime.layout.negotiate_surface(surf, &surface, p).unwrap())
        });
    });

    group.finish();
}

criterion_group!(
    interop_benches,
    bench_channel_bus,
    bench_gesture_arbitration,
    bench_intrinsic_layout_negotiation
);
criterion_main!(interop_benches);
