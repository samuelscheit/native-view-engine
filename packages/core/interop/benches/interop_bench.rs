//! Criterion micro-benchmarks for `nve-interop`: cross-framework `GestureArbitrator`
//! nested-scroll arbitration and `NveChannelBus` pub/sub dispatch.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{SurfaceId, TimestampUs};
    use nve_interop::{
        ChannelPayload, FrameworkKind, GestureArbitrator, GestureCapabilities, InteropRuntime,
        NveChannelBus, PointerPhase, PointerSample, SurfaceEnvironment, SurfaceOrchestrator,
    };

    fn bench_gesture_arbitrator(c: &mut Criterion) {
        let mut arb = GestureArbitrator::new();
        let chain = [
            SurfaceId::new(1, 0),
            SurfaceId::new(2, 0),
            SurfaceId::new(3, 0),
            SurfaceId::new(4, 0),
            SurfaceId::new(5, 0),
            SurfaceId::new(6, 0),
            SurfaceId::new(7, 0),
            SurfaceId::new(8, 0),
        ];
        for &sid in &chain {
            let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
            if sid.raw() < 6 {
                caps.scroll_edges.at_bottom = true;
            }
            arb.set_capabilities(sid, caps);
        }

        c.bench_function("interop_gesture_arbitrate_8_deep_scroll_cycle", |b| {
            b.iter(|| {
                let down = arb.arbitrate(
                    black_box(&chain),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Down,
                        x: 20.0,
                        y: 100.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(1_000),
                    },
                );
                let mv = arb.arbitrate(
                    black_box(&chain),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Move,
                        x: 20.0,
                        y: 50.0,
                        delta_x: 0.0,
                        delta_y: -50.0,
                        timestamp_us: TimestampUs(2_000),
                    },
                );
                let up = arb.arbitrate(
                    black_box(&chain),
                    PointerSample {
                        pointer_id: 1,
                        phase: PointerPhase::Up,
                        x: 20.0,
                        y: 50.0,
                        delta_x: 0.0,
                        delta_y: 0.0,
                        timestamp_us: TimestampUs(3_000),
                    },
                );
                black_box((down.winner, mv.winner, up.winner))
            });
        });
    }

    fn bench_surface_orchestrator(c: &mut Criterion) {
        let mut orch = SurfaceOrchestrator::new();
        let mut ids = Vec::with_capacity(1_000);
        for i in 0..1_000 {
            let id = orch
                .register_root(FrameworkKind::Nve, format!("S{i}"), SurfaceEnvironment::default())
                .expect("register bench root surface");
            ids.push(id);
        }
        c.bench_function("interop_surface_orchestrator_get_1000", |b| {
            b.iter(|| {
                let mut sum = 0usize;
                for &sid in &ids {
                    if let Some(rec) = orch.get(black_box(sid)) {
                        sum += rec.component_name.len();
                    }
                }
                black_box(sum)
            });
        });

        let mut runtime = InteropRuntime::new();
        let root = runtime
            .surfaces
            .register_root(FrameworkKind::Flutter, "Root", SurfaceEnvironment::default())
            .expect("register bench root");
        let mut cur = root;
        for depth in 1..8 {
            cur = runtime
                .surfaces
                .register_child(cur, FrameworkKind::Nve, format!("D{depth}"))
                .expect("register bench child");
            runtime.gestures.set_capabilities(
                cur,
                GestureCapabilities::vertical_scroll(FrameworkKind::Nve),
            );
        }
        let leaf = cur;
        c.bench_function("interop_runtime_arbitrate_pointer_8_deep_zero_alloc", |b| {
            b.iter(|| {
                let d = runtime
                    .arbitrate_pointer(
                        black_box(leaf),
                        PointerSample {
                            pointer_id: 1,
                            phase: PointerPhase::Down,
                            x: 10.0,
                            y: 10.0,
                            delta_x: 0.0,
                            delta_y: 0.0,
                            timestamp_us: TimestampUs(100),
                        },
                    )
                    .expect("arbitrate pointer down");
                let u = runtime
                    .arbitrate_pointer(
                        black_box(leaf),
                        PointerSample {
                            pointer_id: 1,
                            phase: PointerPhase::Up,
                            x: 10.0,
                            y: 10.0,
                            delta_x: 0.0,
                            delta_y: 0.0,
                            timestamp_us: TimestampUs(200),
                        },
                    )
                    .expect("arbitrate pointer up");
                black_box((d.winner, u.winner))
            });
        });
    }

    fn bench_channel_bus(c: &mut Criterion) {
        let mut bus = NveChannelBus::new();
        let received = Arc::new(AtomicUsize::new(0));
        let received_cb = Arc::clone(&received);
        let _sub = bus
            .subscribe("bench.theme", Some(SurfaceId::ROOT), false, move |msg| {
                received_cb.fetch_add(msg.payload.as_bytes().len(), Ordering::Relaxed);
            })
            .expect("valid bench channel subscription");
        let payload = ChannelPayload::from_slice(&[1u8, 2, 3, 4, 5, 6, 7, 8]);

        c.bench_function("interop_channel_bus_publish_bytes", |b| {
            b.iter(|| {
                let delivered = bus
                    .publish_bytes("bench.theme", None, black_box(payload.clone()), false)
                    .expect("valid bench publish");
                black_box((delivered, received.load(Ordering::Relaxed)))
            });
        });
    }

    criterion_group!(
        benches,
        bench_gesture_arbitrator,
        bench_surface_orchestrator,
        bench_channel_bus
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
