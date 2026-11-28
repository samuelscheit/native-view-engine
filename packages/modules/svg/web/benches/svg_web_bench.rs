//! Criterion micro-benchmarks for `nve-svg-web`: `SvgWebSystem` SSR attribute and inline SVG
//! element emission across a 32-icon `NodeStore`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_store::{ChangeSet, NodeId, NodeStore, Rgba};
    use nve_svg::{Svg, SvgPicture};
    use nve_svg_web::SvgWebSystem;

    fn bench_svg_web_ssr(c: &mut Criterion) {
        let pic = SvgPicture::from_paths(
            24.0,
            24.0,
            &["M12 2 L15 9 L22 9 L16 14 L18 21 L12 17 L6 21 L8 14 L2 9 L9 9 Z"],
        );
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(32);
        for i in 0..32u8 {
            let n = tx.create();
            tx.edit::<Svg>(n)
                .picture(Some(pic.clone()))
                .tint(Some(Rgba::rgb(i.wrapping_mul(7), 120, 240)))
                .mirror_directional(i % 2 == 0);
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(SvgWebSystem);

        c.bench_function("svg_web_ssr_render_32_icons", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_svg_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
