//! Zero-allocation heap-guard and scoring-equivalence suite for `nve-mdx` search queries.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;
use std::hint::black_box;
use std::sync::Mutex;

use nve_mdx::{
    CompileOptions, SearchHitRef, SearchIndex, SearchQueryScratch, SiteSource, compile_site,
};

struct CountingAlloc;

thread_local! {
    static ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[inline]
fn bump_alloc_count() {
    let _ = ALLOC_COUNT.try_with(|c| c.set(c.get().wrapping_add(1)));
}

#[inline]
fn thread_alloc_count() -> usize {
    ALLOC_COUNT.with(Cell::get)
}

#[inline]
fn measure_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = thread_alloc_count();
    let out = f();
    let after = thread_alloc_count();
    (out, after.wrapping_sub(before))
}

// SAFETY: Every allocation method forwards unchanged arguments to `System`, upholding `GlobalAlloc`.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `layout` is forwarded unchanged to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump_alloc_count();
        // SAFETY: `ptr`/`layout` came from a prior `System` allocation.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn build_corpus() -> SearchIndex {
    let mut src = SiteSource::new();
    src.insert(
        "title-hit.mdx",
        "---\ntitle: QuantumLayout Engine\n---\nGeneral overview of layout.\n",
    );
    src.insert(
        "heading-hit.mdx",
        "---\ntitle: Architecture\n---\n## QuantumLayout Pipeline\nDetails about pipeline scheduling.\n",
    );
    src.insert(
        "code-hit.mdx",
        "---\ntitle: Low Level API\n---\nUse `QuantumLayout` directly in Rust.\n",
    );
    src.insert(
        "body-hit.mdx",
        "---\ntitle: Notes\n---\nMentions quantumlayout and orchestrator in plain body prose.\n",
    );
    src.insert(
        "unicode-hit.mdx",
        "---\ntitle: Café Über Guide\n---\n## 日本語トークン\nSupports café, résumé, Über-layout, and `surface_orchestrator`.\n",
    );
    // Add 36 more pages (> 72 SearchDocs) so multi-doc sorting exceeds Rust's small-slice threshold
    // where a stable sort would allocate a merge scratch buffer.
    for i in 0..36 {
        src.insert(
            format!("guides/module-{i:02}.mdx"),
            format!(
                "---\ntitle: Layout Module {i}\n---\n# Layout Module {i}\n\n## Surface Pipeline {i}\n\nUses `CommandBuffer` and `SurfaceOrchestrator` for frame layout {i}.\n"
            ),
        );
    }
    let site = compile_site(&src, &CompileOptions::default());
    let encoded = site.search_index.encode().expect("encode corpus NVS2");
    SearchIndex::decode(&encoded).expect("decode corpus NVS2")
}

#[test]
fn counting_alloc_positive_probe() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let (nonzero, allocs) = measure_allocs(|| black_box(vec![1u8; 64]));
    drop(nonzero);
    assert!(
        allocs >= 1,
        "positive probe failed: Vec allocation must increment CountingAlloc (got {allocs})"
    );

    let (zeroed, zeroed_allocs) = measure_allocs(|| black_box(vec![0u8; 64]));
    drop(zeroed);
    assert!(
        zeroed_allocs >= 1,
        "positive probe failed: zeroed Vec allocation must increment CountingAlloc (got {zeroed_allocs})"
    );
}

#[test]
fn query_refs_into_is_zero_alloc_in_steady_state() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let index = build_corpus();
    assert!(
        index.docs.len() > 60,
        "corpus must exceed 60 docs to stress in-place sort"
    );

    let mut scratch = SearchQueryScratch::for_index(&index);
    let mut hits: Vec<SearchHitRef<'_>> = Vec::with_capacity(32);

    // Warm once across representative multi-term prefix, trigram, and Unicode queries.
    let queries = [
        "QuantumLayout",
        "quant pipe",
        "uantum orches",
        "layout surface pipeline",
        "Café Über! 日本語",
        "surface_orchestrator frame",
        "nonexistent_zzzz_term",
        "   !!!   ",
    ];
    for q in queries {
        index.query_refs_into(q, 25, &mut scratch, &mut hits);
    }

    let (total_hits, allocs) = measure_allocs(|| {
        let mut total = 0usize;
        for q in black_box(queries) {
            index.query_refs_into(black_box(q), 25, &mut scratch, &mut hits);
            total += black_box(hits.len());
        }
        // Also verify `limit = 0` and repeated identical terms perform zero allocations.
        index.query_refs_into(black_box("layout layout"), 10, &mut scratch, &mut hits);
        total += black_box(hits.len());
        index.query_refs_into(black_box("layout"), 0, &mut scratch, &mut hits);
        total += black_box(hits.len());
        total
    });

    assert_eq!(
        allocs, 0,
        "Warm SearchIndex::query_refs_into must perform 0 heap allocs across prefix+trigram+Unicode queries, did {allocs}"
    );
    assert!(total_hits > 50, "expected real multi-doc matches, got {total_hits}");
}

#[test]
fn query_refs_into_matches_query_identically_and_preserves_ranking_contract() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let index = build_corpus();
    let mut scratch = SearchQueryScratch::new();
    let mut refs: Vec<SearchHitRef<'_>> = Vec::new();

    let test_cases = [
        ("QuantumLayout", 10),
        ("quant", 10),
        ("uantum", 10),
        ("SurfaceOrchestrator layout", 20),
        ("Café Über", 10),
        ("日本語", 10),
        ("a layout b", 10),
        ("layout!@#$%^&*()-+=pipeline", 15),
        ("surface_orchestrator", 10),
        ("no-match-anywhere", 10),
        ("", 10),
        ("layout", 0),
    ];

    for (q, limit) in test_cases {
        let owned = index.query(q, limit);
        index.query_refs_into(q, limit, &mut scratch, &mut refs);
        assert_eq!(
            refs.len(),
            owned.len(),
            "hit count mismatch for query {q:?}"
        );
        for (r, o) in refs.iter().zip(owned.iter()) {
            assert_eq!(r.route, o.route, "route mismatch for {q:?}");
            assert_eq!(r.title, o.title, "title mismatch for {q:?}");
            assert_eq!(r.section, o.section, "section mismatch for {q:?}");
            assert_eq!(r.anchor, o.anchor, "anchor mismatch for {q:?}");
            assert_eq!(r.excerpt, o.excerpt, "excerpt mismatch for {q:?}");
            assert_eq!(
                r.score.to_bits(),
                o.score.to_bits(),
                "exact f32 score bit mismatch for {q:?}"
            );
            assert_eq!(r.to_owned_hit(), *o);
        }
    }

    // Independent mathematical contract check: title (4.0 * 2.0 = 8.0) > heading (2.5 * 2.0 = 5.0)
    // > code (2.0 * 2.0 = 4.0) > body (1.0 * 2.0 = 2.0).
    index.query_refs_into("QuantumLayout", 10, &mut scratch, &mut refs);
    let routes: Vec<&str> = refs.iter().map(|h| h.route).collect();
    assert_eq!(
        &routes[..4],
        &["/title-hit", "/heading-hit", "/code-hit", "/body-hit"]
    );
    assert!((refs[0].score - 8.0).abs() < 1e-6);
    assert!((refs[1].score - 5.0).abs() < 1e-6);
    assert!((refs[2].score - 4.0).abs() < 1e-6);
    assert!((refs[3].score - 2.0).abs() < 1e-6);

    // Multi-term all-matched bonus (`1.25x`): exact title `quantumlayout` (8.0) + exact body `layout` (2.0) = 10.0 * 1.25 = 12.5 on `/title-hit`.
    index.query_refs_into("QuantumLayout overview", 10, &mut scratch, &mut refs);
    assert_eq!(refs[0].route, "/title-hit");
    assert!((refs[0].score - 12.5).abs() < 1e-6);

    // Trigram-only substring hit (`uantum` is interior substring of `quantumlayout`, 4/4 trigrams -> 4.0 * 0.45 * 1.0 = 1.8 on `/title-hit`).
    index.query_refs_into("uantum", 10, &mut scratch, &mut refs);
    assert_eq!(refs[0].route, "/title-hit");
    assert!((refs[0].score - 1.8).abs() < 1e-6);
}
