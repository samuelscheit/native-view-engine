//! Criterion micro-benchmarks for `nve-web-element-web`: `WebElementWebSystem` SSR emission.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_web_element::{WebElement, WebElementAttrs};
    use nve_web_element_web::WebElementWebSystem;

    fn bench_web_element_web_ssr(c: &mut Criterion) {
        let attrs = WebElementAttrs::from_pairs(vec![
            ("data-role".into(), "card".into()),
            ("data-variant".into(), "subtle".into()),
            ("slot".into(), "body".into()),
        ]);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(32);
        for _ in 0..32 {
            let n = tx.create();
            tx.edit::<WebElement>(n)
                .tag("nve-card")
                .attrs(Some(attrs.clone()));
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        web.table_registry_mut().register(WebElementWebSystem);

        c.bench_function("web_element_web_ssr_render_32_elements", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_web_element_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
