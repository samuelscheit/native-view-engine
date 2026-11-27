//! Per-crate benchmark (`[[bench]]`) and heap-allocation (`GlobalAlloc`/`CountingAlloc`) coverage.

use std::collections::BTreeMap;

use guppy::graph::BuildTargetId;
use syn::visit::{self, Visit};

use crate::ast_layout::file_contains_ident;
use crate::{RepoIndex, Violation};

fn ast_file_calls_black_box(ast: &syn::File) -> bool {
    struct BlackBoxCallScan(bool);
    impl<'ast> Visit<'ast> for BlackBoxCallScan {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(ep) = &*node.func
                && ep.path.segments.last().is_some_and(|s| s.ident == "black_box")
            {
                self.0 = true;
            }
            visit::visit_expr_call(self, node);
        }
    }
    let mut scan = BlackBoxCallScan(false);
    scan.visit_file(ast);
    scan.0
}

fn file_has_runner(src: &str) -> bool {
    file_contains_ident(src, "criterion_main")
        || file_contains_ident(src, "criterion_group")
        || file_contains_ident(src, "Criterion")
        || file_contains_ident(src, "divan")
        || (file_contains_ident(src, "Instant") && file_contains_ident(src, "now"))
}

pub fn check_bench_group_not_stub(files: &[(&str, &str)]) -> Option<String> {
    let mut parsed = Vec::with_capacity(files.len());
    for &(path, src) in files {
        match syn::parse_file(src) {
            Ok(ast) => parsed.push((src, ast)),
            Err(err) => return Some(format!("Failed to parse benchmark file `{path}`: {err}")),
        }
    }
    let has_runner = parsed.iter().any(|(src, _)| file_has_runner(src));
    let has_black_box = parsed.iter().any(|(_, ast)| ast_file_calls_black_box(ast));
    match (has_runner, has_black_box) {
        (false, _) => Some("Benchmark stub detected: missing real benchmark runner".to_string()),
        (true, false) => Some(
            "Benchmark missing `black_box(...)` call: LLVM may optimize measured work away"
                .to_string(),
        ),
        (true, true) => None,
    }
}

pub fn check_bench_not_stub(src: &str) -> Option<String> {
    check_bench_group_not_stub(&[("<inline>", src)])
}

pub fn check_existing_benchmarks_not_stubs(idx: &RepoIndex) -> Vec<Violation> {
    let mut grouped: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
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
            grouped.entry(key).or_default().push((rel, src));
        }
    }
    assert!(!grouped.is_empty(), "expected benchmark targets under packages/**/benches/");
    grouped
        .iter()
        .filter_map(|(target, files)| {
            let refs: Vec<(&str, &str)> = files
                .iter()
                .map(|(p, s)| (p.as_str(), s.as_str()))
                .collect();
            check_bench_group_not_stub(&refs)
                .map(|reason| Violation::new(target, 1, target, reason))
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
