//! Criterion micro-benchmarks for `nve-web-element`: `WebElementAttrs` flat-word encode/validate
//! and `WebElement` store view iteration.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_store::{ChangeSet, FlatResource, NodeId, NodeStore};
    use nve_web_element::{WebElement, WebElementAttrs};

    fn bench_web_element_attrs(c: &mut Criterion) {
        let attrs = WebElementAttrs::from_pairs(vec![
            ("data-role".into(), "card".into()),
            ("data-theme".into(), "dark".into()),
            ("aria-label".into(), "Benchmark Card".into()),
            ("slot".into(), "header".into()),
        ]);
        let mut words = Vec::new();
        assert!(attrs.write_words(&mut words));

        c.bench_function("web_element_attrs_write_and_validate", |b| {
            let mut scratch = Vec::with_capacity(words.len());
            b.iter(|| {
                scratch.clear();
                let ok = attrs.write_words(&mut scratch);
                let valid = WebElementAttrs::validate(black_box(&scratch));
                black_box((ok, valid, scratch.len()))
            });
        });

        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let mut nodes = Vec::with_capacity(32);
        for _ in 0..32 {
            let n = tx.create();
            tx.edit::<WebElement>(n)
                .tag("nve-card")
                .attrs(Some(attrs.clone()));
            nodes.push(n);
        }
        tx.set_children(NodeId::ROOT, &nodes);
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        c.bench_function("web_element_read_32_rows_and_iter_attrs", |b| {
            b.iter(|| {
                let mut bytes = 0usize;
                for &n in &nodes {
                    if let Some(row) = store.try_row::<WebElement>(black_box(n))
                        && let Some(view) = store.resource::<WebElementAttrs>(row.attrs())
                    {
                        bytes += row.tag().len();
                        for (k, v) in view.iter() {
                            bytes += k.len() + v.len();
                        }
                    }
                }
                black_box(bytes)
            });
        });
    }

    criterion_group!(benches, bench_web_element_attrs);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
