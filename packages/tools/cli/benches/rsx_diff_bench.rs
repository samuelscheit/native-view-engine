//! Tier-1 hot-reload differ benchmarks: latency (Criterion) and heap allocations (`CountingAlloc`)
//! per diff on the real `ui_chrome.rs` and a synthetic 5,000-line `rsx!` file, for the cold path
//! (fresh baseline: lex + index both sides) and the warm `nve dev` path (cached baseline tokens and
//! index: lex the save only).
//!
//! Guards (the bench aborts otherwise): every scenario classifies as expected, stays within its
//! allocation-count and allocated-byte budget, and every 5k-line median is `< 10 ms`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

#[allow(
    dead_code,
    unused_imports,
    reason = "the bench drives only `Baseline` (the watch cache serves `nve dev`); \
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
use rsx_diff::DiffOutcome;
use rsx_diff::cache::Baseline;

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

/// Allocation budget of one diff: `(count, bytes)`.
type Budget = (usize, usize);

struct Scenario {
    name: &'static str,
    old: String,
    new: String,
    expect: Expect,
    cold: Budget,
    warm: Budget,
}

fn classify(outcome: &DiffOutcome) -> Expect {
    match outcome {
        DiffOutcome::NoOp => Expect::NoOp,
        DiffOutcome::Tier1Patch { .. } => Expect::Tier1,
        DiffOutcome::RequireTier2Rebuild { .. } => Expect::Tier2,
    }
}

const KB: usize = 1024;

fn scenarios() -> Vec<Scenario> {
    let ui_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples/studio-workspace/src/ui_chrome.rs");
    let ui =
        std::fs::read_to_string(&ui_path).expect("read examples/studio-workspace ui_chrome.rs");
    let big = synthetic_rsx_source(5_000);
    let scenario = |name, old: &String, new: String, expect, cold, warm| Scenario {
        name,
        old: old.clone(),
        new,
        expect,
        cold,
        warm,
    };
    vec![
        scenario(
            "ui_chrome/text_edit",
            &ui,
            ui.replacen("\"LOCAL-FIRST | NVE\"", "\"LOCAL-FIRST | LIVE\"", 1),
            Expect::Tier1,
            (32, 160 * KB),
            (24, 80 * KB),
        ),
        scenario(
            "ui_chrome/comment_noop",
            &ui,
            ui.replacen("rsx! {", "rsx! { // probe\n", 1),
            Expect::NoOp,
            (14, 128 * KB),
            (7, 64 * KB),
        ),
        scenario(
            "synthetic_5k/typed_edit_last",
            &big,
            replace_last(&big, "padding: 8.0,", "padding: -12.5,"),
            Expect::Tier1,
            (40, 1_400 * KB),
            (24, 640 * KB),
        ),
        scenario(
            "synthetic_5k/multi_edit_last",
            &big,
            replace_last(
                &replace_last(
                    &replace_last(&big, "FlexDirection::Row", "FlexDirection::Column"),
                    "ColorRgba::rgb(15, 23, 42)",
                    "ColorRgba::rgb(15, 23, 204)",
                ),
                "\"Press ",
                "\"Tap ",
            ),
            Expect::Tier1,
            (64, 1_400 * KB),
            (48, 640 * KB),
        ),
        scenario(
            "synthetic_5k/comment_noop",
            &big,
            replace_last(&big, "padding: 8.0,", "padding: 8.0, // probe"),
            Expect::NoOp,
            (14, 1_200 * KB),
            (7, 640 * KB),
        ),
        scenario(
            "synthetic_5k/logic_edit_last",
            &big,
            replace_last(&big, "count() * ", "count() + "),
            Expect::Tier2,
            (28, 1_400 * KB),
            (12, 640 * KB),
        ),
    ]
}

/// A baseline whose tokens and index are already cached, as after the first save in `nve dev`.
fn warm_baseline(s: &Scenario) -> Baseline {
    let mut baseline = Baseline::new(s.old.clone());
    let _ = baseline.classify("bench.rs", &s.new, 1);
    baseline
}

fn diff(baseline: &mut Baseline, s: &Scenario) -> DiffOutcome {
    baseline
        .classify(black_box("bench.rs"), black_box(&s.new), black_box(1))
        .0
}

fn measure(f: impl FnOnce() -> DiffOutcome) -> (DiffOutcome, Budget) {
    let (count, bytes) = (
        ALLOC_COUNT.load(Ordering::SeqCst),
        ALLOC_BYTES.load(Ordering::SeqCst),
    );
    let outcome = f();
    let used = (
        ALLOC_COUNT.load(Ordering::SeqCst) - count,
        ALLOC_BYTES.load(Ordering::SeqCst) - bytes,
    );
    (outcome, used)
}

fn median(mut run: impl FnMut() -> Duration) -> Duration {
    let mut samples: Vec<Duration> = (0..50).map(|_| run()).collect();
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// Classification, allocation and latency guards, printed as a summary table.
fn verify_and_report(scenarios: &[Scenario]) {
    println!(
        "{:<34} {:>6} {:>12} {:>14} {:>12} {:>14} {:>10} {:>10}",
        "scenario",
        "class",
        "cold allocs",
        "cold bytes",
        "warm allocs",
        "warm bytes",
        "cold",
        "warm"
    );
    for s in scenarios {
        let (cold_outcome, cold) = {
            let mut baseline = Baseline::new(s.old.clone());
            measure(|| diff(&mut baseline, s))
        };
        let mut baseline = warm_baseline(s);
        let (warm_outcome, warm) = measure(|| diff(&mut baseline, s));
        assert_eq!(
            classify(&cold_outcome),
            s.expect,
            "{}: {cold_outcome:?}",
            s.name
        );
        assert_eq!(
            warm_outcome, cold_outcome,
            "{}: warm and cold paths disagree",
            s.name
        );

        let cold_median = median(|| {
            let mut fresh = Baseline::new(s.old.clone());
            let t = Instant::now();
            black_box(diff(&mut fresh, s));
            t.elapsed()
        });
        let warm_median = median(|| {
            let t = Instant::now();
            black_box(diff(&mut baseline, s));
            t.elapsed()
        });
        println!(
            "{:<34} {:>6} {:>12} {:>14} {:>12} {:>14} {:>10.1?} {:>10.1?}",
            s.name,
            format!("{:?}", s.expect),
            cold.0,
            cold.1,
            warm.0,
            warm.1,
            cold_median,
            warm_median
        );
        for (path, used, budget) in [("cold", cold, s.cold), ("warm", warm, s.warm)] {
            assert!(
                used.0 <= budget.0 && used.1 <= budget.1,
                "{} {path}: {} allocs / {} B exceed the budget of {} allocs / {} B",
                s.name,
                used.0,
                used.1,
                budget.0,
                budget.1
            );
        }
        if s.name.starts_with("synthetic_5k") {
            assert!(
                cold_median < Duration::from_millis(10),
                "{}: cold median {cold_median:?} >= 10 ms",
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
        group.bench_function(format!("{}/cold", s.name), |b| {
            b.iter_batched(
                || Baseline::new(s.old.clone()),
                |mut baseline| black_box(diff(&mut baseline, s)),
                BatchSize::SmallInput,
            );
        });
        let mut baseline = warm_baseline(s);
        group.bench_function(format!("{}/warm", s.name), |b| {
            b.iter(|| black_box(diff(&mut baseline, s)));
        });
    }
    // Dedicated benchmark for `TypedRsxPatch -> RowPatch` binary flat word encoding.
    let multi = &scenarios[3];
    let mut multi_baseline = Baseline::new(multi.old.clone());
    let sample_patch = match multi_baseline.classify("bench.rs", &multi.new, 1).0 {
        DiffOutcome::Tier1Patch { patch, .. } => Some(patch),
        _ => None,
    }
    .expect("expected Tier1Patch for multi_edit_last");
    let (encoded, enc_allocs) = {
        let c0 = ALLOC_COUNT.load(Ordering::SeqCst);
        let b0 = ALLOC_BYTES.load(Ordering::SeqCst);
        let msgs = sample_patch
            .to_template_patches("bench.rs", 2)
            .expect("RowPatch word encode");
        (
            msgs,
            (
                ALLOC_COUNT.load(Ordering::SeqCst) - c0,
                ALLOC_BYTES.load(Ordering::SeqCst) - b0,
            ),
        )
    };
    assert!(
        !encoded.is_empty() && enc_allocs.0 <= 24 && enc_allocs.1 <= 4096,
        "RowPatch word encode budget exceeded: {} allocs / {} B",
        enc_allocs.0,
        enc_allocs.1
    );
    println!(
        "{:<34} {:>6} {:>12} {:>14}",
        "row_patch_encode/multi_edit", "Tier1", enc_allocs.0, enc_allocs.1
    );
    group.bench_function("row_patch_encode/multi_edit", |b| {
        b.iter(|| {
            black_box(
                sample_patch
                    .to_template_patches(black_box("bench.rs"), black_box(2))
                    .expect("RowPatch word encode"),
            )
        });
    });
    group.finish();
}

criterion_group!(benches, bench_rsx_diff);
criterion_main!(benches);
