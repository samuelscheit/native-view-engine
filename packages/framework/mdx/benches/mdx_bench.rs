//! Criterion benchmarks for `nve-mdx`: `compile_document`, `SearchIndex::encode`,
//! `SearchIndex::decode`, `SearchIndex::query`, and warm `SearchIndex::query_refs_into`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_mdx::{
        CompileOptions, SearchHitRef, SearchIndex, SearchQueryScratch, SiteSource,
        compile_document, compile_site,
    };

    const SAMPLE_MDX: &str = r#"---
title: CommandBuffer Architecture
description: Batched binary IR and zero-copy scene submission
order: 2
badge: v2
---

# CommandBuffer Architecture

Welcome to **{frontmatter.title}**. See [Routing](/guides/routing) and `NveSceneView`.

## Binary Scene Layout

| Backend | Latency | Notes |
| :--- | :---: | ---: |
| Skia | `0.4ms` | Direct canvas |
| Web | `0.5ms` | Batched DOM |

```rust title="src/frame.rs" highlight="1,3" showLineNumbers
pub fn submit_frame(buf: &[u8]) -> u32 {
    let len = buf.len() as u32;
    len
}
```

:::callout{kind="warning" title="ABI Invariant"}
Cross FFI boundaries via flat binary scenes only.
:::

<MetricCard label="Frame Budget" value={16} featured />
"#;

    fn build_bench_index() -> (SearchIndex, Vec<u8>) {
        let mut src = SiteSource::new();
        for i in 0..80 {
            src.insert(
                format!("guides/page-{i:03}.mdx"),
                format!(
                    "---\ntitle: Rendering Pipeline Module {i}\n---\n# Rendering Pipeline Module {i}\n\n## CommandBuffer Layout {i}\n\nUses `SurfaceOrchestrator`, `QuantumLayout`, and `NveSceneView` for batching frame {i}.\n"
                ),
            );
        }
        let site = compile_site(&src, &CompileOptions::default());
        let encoded = site.search_index.encode().expect("encode bench NVS2");
        (site.search_index, encoded)
    }

    fn bench_mdx(c: &mut Criterion) {
        let mut g = c.benchmark_group("mdx");

        g.bench_function("compile_document_rich_mdx", |b| {
            b.iter(|| {
                let page = compile_document(
                    black_box("/guides/ir"),
                    black_box("guides/ir.mdx"),
                    black_box(SAMPLE_MDX),
                );
                black_box(page.blocks.len())
            });
        });

        let (index, encoded) = build_bench_index();

        g.bench_function("search_index_encode_80_pages", |b| {
            b.iter(|| {
                let bytes = black_box(&index).encode().expect("encode NVS2");
                black_box(bytes.len())
            });
        });

        g.bench_function("search_index_decode_80_pages", |b| {
            b.iter(|| {
                let decoded = SearchIndex::decode(black_box(&encoded)).expect("decode NVS2");
                black_box(decoded.docs.len())
            });
        });

        g.bench_function("search_index_query_alloc", |b| {
            b.iter(|| {
                let hits = black_box(&index).query(black_box("SurfaceOrchestrator layout"), 10);
                black_box(hits.len())
            });
        });

        let mut scratch = SearchQueryScratch::for_index(&index);
        let mut hits: Vec<SearchHitRef<'_>> = Vec::with_capacity(16);
        // Warm once before measuring steady-state zero-alloc path.
        index.query_refs_into("SurfaceOrchestrator layout", 10, &mut scratch, &mut hits);

        g.bench_function("search_index_query_refs_into_warm", |b| {
            b.iter(|| {
                black_box(&index).query_refs_into(
                    black_box("SurfaceOrchestrator layout"),
                    10,
                    &mut scratch,
                    &mut hits,
                );
                black_box(hits.len())
            });
        });

        g.finish();
    }

    criterion_group!(benches, bench_mdx);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
