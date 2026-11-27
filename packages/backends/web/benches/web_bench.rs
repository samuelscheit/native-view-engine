//! Criterion benchmarks for `nve-backend-web`: SSR HTML5 + deduplicated CSS emission over
//! `NodeStore` and `NVB1` base64 state envelope encode/decode.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::snapshot::{decode_state_envelope, encode_state_envelope};
    use nve_backend_web::WebBackend;
    use nve_components::tables::{Layout, TextContent, Visual};
    use nve_engine::{StoreBackend, Viewport};
    use nve_store::{ChangeSet, Length, NodeId, NodeStore, Rgba, SurfaceId};

    fn bench_web_ssr_and_snapshot(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        {
            let mut tx = store.tx();
            let nodes: Vec<NodeId> = (0..128)
                .map(|i| {
                    let mut nm = tx.spawn();
                    nm.edit::<Layout>()
                        .width(Length::points(280.0))
                        .height(Length::points(40.0));
                    nm.edit::<Visual>()
                        .background(Rgba::rgb(15, 23, 42))
                        .border_radius(8.0);
                    if i % 2 == 0 {
                        nm.edit::<TextContent>().text("SSR Card Label");
                    }
                    nm.id()
                })
                .collect();
            tx.set_children(NodeId::ROOT, &nodes);
            tx.commit_into(&mut changes);
        }

        let mut backend = WebBackend::new();
        backend.set_viewport(
            SurfaceId::ROOT,
            Viewport {
                width: 1280.0,
                height: 720.0,
                scale_factor: 1.0,
            },
        );
        backend
            .commit(&store, &changes)
            .expect("initial web bench commit");

        c.bench_function("web_render_ssr_128_nodes", |b| {
            b.iter(|| {
                let ssr = backend.render_ssr(black_box(&store));
                black_box((ssr.html.len(), ssr.style_css.len()))
            });
        });

        let state_bytes = vec![0x42u8; 4096];
        let encoded = encode_state_envelope(2, &state_bytes);
        c.bench_function("web_snapshot_state_envelope_4kb_roundtrip", |b| {
            b.iter(|| {
                let enc = encode_state_envelope(2, black_box(&state_bytes));
                let dec = decode_state_envelope(black_box(&encoded), 2)
                    .expect("valid bench state envelope");
                black_box((enc.len(), dec.len()))
            });
        });
    }

    criterion_group!(benches, bench_web_ssr_and_snapshot);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
