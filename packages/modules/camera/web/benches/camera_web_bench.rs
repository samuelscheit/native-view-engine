//! Criterion micro-benchmarks for `nve-camera-web`: `CameraWebSystem` SSR emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_camera::{CameraFacing, CameraPreviewTable, CameraTorch};
    use nve_camera_web::CameraWebSystem;
    use nve_store::{ChangeSet, NodeId, NodeStore};

    fn bench_camera_web_ssr(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(16);
        for i in 0..16u16 {
            let n = tx.create();
            tx.edit::<CameraPreviewTable>(n)
                .device_id("cam-0")
                .facing(if i % 2 == 0 {
                    CameraFacing::Back
                } else {
                    CameraFacing::Front
                })
                .torch(CameraTorch::Auto)
                .fps(60)
                .zoom(1.5);
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(CameraWebSystem);

        c.bench_function("camera_web_ssr_render_16_previews", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_camera_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
