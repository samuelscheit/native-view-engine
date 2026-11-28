//! Criterion micro-benchmarks for `nve-web-html-web`: full `register_web_html_systems` SSR pass.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_backend_web::WebBackend;
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_web_html::{
        ClosedBy, DataAttrList, HtmlDataAttrs, HtmlInvoker, HtmlSemantics, PopoverMode,
        SemanticElement, WebHtml,
    };
    use nve_web_html_web::register_web_html_systems;

    fn bench_web_html_web_ssr(c: &mut Criterion) {
        let data_attrs = DataAttrList::from_pairs(vec![
            ("data-slot".into(), "panel".into()),
            ("data-state".into(), "open".into()),
        ]);
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut children = Vec::with_capacity(32);
        for i in 0..32usize {
            let n = tx.create();
            tx.edit::<HtmlSemantics>(n)
                .element(Some(if i % 2 == 0 {
                    SemanticElement::Dialog
                } else {
                    SemanticElement::Nav
                }))
                .open(i % 2 == 0)
                .closed_by(Some(ClosedBy::Any))
                .popover(Some(PopoverMode::Auto));
            tx.edit::<HtmlInvoker>(n)
                .popover_target("menu-popover")
                .command("toggle-popover");
            tx.edit::<HtmlDataAttrs>(n).entries(Some(data_attrs.clone()));
            if i % 4 == 0 {
                tx.edit::<WebHtml>(n).tag("section").html("<span>ok</span>");
            }
            children.push(n);
        }
        tx.set_children(NodeId::ROOT, &children);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let mut web = WebBackend::new();
        register_web_html_systems(web.table_registry_mut());

        c.bench_function("web_html_web_ssr_render_32_nodes", |b| {
            b.iter(|| {
                let html = web.render_html(black_box(&store));
                black_box(html.len())
            });
        });
    }

    criterion_group!(benches, bench_web_html_web_ssr);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
