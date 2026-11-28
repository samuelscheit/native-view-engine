//! Criterion micro-benchmarks for `nve-web-html`: `SemanticElement::parse` and HTML5 table
//! store reads/writes.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_store::{ChangeSet, NodeId, NodeStore};
    use nve_web_html::{
        ClosedBy, HtmlInvoker, HtmlSemantics, PopoverMode, SemanticElement, WebHtml,
    };

    const SAMPLE_TAGS: &[&str] = &[
        "dialog", "details", "summary", "form", "nav", "main", "article", "section", "table",
        "thead", "tbody", "tr", "th", "td", "button", "label", "unknown",
    ];

    fn bench_web_html_spec(c: &mut Criterion) {
        c.bench_function("web_html_semantic_element_parse_17_tags", |b| {
            b.iter(|| {
                let mut hits = 0usize;
                for &tag in SAMPLE_TAGS {
                    if SemanticElement::parse(black_box(tag)).is_some() {
                        hits += 1;
                    }
                }
                black_box(hits)
            });
        });

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut nodes = Vec::with_capacity(64);
        for i in 0..64usize {
            let n = tx.create();
            tx.edit::<HtmlSemantics>(n)
                .element(Some(if i % 2 == 0 {
                    SemanticElement::Dialog
                } else {
                    SemanticElement::Section
                }))
                .open(i % 4 == 0)
                .closed_by(Some(ClosedBy::Any))
                .popover(Some(PopoverMode::Auto));
            tx.edit::<HtmlInvoker>(n)
                .popover_target("menu-popover")
                .command("toggle-popover");
            tx.edit::<WebHtml>(n)
                .tag("article")
                .html("<p>Hello NVE</p>");
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        c.bench_function("web_html_read_64_rows", |b| {
            b.iter(|| {
                let mut acc = 0usize;
                for &n in &nodes {
                    if let Some(s) = store.try_row::<HtmlSemantics>(black_box(n))
                        && let Some(el) = s.element()
                    {
                        acc += el.as_html_tag().len();
                    }
                    if let Some(inv) = store.try_row::<HtmlInvoker>(n) {
                        acc += inv.command().len();
                    }
                    if let Some(raw) = store.try_row::<WebHtml>(n) {
                        acc += raw.html().len();
                    }
                }
                black_box(acc)
            });
        });
    }

    criterion_group!(benches, bench_web_html_spec);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
