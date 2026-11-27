//! Criterion micro-benchmarks for `nve-abi` (`NveSceneTable`, fixed-point number codec, and
//! `native_events_to_direct` conversion).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::num::{parse_f32, write_f32};
    use nve_abi::{
        native_events_to_direct, EventPayload, NativeEvent, NativeEventKind, NveSceneTable,
        SceneNodeInput, NVE_PRIMARY_SLOT_NONE,
    };

    fn bench_scene_table(c: &mut Criterion) {
        let mut table = NveSceneTable::new();
        let mut frame = 0u64;
        c.bench_function("scene_table_upsert_256_nodes", |b| {
            b.iter(|| {
                frame = frame.wrapping_add(1);
                table.begin_frame(frame);
                for id in 1..=256u32 {
                    table
                        .upsert_node(SceneNodeInput {
                            node_id: id,
                            primary_slot: NVE_PRIMARY_SLOT_NONE,
                            x: (id as f32) * 4.0,
                            y: (id as f32) * 2.0,
                            width: 120.0,
                            height: 36.0,
                            ..SceneNodeInput::default()
                        })
                        .expect("valid bench scene node upsert");
                }
                black_box(table.as_view().node_count)
            });
        });
    }

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
        bench_scene_table,
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
