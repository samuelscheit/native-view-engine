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
    use nve_interop::{
        ChannelPayload, FrameworkKind, GestureArbitrator, GestureCapabilities, NveChannelBus,
        PointerPhase, PointerSample,
    };

    fn bench_gesture_arbitrator(c: &mut Criterion) {
        let mut arb = GestureArbitrator::new();
        let chain = [1u32, 2, 3, 4, 5, 6, 7, 8];
        for &sid in &chain {
            let mut caps = GestureCapabilities::vertical_scroll(FrameworkKind::Nve);
            if sid < 6 {
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
                        timestamp_us: 1_000,
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
                        timestamp_us: 2_000,
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
                        timestamp_us: 3_000,
                    },
                );
                black_box((down.winner, mv.winner, up.winner))
            });
        });
    }

    fn bench_channel_bus(c: &mut Criterion) {
        let mut bus = NveChannelBus::new();
        let received = Arc::new(AtomicUsize::new(0));
        let received_cb = Arc::clone(&received);
        let _sub = bus
            .subscribe("bench.theme", Some(1), false, move |msg| {
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

    criterion_group!(benches, bench_gesture_arbitrator, bench_channel_bus);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
