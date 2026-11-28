//! Criterion micro-benchmarks for `nve-abi` (fixed-point number codec and
//! `native_events_to_direct` conversion).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::num::{parse_f32, write_f32};
    use nve_abi::{
        native_events_to_direct, EventPayload, NativeEvent, NativeEventKind,
    };

    fn bench_num_codec(c: &mut Criterion) {
        let values = [0.0f32, 12.0, 0.5, -3.125, 1920.5, 0.333, 99.999, 42.25];
        let inputs = ["0", "12", "0.5", "-3.125", "1920.5", "0.333", "99.999", "42.25"];
        let mut out = String::with_capacity(128);

        c.bench_function("num_write_f32_8_values", |b| {
            b.iter(|| {
                out.clear();
                for &v in black_box(&values) {
                    write_f32(&mut out, v);
                    out.push(',');
                }
                black_box(out.len())
            });
        });

        c.bench_function("num_parse_f32_8_values", |b| {
            b.iter(|| {
                let mut sum = 0.0f32;
                for &s in black_box(&inputs) {
                    sum += parse_f32(s).expect("valid bench float literal");
                }
                black_box(sum)
            });
        });
    }

    fn bench_direct_event_conversion(c: &mut Criterion) {
        let events: Vec<NativeEvent> = (0..32u32)
            .map(|i| NativeEvent {
                surface_id: 1,
                node_id: i + 1,
                timestamp_us: u64::from(i) * 1000,
                kind: NativeEventKind::Press,
                payload: EventPayload::Point {
                    x: i as f32 * 10.0,
                    y: i as f32 * 5.0,
                },
            })
            .collect();
        let mut direct = Vec::with_capacity(32);

        c.bench_function("native_events_to_direct_32", |b| {
            b.iter(|| {
                direct.clear();
                native_events_to_direct(black_box(&events), &mut direct);
                black_box(direct.len())
            });
        });
    }

    criterion_group!(
        benches,
        bench_num_codec,
        bench_direct_event_conversion
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
