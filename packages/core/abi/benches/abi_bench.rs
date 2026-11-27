//! Criterion micro-benchmarks for `nve-abi` (`NveSceneTable`, fixed-point number codec, and
//! `EventBatchBuf` / `ValidatedEvents` batching).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::num::{parse_f32, write_f32};
    use nve_abi::{
        EventBatchBuf, FrameId, NodeId, NveSceneTable, PrimarySlot, SceneNodeInput, SurfaceId,
        TimestampUs, ValidatedEvents,
    };

    fn bench_scene_table(c: &mut Criterion) {
        let mut table = NveSceneTable::new();
        let mut frame = FrameId::ZERO;
        c.bench_function("scene_table_upsert_256_nodes", |b| {
            b.iter(|| {
                frame = frame.next();
                table.begin_frame(frame);
                for id in 1..=256u32 {
                    table
                        .upsert_node(SceneNodeInput {
                            node_id: NodeId::new(id, 0),
                            primary_slot: PrimarySlot::NONE,
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
        let mut buf = EventBatchBuf::with_capacity(32, 256);

        c.bench_function("event_batch_push_and_validate_32", |b| {
            b.iter(|| {
                buf.clear();
                for i in 0..32u32 {
                    buf.push_press(
                        SurfaceId::ROOT,
                        NodeId::new(i + 1, 0),
                        TimestampUs(u64::from(i) * 1000),
                        i as f32 * 10.0,
                        i as f32 * 5.0,
                    );
                }
                let raw = buf.as_raw_batch();
                // SAFETY: `buf` stays alive for this validation pass.
                let validated = unsafe { ValidatedEvents::try_from_raw(raw) }
                    .expect("valid bench event batch");
                black_box(validated.len())
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
