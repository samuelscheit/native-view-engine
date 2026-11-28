//! Criterion micro-benchmarks for `nve-canvas-web`: `CanvasWebSystem` SSR attribute emission for
//! recorded `DrawList` canvases and `DirectCanvasRegistry`-backed canvases.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::ColorRgba;
    use nve_backend_web::WebBackend;
    use nve_canvas::{Canvas, DirectCanvasRegistry, DrawList};
    use nve_canvas_web::CanvasWebSystem;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    fn bench_canvas_web_ssr(c: &mut Criterion) {
        let draw_list = DrawList::record(|p| {
            p.clear(ColorRgba::BLACK);
            for i in 0..32 {
                let x = (i as f32) * 8.0;
                p.fill_rect(x, 4.0, 6.0, 24.0, ColorRgba::rgb(40, 140, 240));
                p.fill_rounded_rect(x, 32.0, 6.0, 12.0, 2.0, ColorRgba::rgb(255, 255, 255));
            }
        });

        let direct = DirectCanvasRegistry::new();
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(16);
        for i in 0..16 {
            let n = tx.create();
            if i % 2 == 0 {
                tx.edit::<Canvas>(n).draw_list(Some(draw_list.clone()));
            } else {
                direct.bind(&mut tx, n, |p, size| {
                    p.fill_rect(0.0, 0.0, size.width, size.height, ColorRgba::rgb(12, 16, 24));
                });
            }
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut()
            .register(CanvasWebSystem::with_direct(direct));

        c.bench_function("canvas_web_ssr_render_16_mixed_canvases", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_canvas_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
