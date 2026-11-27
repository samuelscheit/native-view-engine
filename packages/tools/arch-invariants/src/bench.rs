//! Per-crate benchmark (`[[bench]]`) and heap-allocation (`GlobalAlloc`/`CountingAlloc`) coverage.

use std::collections::BTreeMap;

use guppy::graph::BuildTargetId;

use crate::ast_layout::file_contains_ident;
use crate::{RepoIndex, Violation};

pub fn check_bench_not_stub(src: &str) -> Option<&'static str> {
    let has_runner = file_contains_ident(src, "criterion_main")
        || file_contains_ident(src, "criterion_group")
        || file_contains_ident(src, "Criterion")
        || file_contains_ident(src, "divan")
        || (file_contains_ident(src, "Instant") && file_contains_ident(src, "now"));
    let has_black_box = file_contains_ident(src, "black_box");
    match (has_runner, has_black_box) {
        (false, _) => Some("Benchmark stub detected: missing real benchmark runner"),
        (true, false) => Some("Benchmark missing `black_box`: LLVM may optimize measured work away"),
        (true, true) => None,
    }
}

pub fn check_existing_benchmarks_not_stubs(idx: &RepoIndex) -> Vec<Violation> {
    let mut grouped: BTreeMap<String, String> = BTreeMap::new();
    for rel in idx.rs_files_under("packages") {
        let Some(pos) = rel.find("/benches/") else {
            continue;
        };
        let after = &rel[pos + "/benches/".len()..];
        let key = if let Some((dir, _)) = after.split_once('/') {
            format!("{}/benches/{dir}/main.rs", &rel[..pos])
        } else {
            rel.clone()
        };
        if let Some(src) = idx.read_text(&rel) {
            let entry = grouped.entry(key).or_default();
            entry.push_str(&src);
            entry.push('\n');
        }
    }
    assert!(!grouped.is_empty(), "expected benchmark targets under packages/**/benches/");
    grouped
        .iter()
        .filter_map(|(target, combined)| {
            check_bench_not_stub(combined).map(|reason| Violation::new(target, 1, target, reason))
        })
        .collect()
}

pub struct BenchAllocReport {
    pub active_nodestore: Vec<Violation>,
    pub core_backend_benches: Vec<Violation>,
    pub all_crate_benches: Vec<Violation>,
    pub alloc_guards: Vec<Violation>,
}

pub fn check_crate_benchmarks_and_alloc_guards(idx: &RepoIndex) -> BenchAllocReport {
    let mut report = BenchAllocReport {
        active_nodestore: Vec::new(),
        core_backend_benches: Vec::new(),
        all_crate_benches: Vec::new(),
        alloc_guards: Vec::new(),
    };

    for pkg in idx.graph.workspace().iter() {
        let Some(dir) = pkg.source().workspace_path().map(|p| p.as_str().replace('\\', "/")) else {
            continue;
        };
        if pkg.is_proc_macro() || dir == "packages/tools/bench" || dir == "packages/tools/arch-invariants" {
            continue;
        }
        let is_core_or_backend =
            dir.starts_with("packages/core/") || dir.starts_with("packages/backends/");
        let in_bench_scope = is_core_or_backend
            || dir.starts_with("packages/framework/")
            || dir.starts_with("packages/modules/")
            || dir == "packages/tools/cli";
        let in_alloc_scope = is_core_or_backend || dir.starts_with("packages/framework/");
        if !in_bench_scope {
            continue;
        }
        let manifest = format!("{dir}/Cargo.toml");

        let has_bench_target = pkg
            .build_targets()
            .any(|t| matches!(t.id(), BuildTargetId::Benchmark(_)));
        let has_bench_file = idx.rs_files_under(&format!("{dir}/benches")).iter().any(|rel| {
            idx.read_text(rel)
                .is_some_and(|src| check_bench_not_stub(&src).is_none())
        });
        if !(has_bench_target || has_bench_file) {
            let v = Violation::new(
                &manifest,
                1,
                &dir,
                "Crate must ship at least one real colocated benchmark (`[[bench]]` or non-stub `benches/*.rs`)",
            );
            if dir == "packages/core/store" {
                report.active_nodestore.push(v.clone());
            }
            if is_core_or_backend {
                report.core_backend_benches.push(v.clone());
            }
            report.all_crate_benches.push(v);
        }

        if in_alloc_scope {
            let has_alloc = ["tests", "benches"].iter().any(|sub| {
                idx.rs_files_under(&format!("{dir}/{sub}")).iter().any(|rel| {
                    idx.read_text(rel).is_some_and(|src| {
                        file_contains_ident(&src, "GlobalAlloc")
                            || file_contains_ident(&src, "CountingAlloc")
                    })
                })
            });
            if !has_alloc {
                let v = Violation::new(
                    &manifest,
                    1,
                    &dir,
                    "Crate under packages/{core,backends,framework}/* must ship a `GlobalAlloc`/`CountingAlloc` guard",
                );
                if dir == "packages/core/store" || dir == "packages/backends/headless" {
                    report.active_nodestore.push(v.clone());
                }
                report.alloc_guards.push(v);
            }
        }
    }
    report
}
