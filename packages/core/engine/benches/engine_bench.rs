//! Criterion benchmarks for `nve-engine`: Frame-0 `NVE2` keyframe encoding & footer/XXH64
//! validation plus atomic `StateFile` envelope round-trip.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_components::tables::{Interaction, Layout, TextContent, Typography, Visual};
    use nve_engine::{Frame0File, Frame0Meta, StateFile};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba};

    fn build_1k_store() -> NodeStore {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(1_000);
        for i in 0..1_000u32 {
            let mut nm = tx.spawn();
            nm.edit::<Layout>()
                .width(Length::points(240.0))
                .height(Length::points(36.0));
            nm.edit::<Visual>()
                .background(Rgba::rgb((i & 255) as u8, 41, 59))
                .border_radius(6.0)
                .opacity(1.0);
            if i % 2 == 0 {
                nm.edit::<Typography>()
                    .font_size(14.0)
                    .color(Rgba::rgb(241, 245, 249));
                nm.edit::<TextContent>().text("Bench Row Label");
            }
            if i % 8 == 0 {
                nm.edit::<Interaction>().event_mask(1);
            }
            children.push(nm.id());
        }
        tx.set_children(NodeId::ROOT, &children);
        tx.commit_into(&mut ChangeSet::default());
        store
    }

    fn bench_persistence(c: &mut Criterion) {
        let store = build_1k_store();
        let meta = Frame0Meta {
            table_schema_hash: nve_store::registered_tables_schema_hash(),
            app_build_id: 0x0001_0002_0003_0004,
            env_hash: 0x0005_0006_0007_0008,
        };
        let encoded_frame0 = Frame0File::encode(&store, meta);

        c.bench_function("frame0_encode_1k_nodes", |b| {
            b.iter(|| {
                let bytes = Frame0File::encode(black_box(&store), black_box(meta));
                black_box(bytes.len())
            });
        });

        c.bench_function("frame0_validate_1k_nodes", |b| {
            b.iter(|| {
                let (footer, payload) =
                    Frame0File::validate_bytes(black_box(&encoded_frame0), black_box(meta))
                        .expect("valid bench frame0");
                black_box((footer.node_count, payload.len()))
            });
        });

        let payload = vec![0xA5u8; 4096];
        c.bench_function("state_file_envelope_roundtrip_4k", |b| {
            b.iter(|| {
                let env = StateFile::encode_envelope(1, black_box(&payload))
                    .expect("valid state envelope");
                let decoded = StateFile::decode_envelope(black_box(&env), 1)
                    .expect("valid decoded state");
                black_box(decoded.len())
            });
        });
    }

    criterion_group!(benches, bench_persistence);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
