//! Anti-stub benchmark quality (enforced immediately today) plus staged per-crate benchmark
//! and heap-allocation (`GlobalAlloc`/`CountingAlloc`) coverage invariants.

use crate::support::{
    assert_no_violations, assert_staged_violations, contains_ident, repo_index, Violation,
};
use crate::Stage;

/// Enforces [INV:bench-and-alloc-guards].
/// Active immediately today: every existing `benches/*.rs` file across `packages/` must use a
/// real benchmark harness AND `black_box` (zero empty/no-op benchmark stubs).
#[test]
fn existing_benchmarks_are_not_stubs() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut grouped_targets: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    // Group Cargo benchmark targets: `.../benches/<name>.rs` as individual targets, and
    // `.../benches/<name>/*.rs` (directory bench targets with `main.rs`) combined per target.
    for rel in idx.rs_files_under("packages") {
        let Some(benches_pos) = rel.find("/benches/") else {
            continue;
        };
        let after = &rel[benches_pos + "/benches/".len()..];
        let target_key = if let Some((dir, _rest)) = after.split_once('/') {
            format!("{}/benches/{dir}/main.rs", &rel[..benches_pos])
        } else {
            rel.clone()
        };
        if let Some(cached) = idx.read_rust(&rel) {
            let entry = grouped_targets.entry(target_key).or_default();
            entry.push_str(&cached.all_clean);
            entry.push('\n');
        }
    }

    assert!(
        !grouped_targets.is_empty(),
        "expected existing benchmark targets under packages/**/benches/"
    );
    for (target, combined_clean) in &grouped_targets {
        if let Some(reason) = check_bench_not_stub(combined_clean) {
            violations.push(Violation::new(target, 1, target, reason));
        }
    }
    assert_no_violations(
        "All existing `benches/` targets are real benchmarks (anti-stub)",
        &violations,
    );
}

pub fn check_bench_not_stub(clean_src: &str) -> Option<&'static str> {
    let has_runner = clean_src.contains("criterion_main!")
        || clean_src.contains("criterion_group!")
        || contains_ident(clean_src, "Criterion")
        || contains_ident(clean_src, "divan")
        || clean_src.contains("Instant::now");
    let has_black_box = contains_ident(clean_src, "black_box");
    match (has_runner, has_black_box) {
        (false, _) => Some(
            "Benchmark stub detected: missing real benchmark runner (`criterion_main!`/`criterion_group!`/`Criterion`/`divan`/`Instant::now`)",
        ),
        (true, false) => Some(
            "Benchmark missing `black_box`: LLVM may optimize measured work away",
        ),
        (true, true) => None,
    }
}

#[test]
fn crates_have_benchmarks_and_allocation_guards() {
    let idx = repo_index();
    let mut v_nodestore_bench = Vec::new();
    let mut v_all_bench = Vec::new();
    let mut v_alloc_guard = Vec::new();
    let mut scanned_crates = 0usize;

    for pkg in idx.graph.workspace_packages() {
        if pkg.is_proc_macro || pkg.rel_dir == "packages/tools/bench" {
            continue;
        }
        let is_core_or_backend_ns = matches!(
            pkg.rel_dir.as_str(),
            "packages/core/abi"
                | "packages/core/hash"
                | "packages/core/store"
                | "packages/core/engine"
        ) || pkg.rel_dir.starts_with("packages/backends/");

        let in_bench_scope = pkg.rel_dir.starts_with("packages/core/")
            || pkg.rel_dir.starts_with("packages/backends/")
            || pkg.rel_dir.starts_with("packages/framework/")
            || pkg.rel_dir.starts_with("packages/modules/")
            || pkg.rel_dir == "packages/tools/cli";

        let in_alloc_scope = pkg.rel_dir.starts_with("packages/core/")
            || pkg.rel_dir.starts_with("packages/backends/")
            || pkg.rel_dir.starts_with("packages/framework/");

        if !in_bench_scope {
            continue;
        }
        scanned_crates += 1;

        let bench_files = idx.rs_files_under(&format!("{}/benches", pkg.rel_dir));
        let has_real_bench = pkg.has_bench_target
            || bench_files.iter().any(|rel| {
                idx.read_rust(rel)
                    .is_some_and(|c| check_bench_not_stub(&c.all_clean).is_none())
            });
        if !has_real_bench {
            let v = Violation::new(
                &pkg.rel_manifest,
                1,
                &pkg.rel_dir,
                "Crate must ship at least one real colocated benchmark (`[[bench]]` or non-stub `benches/*.rs`)",
            );
            if is_core_or_backend_ns {
                v_nodestore_bench.push(v.clone());
            }
            v_all_bench.push(v);
        }

        if in_alloc_scope {
            let mut has_alloc_guard = false;
            for sub in ["tests", "benches"] {
                for rel in idx.rs_files_under(&format!("{}/{sub}", pkg.rel_dir)) {
                    if let Some(cached) = idx.read_rust(&rel)
                        && (contains_ident(&cached.all_clean, "GlobalAlloc")
                            || contains_ident(&cached.all_clean, "CountingAlloc"))
                    {
                        has_alloc_guard = true;
                        break;
                    }
                }
                if has_alloc_guard {
                    break;
                }
            }
            if !has_alloc_guard {
                v_alloc_guard.push(Violation::new(
                    &pkg.rel_manifest,
                    1,
                    &pkg.rel_dir,
                    "Crate under packages/{core,backends,framework}/* must ship at least one `GlobalAlloc`/`CountingAlloc` heap-allocation guard in `tests/` or `benches/`",
                ));
            }
        }
    }

    assert!(
        scanned_crates > 0,
        "benchmark/alloc scanner must inspect workspace crates"
    );
    assert_staged_violations(
        Stage::PostNodeStore,
        "Core (`abi`/`hash`/`store`/`engine`) and backend crates ship colocated benchmarks",
        &v_nodestore_bench,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "All non-proc-macro crates under packages/{core,backends,framework,modules}/* + cli ship colocated benchmarks",
        &v_all_bench,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "All non-proc-macro crates under packages/{core,backends,framework}/* ship a GlobalAlloc/CountingAlloc guard",
        &v_alloc_guard,
    );
}
