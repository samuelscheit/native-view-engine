//! Workspace architectural invariant suite (see `tests/unit_tests.rs` for synthetic positive/negative unit tests).

use std::collections::BTreeSet;

use nve_arch_invariants::{
    assert_no_violations, assert_staged_violations, ast_enums, ast_layout, bench, dag,
    find_files_named, host_runtimes, hot_path, hygiene, is_self_invariant_file, is_test_or_bench_rs, purity,
    rel_path, repo_index, rpc_dag, spec_registry, Stage, Violation,
};

// ============================================================================
// Workspace DAG invariants (`guppy` + `target-spec`)
// ============================================================================

/// Enforces [INV:module-and-host-boundaries].
#[test]
fn no_sdk_library_crates_in_hosts_and_zero_packages_to_hosts_deps() {
    let idx = repo_index();
    let approved: BTreeSet<&str> = dag::APPROVED_HOST_CARGO_TOMLS.iter().copied().collect();
    let mut violations: Vec<Violation> = find_files_named(&idx.root.join("hosts"), "Cargo.toml")
        .into_iter()
        .map(|p| rel_path(&idx.root, &p))
        .filter(|rel| !approved.contains(rel.as_str()))
        .map(|rel| {
            Violation::new(
                &rel,
                1,
                &rel,
                "hosts/ must not contain SDK library crates outside approved host shell entry points",
            )
        })
        .collect();
    violations.extend(dag::check_packages_to_hosts_deps(&idx.graph));
    assert_no_violations("No SDK library crates in hosts/ & no packages/** -> hosts/**", &violations);
}

/// Enforces [INV:core-zero-dioxus].
#[test]
fn core_crates_zero_dioxus_and_platform_dependencies() {
    let idx = repo_index();
    let mut violations = dag::check_core_zero_dioxus(&idx.graph);
    violations.extend(dag::check_core_zero_platform_crates(&idx.graph));
    assert_no_violations("Framework- & platform-agnostic packages/core/*", &violations);
}

/// Enforces [INV:zero-runtime-serde-json] and [INV:module-and-host-boundaries].
#[test]
fn core_backends_modules_zero_serde_json_and_zero_module_deps() {
    let idx = repo_index();
    let mut violations = dag::check_zero_serde_json(&idx.graph);
    violations.extend(dag::check_core_backend_module_deps(&idx.graph));
    violations.extend(dag::scan_web_backend_new_auto_reg(idx));
    assert_no_violations(
        "Zero runtime serde_json, zero core/backend -> modules deps, zero WebBackend::new auto-reg",
        &violations,
    );
}

/// Enforces [INV:module-and-host-boundaries].
#[test]
fn module_root_spec_purity_and_wasm32_dev_dep_gating() {
    let idx = repo_index();
    let violations = dag::check_module_root_spec_and_wasm32_dev_deps(&idx.graph);
    assert_no_violations(
        "Module root spec purity, no web+skia feature mix, and real target-spec wasm32 dev-dep gating",
        &violations,
    );
}

/// Enforces [INV:zero-fnv1a].
#[test]
fn zero_fnv_crate_dependency_workspace_wide() {
    let idx = repo_index();
    let violations = rpc_dag::check_zero_fnv_crate_dep(&idx.graph);
    assert_no_violations("Zero `fnv` crate dependency across workspace", &violations);
}

/// Enforces [INV:zero-runtime-serde-json].
#[test]
fn framework_rpc_and_server_layer_purity() {
    let idx = repo_index();
    let violations = rpc_dag::check_framework_rpc_server_purity(&idx.graph);
    assert_no_violations(
        "Framework RPC/server layer purity (zero serde_json; zero tokio/hyper/axum in sans-I/O crates; zero core -> framework deps)",
        &violations,
    );
}

// ============================================================================
// Cross-file `syn` AST invariants
// ============================================================================

#[test]
fn cross_file_enums_converters_and_match_tables() {
    let idx = repo_index();
    let mut core_owned = Vec::new();
    let mut backend_owned = Vec::new();
    let mut violations = Vec::new();

    for rel in idx.rs_files_under("packages") {
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let Some(parsed) = idx.parse_rust(&rel) else {
            continue;
        };
        ast_enums::scan_match_tables(&rel, &parsed.raw, &parsed.ast, &mut violations);
        if rel.starts_with("packages/core/") {
            core_owned.push((rel, parsed));
        } else if rel.starts_with("packages/backends/") {
            backend_owned.push((rel, parsed));
        }
    }

    let core_refs: Vec<_> = core_owned.iter().map(|(r, p)| (r.clone(), p.raw.as_str(), &p.ast)).collect();
    let backend_refs: Vec<_> = backend_owned.iter().map(|(r, p)| (r.clone(), p.raw.as_str(), &p.ast)).collect();
    assert!(!core_refs.is_empty() && !backend_refs.is_empty());
    ast_enums::scan_duplicate_enums_and_converters(&core_refs, &backend_refs, &mut violations);

    assert_staged_violations(
        Stage::Post1B1D,
        "Cross-file duplicate enums, redundant NVE->NVE converters, mirrored <-> match pairs, discriminant matches, core str tables",
        &violations,
    );
}

/// Enforces [INV:abi-repr-c-locks].
#[test]
fn cross_file_repr_c_abi_layout_asserts_and_cache_line_locks() {
    let idx = repo_index();
    let collect = |prefix: &str| {
        idx.rs_files_under(prefix)
            .into_iter()
            .filter(|r| !is_test_or_bench_rs(r))
            .filter_map(|r| idx.parse_rust(&r).map(|p| (r, p)))
            .collect::<Vec<_>>()
    };
    let abi_owned = collect("packages/core/abi/src");
    let store_owned = collect("packages/core/store/src");
    assert!(!abi_owned.is_empty() && !store_owned.is_empty());

    let abi_refs: Vec<_> = abi_owned.iter().map(|(r, p)| (r.clone(), &p.ast)).collect();
    let store_refs: Vec<_> = store_owned.iter().map(|(r, p)| (r.clone(), &p.ast)).collect();
    let violations = ast_layout::scan_repr_c_abi_locks(&abi_refs, &store_refs);

    assert_staged_violations(
        Stage::Post1B1D,
        "Cross-file #[repr(C)] ABI layout asserts & cache-line locks (NodeRow=32, NveRenderNode=64, NveEvent=64)",
        &violations,
    );
}

/// Enforces [INV:typed-domain-handles].
#[test]
fn bare_primitive_domain_fields_in_core_and_module_roots() {
    let idx = repo_index();
    let mut v_store = Vec::new();
    let mut v_1b1d = Vec::new();

    for dir in ["abi", "store", "engine", "tables", "components", "codec"] {
        for rel in idx.rs_files_under(&format!("packages/core/{dir}/src")) {
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            if let Some(parsed) = idx.parse_rust(&rel) {
                let target = if dir == "store" && rel != "packages/core/store/src/events.rs" {
                    &mut v_store
                } else {
                    &mut v_1b1d
                };
                ast_layout::scan_bare_primitive_domain_fields(&rel, &parsed.raw, &parsed.ast, target);
            }
        }
    }
    for pkg in idx.graph.workspace().iter() {
        let Some(dir) = pkg.source().workspace_path().map(|p| p.as_str().replace('\\', "/")) else {
            continue;
        };
        if dir.starts_with("packages/modules/") && dir.matches('/').count() == 2 {
            for rel in idx.rs_files_under(&format!("{dir}/src")) {
                if !is_test_or_bench_rs(&rel)
                    && let Some(parsed) = idx.parse_rust(&rel)
                {
                    ast_layout::scan_bare_primitive_domain_fields(
                        &rel,
                        &parsed.raw,
                        &parsed.ast,
                        &mut v_1b1d,
                    );
                }
            }
        }
    }

    assert_no_violations("Zero bare primitive domain fields in packages/core/store", &v_store);
    assert_staged_violations(
        Stage::Post1B1D,
        "Zero bare primitive domain fields in core/{abi,engine,tables,components,codec} & modules/<cap>",
        &v_1b1d,
    );
}

// ============================================================================
// Repo hygiene & benchmark / CountingAlloc coverage
// ============================================================================

/// Enforces [INV:backlog-open-only], [INV:generated-files-hygiene], and [INV:max-650-lines].
#[test]
fn repo_hygiene_backlog_artifacts_buzzwords_personal_paths_readmes_and_650_line_ratchet() {
    let idx = repo_index();
    assert_no_violations("Open-only docs/BACKLOG.md", &hygiene::check_backlog_open_only(idx));
    assert_no_violations(
        "Generated-file naming & zero tracked generated/build/trace artifacts",
        &hygiene::check_generated_files_and_artifacts(idx),
    );
    assert_no_violations("Zero marketing buzzword `Zero-JSON`", &hygiene::check_zero_json_buzzword(idx));
    assert_no_violations(
        "No hardcoded personal endpoints, hosts, or home paths",
        &hygiene::check_personal_endpoints_or_paths(idx),
    );
    let (v_readme, v_doctest) = hygiene::check_crate_readmes_and_doctest_hooks(idx);
    assert_no_violations("Every packages/** crate has non-empty README.md (>= 5 lines)", &v_readme);
    assert_no_violations(
        "Every packages/** library crate wires README.md into rustdoc doctests",
        &v_doctest,
    );
    assert_no_violations(
        "Production Rust file size <= 650 lines across packages/ and hosts/ (monotonic ratchet)",
        &hygiene::check_production_file_sizes(idx),
    );

    let mut v_hashes = Vec::new();
    for rel in &idx.tracked {
        if is_self_invariant_file(rel) {
            continue;
        }
        if rel.ends_with(".rs") {
            if let Some(parsed) = idx.parse_rust_if_contains(rel, "SCHEMA_HASH") {
                ast_layout::scan_hardcoded_schema_hashes(rel, &parsed.raw, &parsed.ast, &mut v_hashes);
            }
        } else if rel.starts_with("docs/content/")
            && rel.ends_with(".mdx")
            && let Some(content) = idx.read_text(rel)
        {
            for (i, line) in content.lines().enumerate() {
                if hygiene::has_16_hex_literal(line) {
                    v_hashes.push(Violation::new(
                        rel,
                        i + 1,
                        line,
                        "Raw 16-hex schema hash in MDX docs goes stale; reference symbol name instead",
                    ));
                }
            }
        }
    }
    assert_no_violations("Zero hardcoded *_SCHEMA_HASH hex literals", &v_hashes);
}

/// Enforces [INV:zero-fnv1a].
#[test]
fn active_hot_path_utf8_alloc_guards_and_zero_fnv1a_repo_wide() {
    let idx = repo_index();
    assert_no_violations(
        "Zero hot-path allocations or UTF-8 re-validation in packages/core/store & EXISTING_HOT_PATH_FILES",
        &hot_path::check_hot_path_perf_and_utf8(idx),
    );
    assert_no_violations(
        "Zero per-node Any::downcast_ref across backends, core/{abi,components,tables}, engine/layout, and hot store",
        &hot_path::check_zero_per_node_downcast_ref(idx),
    );
    assert_no_violations(
        "Zero FNV-1a identifiers or offset/prime constants repo-wide (code, docs, scripts)",
        &hot_path::check_zero_fnv1a_repo_wide(idx),
    );
}

/// Enforces [INV:no-legacy-acronyms].
#[test]
fn staged_table_hygiene_platform_purity_acronyms_and_codegen_53bit() {
    let idx = repo_index();
    let mut v_table = Vec::new();
    let mut v_purity = Vec::new();

    for rel in idx.rs_files_under("packages") {
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let Some(parsed) = idx.parse_rust(&rel) else {
            continue;
        };
        purity::scan_table_and_codec_hygiene(&rel, &parsed.raw, &parsed.ast, &mut v_table);
        if rel.starts_with("packages/core/") {
            purity::scan_platform_purity(&rel, &parsed.raw, &parsed.ast, &mut v_purity);
        }
    }

    assert_staged_violations(
        Stage::Post1B1D,
        "NveTable field hygiene (#[bits], sentinels, split bools), Unset variants, SET_* masks, Tier-1 encode/decode_bytes, dioxus numeric->String",
        &v_table,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "Platform purity: zero CSS/HTML formatters in packages/core/* and zero motion/MotionClock in core",
        &v_purity,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "Zero banned legacy 4-letter format acronyms outside DOC_EXCLUDES",
        &purity::check_banned_4_letter_acronyms(idx),
    );
    assert_staged_violations(
        Stage::Post1B1E,
        "Codegen Dart/TypeScript 53-bit integer safety & legacy wire/reloc/hybrid_rust/scene_abi path bans",
        &purity::check_codegen_hygiene_and_dart_53bit(idx),
    );
}

/// Enforces [INV:bench-and-alloc-guards].
#[test]
fn benchmarks_and_counting_alloc_coverage() {
    let idx = repo_index();
    assert_no_violations(
        "All existing benches/ targets are non-stub benchmarks",
        &bench::check_existing_benchmarks_not_stubs(idx),
    );
    let report = bench::check_crate_benchmarks_and_alloc_guards(idx);
    assert_no_violations(
        "nve-store ships colocated bench & nve-store + nve-backend-headless ship CountingAlloc",
        &report.active_nodestore,
    );
    assert_no_violations(
        "Core & backend crates ship colocated benchmarks",
        &report.core_backend_benches,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "All non-proc-macro packages/* crates ship colocated benchmarks",
        &report.all_crate_benches,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "All packages/{core,backends,framework}/* crates ship GlobalAlloc/CountingAlloc guards",
        &report.alloc_guards,
    );
}

#[test]
fn spec_invariant_ids_have_mechanical_guards() {
    let idx = repo_index();
    let (spec_count, violations) = spec_registry::check_repo_spec_invariants(idx);
    assert!(
        spec_count > 0,
        "expected at least one `[INV:<id>]` tag in SPEC_DOCS so the registry guard cannot pass vacuously"
    );
    assert_no_violations(
        "Every promised `[INV:<id>]` in AGENTS.md / docs/BACKLOG.md / docs/architecture.md has a backing mechanical guard",
        &violations,
    );
}

/// The static host runtimes (`packages/core/host-*`) mirror status codes, `FieldKind`, value tags,
/// limits, `Length` tags, and golden XXH64 digests by hand: every copy must match the Rust source.
#[test]
fn host_runtime_mirrors_match_rust_sources() {
    let idx = repo_index();
    assert_no_violations(
        "Host runtime mirrors match nve_abi / nve_store / nve_hash",
        &host_runtimes::check_host_runtime_mirrors(idx),
    );
}
