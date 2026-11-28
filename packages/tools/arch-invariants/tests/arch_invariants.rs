//! Workspace architectural invariant suite (see `tests/unit_tests.rs` for synthetic positive/negative unit tests).

use std::collections::BTreeSet;

use nve_arch_invariants::{
    assert_no_violations, assert_staged_ratchet, ast_enums, ast_layout, bench, dag,
    find_files_named, host_runtimes, hot_path, hygiene, is_self_invariant_file, is_test_or_bench_rs, native_build, purity,
    rel_path, repo_index, rpc_dag, scene_abi_guard, spec_registry, theme_hygiene, inline_style_keys, Stage, Violation,
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

/// Enforces [INV:core-zero-native-cxx].
#[test]
fn core_crates_compile_zero_native_cxx() {
    let idx = repo_index();
    assert_no_violations(
        "packages/core/* Rust crates compile zero native C/C++",
        &native_build::check_core_zero_native_cxx(idx),
    );
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

/// Enforces [INV:no-duplicate-enum-match-arms].
#[test]
fn no_duplicate_enum_match_arms_workspace_wide() {
    let idx = repo_index();
    let report = ast_enums::check_duplicate_enum_match_arms(idx);
    assert_no_violations(
        "Zero new duplicated enum match/assignment tables (same-file & cross-file, same-direction & mirrored) and zero stale baseline entries",
        &report.active_violations,
    );
    assert_staged_ratchet(
        Stage::Post1B1D,
        "Pre-existing allowlisted duplicate enum match/assignment tables",
        20,
        &report.allowlisted_violations,
    );
}

/// Enforces [INV:no-manual-repr-enum-converters].
#[test]
fn no_manual_repr_enum_converters() {
    let idx = repo_index();
    let all_owned = ast_enums::collect_workspace_prod_rs(idx);
    let all_refs: Vec<_> = all_owned
        .iter()
        .map(|f| {
            let crate_key = f
                .crate_name
                .clone()
                .unwrap_or_else(|| f.rel.clone());
            (f.rel.clone(), crate_key, f.parsed.raw.as_str(), &f.parsed.ast)
        })
        .collect();
    assert!(!all_refs.is_empty());
    let mut violations = Vec::new();
    ast_enums::scan_manual_repr_enum_converters_with_crates(&all_refs, &mut violations);
    assert_no_violations(
        "Zero hand-written integer-to-enum converters for #[repr(<int>)] unit enums (derive ReprEnum / WordEnum / NveCodec)",
        &violations,
    );
}

#[test]
fn cross_file_enums_converters_and_match_tables() {
    let idx = repo_index();
    let all_owned = ast_enums::collect_workspace_prod_rs(idx);
    let mut violations = Vec::new();

    for f in &all_owned {
        if f.rel.starts_with("packages/") {
            ast_enums::scan_match_tables(&f.rel, &f.parsed.raw, &f.parsed.ast, &mut violations);
        }
    }

    let all_refs: Vec<_> = all_owned
        .iter()
        .map(|f| (f.rel.clone(), f.parsed.raw.as_str(), &f.parsed.ast))
        .collect();
    assert!(!all_refs.is_empty());
    ast_enums::scan_duplicate_enums_and_converters(&all_refs, &mut violations);

    assert_staged_ratchet(
        Stage::Post1B1D,
        "Cross-file duplicate enums, redundant NVE->NVE converters, discriminant matches, core str tables",
        67,
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

    assert_no_violations(
        "Cross-file #[repr(C)] ABI layout asserts & cache-line locks (NodeRow=32, NveNodeRow=32, NveEvent=64)",
        &violations,
    );
}

/// Enforces [INV:typed-domain-handles].
#[test]
fn bare_primitive_domain_fields_in_core_and_module_roots() {
    let idx = repo_index();
    let mut v_store = Vec::new();
    let mut v_1b1d = Vec::new();

    let mut v_handle_hygiene = Vec::new();
    let mut v_handle_params = Vec::new();
    let mut saw_abi_id_file = false;
    for root in ["packages", "hosts", "examples"] {
        for rel in idx.rs_files_under(root) {
            if is_self_invariant_file(&rel) {
                continue;
            }
            if let Some(parsed) = idx.parse_rust(&rel) {
                if rel == "packages/core/abi/src/id.rs" {
                    saw_abi_id_file = true;
                }
                ast_layout::scan_handle_hygiene(&rel, &parsed.raw, &parsed.ast, &mut v_handle_hygiene);
            }
        }
    }
    assert!(
        saw_abi_id_file,
        "handle hygiene scan must visit and parse `packages/core/abi/src/id.rs` so it cannot pass vacuously"
    );
    assert_no_violations(
        "Unified NodeId/SurfaceId privacy, zero raw tuple constructors outside id.rs, and zero StoreNodeId aliases",
        &v_handle_hygiene,
    );

    let mut core_crates_scanned = 0usize;
    let mut core_files_scanned = 0usize;
    for pkg in idx.graph.workspace().iter() {
        let Some(dir) = pkg.source().workspace_path().map(|p| p.as_str().replace('\\', "/")) else {
            continue;
        };
        if dir.starts_with("packages/core/") {
            core_crates_scanned += 1;
            for rel in idx.rs_files_under(&format!("{dir}/src")) {
                if is_test_or_bench_rs(&rel) {
                    continue;
                }
                if let Some(parsed) = idx.parse_rust(&rel) {
                    core_files_scanned += 1;
                    let target = if dir == "packages/core/store"
                        && rel != "packages/core/store/src/events.rs"
                    {
                        &mut v_store
                    } else {
                        &mut v_1b1d
                    };
                    ast_layout::scan_bare_primitive_domain_fields(
                        &rel,
                        &parsed.raw,
                        &parsed.ast,
                        target,
                    );
                    ast_layout::scan_bare_handle_params(
                        &rel,
                        &parsed.raw,
                        &parsed.ast,
                        &mut v_handle_params,
                    );
                }
            }
        }
    }
    assert!(
        core_crates_scanned > 0 && core_files_scanned > 0,
        "bare handle parameter scan must visit and parse packages/core/* source files so it cannot pass vacuously"
    );
    assert_no_violations(
        "Zero bare primitive surface_id/node_id/frame_id/interface_id parameters in core Rust APIs",
        &v_handle_params,
    );
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
    assert_staged_ratchet(
        Stage::Post1B1D,
        "Zero bare primitive domain fields in core/{abi,engine,tables,codec} & modules/<cap>",
        61,
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
        "Production Rust file size <= 650 lines across packages/, hosts/ and examples/ (monotonic ratchet)",
        &hygiene::check_production_file_sizes(idx),
    );
    assert_no_violations("Themed UI sources use Theme tokens only", &theme_hygiene::check_theme_token_hygiene(idx));
    assert_no_violations("Tone::solid is only a fill (WCAG 1.4.11)", &theme_hygiene::check_solid_only_as_fill(idx));
    assert_no_violations("No theme mode scraped from the rendered tree", &theme_hygiene::check_no_theme_tree_scraping(idx));
    assert_no_violations(
        "Zero author-facing RSX `style:` attributes and zero `style: Option<String>` component props (zero runtime CSS parser)",
        &inline_style_keys::check_no_inline_css_style_attrs(idx),
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

/// Enforces [INV:single-scene-abi].
#[test]
fn one_scene_abi_and_no_cxx_fabric_bridge() {
    assert_no_violations(
        "Zero retired legacy scene records / readers (NveRenderNode, NveSceneTable, NveSceneBuffer, NveRuntimeSceneView, RuntimeSceneReader, ...) and zero cxx/nve-fabric-bridge references",
        &scene_abi_guard::check_single_scene_abi(repo_index()),
    );
}

/// Enforces [INV:no-reloc-mmap-snapshot].
#[test]
fn no_retired_reloc_mmap_snapshot_paths_symbols_or_memmap2_deps() {
    let idx = repo_index();
    assert_no_violations(
        "Zero retired legacy persistence paths (mmap_arena / reloc_scene / nve_snapshot / instant_boot / mmap_snapshot / reloc/)",
        &hygiene::check_banned_legacy_persistence_paths(idx),
    );
    assert_no_violations(
        "Zero retired legacy persistence symbols (MmapOffsetArena / RelocSceneNode / RelocSceneRoot / RelocatableStateArena)",
        &hygiene::check_banned_legacy_persistence_symbols(idx),
    );
    assert_no_violations(
        "Zero direct memmap2 dependencies workspace-wide and zero transitive memmap2 in core (non-dioxus) / headless / web",
        &dag::check_no_memmap2_dep(&idx.graph),
    );
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
        "Zero per-node Any::downcast_ref across backends, core/{abi,tables}, engine/layout, and hot store",
        &hot_path::check_zero_per_node_downcast_ref(idx),
    );
    assert_no_violations(
        "Zero FNV-1a identifiers or offset/prime constants repo-wide (code, docs, scripts)",
        &hot_path::check_zero_fnv1a_repo_wide(idx),
    );
}

#[test]
fn table_hygiene_platform_purity_and_codegen_53bit() {
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

    assert_no_violations(
        "NveTable field hygiene (#[bits], sentinels, split bools), Unset variants, SET_* masks, Tier-1 encode/decode_bytes, dioxus numeric->String",
        &v_table,
    );
    assert_no_violations(
        "Platform purity: zero CSS/HTML formatters in packages/core/* and zero motion/MotionClock in core",
        &v_purity,
    );
    assert_no_violations(
        "Codegen Dart/TypeScript 53-bit integer safety & legacy wire/reloc/hybrid_rust/scene_abi path bans",
        &purity::check_codegen_hygiene_and_dart_53bit(idx),
    );
}

/// Enforces [INV:no-legacy-acronyms] (shrink-only monotonic ratchet; zero-tolerance under `NVE_STRICT_INVARIANTS=1b1d`).
#[test]
fn no_banned_legacy_4_letter_acronyms_ratchet() {
    let idx = repo_index();
    assert_staged_ratchet(
        Stage::Post1B1D,
        "Zero banned legacy 4-letter format acronyms outside DOC_EXCLUDES",
        424,
        &purity::check_banned_4_letter_acronyms(idx),
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
    assert_staged_ratchet(
        Stage::Post1B1D,
        "All non-proc-macro packages/* crates ship colocated benchmarks",
        26,
        &report.all_crate_benches,
    );
    assert_staged_ratchet(
        Stage::Post1B1D,
        "All packages/{core,backends,framework}/* crates ship GlobalAlloc/CountingAlloc guards",
        2,
        &report.alloc_guards,
    );
}

#[test]
fn spec_invariant_ids_have_mechanical_guards() {
    let idx = repo_index();
    let report = spec_registry::check_repo_spec_invariants(idx);
    assert!(
        report.spec_count > 0 && report.enforced_count() > 0,
        "expected enforced `[INV:<id>]` guards in SPEC_DOCS (found {} spec tags, {} enforced, {} ratcheted)",
        report.spec_count,
        report.enforced_count(),
        report.ratcheted_count(),
    );
    assert_no_violations(
        "Every promised `[INV:<id>]` in AGENTS.md / docs/BACKLOG.md / docs/architecture.md has a backing mechanical guard",
        &report.violations,
    );
    assert_eq!(
        report.enforced_count() + report.ratcheted_count(),
        report.spec_count,
        "every covered spec invariant must be classified honestly as Enforced or Ratcheted",
    );
    let ratcheted_violations: Vec<Violation> = report
        .covered
        .iter()
        .filter(|(_, strength)| **strength == spec_registry::GuardStrength::Ratcheted)
        .map(|(id, _)| {
            Violation::new(
                "AGENTS.md",
                0,
                format!("[INV:{id}]"),
                format!("Spec invariant `[INV:{id}]` is backed by a staged ratchet rather than zero-tolerance Enforced"),
            )
        })
        .collect();
    assert_staged_ratchet(
        Stage::Post1B1D,
        "Spec invariants backed by a staged ratchet instead of zero-tolerance Enforced",
        4,
        &ratcheted_violations,
    );
}

/// The static host runtimes (`packages/core/host-*`) mirror status codes, `FieldKind`, value tags,
/// limits, and `Length` tags by hand: every copy must match the Rust source. (Golden XXH64 vectors
/// are generated from `nve_hash` by `nve codegen abi`.)
#[test]
fn host_runtime_mirrors_match_rust_sources() {
    let idx = repo_index();
    assert_no_violations(
        "Host runtime mirrors match nve_abi / nve_store",
        &host_runtimes::check_host_runtime_mirrors(idx),
    );
}

/// The `NveBinValue` validators of every managed host runtime check strings and skip bytes in place:
/// no view, slice, copy, or decoded string (allocation-free validation, enforced mechanically).
#[test]
fn host_runtime_validators_stay_in_place() {
    let idx = repo_index();
    assert_no_violations(
        "Host runtime BinReader validators never view, copy, or decode",
        &host_runtimes::check_in_place_validators(idx),
    );
}
