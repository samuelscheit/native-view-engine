//! Criterion micro-benchmarks for `nve-webview-web`: `WebViewWebSystem` SSR emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_webview::WebView;
    use nve_webview_web::WebViewWebSystem;

    fn bench_webview_web_ssr(c: &mut Criterion) {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(16);
        for _ in 0..16 {
            let n = tx.create();
            tx.edit::<WebView>(n).url("https://nve.dev/docs");
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(WebViewWebSystem);

        c.bench_function("webview_web_ssr_render_16_iframes", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_webview_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
