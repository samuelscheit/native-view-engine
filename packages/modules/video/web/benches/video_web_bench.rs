//! Criterion micro-benchmarks for `nve-video-web`: `VideoWebSystem` SSR emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_video::VideoPlayer;
    use nve_video_web::VideoWebSystem;

    fn bench_video_web_ssr(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(16);
        for _ in 0..16 {
            let n = tx.create();
            tx.edit::<VideoPlayer>(n)
                .source_uri("https://cdn.nve.dev/demo.mp4")
                .autoplay(true)
                .muted(true)
                .looping(true);
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(VideoWebSystem);

        c.bench_function("video_web_ssr_render_16_players", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_video_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
