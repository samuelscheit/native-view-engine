//! Tier-1 hot-reload differ benchmarks: `diff_rsx_source` latency (Criterion) and heap allocations
//! per diff (`CountingAlloc`) on the real `ui_chrome.rs` and a synthetic 5,000-line `rsx!` file.
//!
//! Guards: every scenario must classify as expected, and the 5k-line typed edit must diff in
//! `< 10 ms` (median of 50 runs) or the bench aborts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};

#[allow(
    dead_code,
    unused_imports,
    reason = "the bench drives only `diff_rsx_source` (the watch cache serves `nve dev`); \
              `cargo check --benches` sets cfg(test) without the test harness, so the module's \
              `#[test]` fns are stripped and their imports look unused"
)]
#[path = "../src/rsx_diff/mod.rs"]
mod rsx_diff;

// `cargo check --benches` compiles with cfg(test), where `rsx_diff` already declares the fixture.
#[cfg(not(test))]
#[path = "../src/rsx_diff/fixture.rs"]
mod fixture;
#[cfg(test)]
use rsx_diff::fixture;

use fixture::{replace_last, synthetic_rsx_source};
use rsx_diff::{DiffOutcome, diff_rsx_source};

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: Delegates directly to `std::alloc::System` while incrementing atomic counters.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::alloc` preconditions; delegated to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Caller upholds `GlobalAlloc::dealloc` preconditions; delegated to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: Caller upholds `GlobalAlloc::realloc` preconditions; delegated to `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expect {
    NoOp,
    Tier1,
    Tier2,
}

struct Scenario {
    name: &'static str,
    old: String,
    new: String,
    expect: Expect,
}

fn classify(outcome: &DiffOutcome) -> Expect {
    match outcome {
        DiffOutcome::NoOp => Expect::NoOp,
        DiffOutcome::Tier1Patch { .. } => Expect::Tier1,
        DiffOutcome::RequireTier2Rebuild { .. } => Expect::Tier2,
    }
}

fn scenarios() -> Vec<Scenario> {
    let ui_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/studio-workspace/src/ui_chrome.rs");
    let ui =
        std::fs::read_to_string(&ui_path).expect("read examples/studio-workspace ui_chrome.rs");
    let big = synthetic_rsx_source(5_000);
    let scenario = |name, old: &String, new: String, expect| Scenario {
        name,
        old: old.clone(),
        new,
        expect,
    };
    vec![
        scenario(
            "ui_chrome/text_edit",
            &ui,
            ui.replacen("\"LOCAL-FIRST | NVE\"", "\"LOCAL-FIRST | LIVE\"", 1),
            Expect::Tier1,
        ),
        scenario(
            "ui_chrome/style_edit",
            &ui,
            ui.replacen("gap: 8px;\",", "gap: 12px;\",", 1),
            Expect::Tier1,
        ),
        scenario(
            "ui_chrome/comment_noop",
            &ui,
            ui.replacen("rsx! {", "rsx! { // probe\n", 1),
            Expect::NoOp,
        ),
        scenario(
            "synthetic_5k/typed_edit_last",
            &big,
            replace_last(&big, "padding: 8.0,", "padding: px(-12.5),"),
            Expect::Tier1,
        ),
        scenario(
            "synthetic_5k/multi_typed_edit_last",
            &big,
            replace_last(
                &replace_last(&big, "FlexDirection::Row", "FlexDirection::Column"),
                "Rgba::rgb(15, 23, 42)",
                "hex(0x0f172aCC)",
            ),
            Expect::Tier1,
        ),
        scenario(
            "synthetic_5k/comment_noop",
            &big,
            replace_last(&big, "padding: 8.0,", "padding: 8.0, // probe"),
            Expect::NoOp,
        ),
        scenario(
            "synthetic_5k/logic_edit_last",
            &big,
            replace_last(&big, "count() * ", "count() + "),
            Expect::Tier2,
        ),
    ]
}

fn diff(s: &Scenario) -> DiffOutcome {
    diff_rsx_source(
        black_box("bench.rs"),
        black_box(&s.old),
        black_box(&s.new),
        black_box(1),
    )
}

/// Classification, allocation and latency guards, printed as a summary table.
fn verify_and_report(scenarios: &[Scenario]) {
    println!(
        "{:<36} {:>8} {:>8} {:>12} {:>10}",
        "scenario", "outcome", "allocs", "alloc bytes", "median"
    );
    for s in scenarios {
        let (count0, bytes0) = (
            ALLOC_COUNT.load(Ordering::SeqCst),
            ALLOC_BYTES.load(Ordering::SeqCst),
        );
        let outcome = diff(s);
        let allocs = ALLOC_COUNT.load(Ordering::SeqCst) - count0;
        let bytes = ALLOC_BYTES.load(Ordering::SeqCst) - bytes0;
        assert_eq!(classify(&outcome), s.expect, "{}: {outcome:?}", s.name);
        drop(outcome);

        let mut samples: Vec<Duration> = (0..50)
            .map(|_| {
                let t = Instant::now();
                black_box(diff(s));
                t.elapsed()
            })
            .collect();
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        println!(
            "{:<36} {:>8} {:>8} {:>12} {:>10.1?}",
            s.name,
            format!("{:?}", s.expect),
            allocs,
            bytes,
            median
        );
        if s.name.starts_with("synthetic_5k") {
            assert!(
                median < Duration::from_millis(10),
                "{}: median {median:?} >= 10 ms",
                s.name
            );
        }
    }
}

fn bench_rsx_diff(c: &mut Criterion) {
    let scenarios = scenarios();
    verify_and_report(&scenarios);
    let mut group = c.benchmark_group("rsx_diff");
    for s in &scenarios {
        group.bench_function(s.name, |b| b.iter(|| black_box(diff(s))));
    }
    group.finish();
}

criterion_group!(benches, bench_rsx_diff);
criterion_main!(benches);
