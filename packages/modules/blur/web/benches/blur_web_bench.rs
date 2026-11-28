//! Criterion micro-benchmarks for `nve-blur-web`: `BlurWebSystem` SSR backdrop-filter emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_blur::{BlurMaterial, BlurSurface};
    use nve_blur_web::BlurWebSystem;
    use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};

    fn bench_blur_web_ssr(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(32);
        for i in 0..32u8 {
            let n = tx.create();
            tx.edit::<BlurSurface>(n)
                .material(if i % 2 == 0 {
                    BlurMaterial::Regular
                } else {
                    BlurMaterial::UltraThin
                })
                .intensity(0.85)
                .tint(Some(Rgba::new(20, 24, 32, 96)));
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(BlurWebSystem);

        c.bench_function("blur_web_ssr_render_32_surfaces", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_blur_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
