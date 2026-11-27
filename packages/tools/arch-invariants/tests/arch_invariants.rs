//! Workspace architectural invariant suite + positive/negative unit tests.

use std::collections::BTreeSet;

use guppy::graph::PackageGraph;
use nve_arch_invariants::{
    assert_no_violations, assert_staged_violations, ast_enums, ast_layout, bench, dag,
    find_files_named, hygiene, is_cfg_test_attr, is_self_invariant_file, is_test_or_bench_rs,
    rel_path, repo_index, Stage, Violation,
};

// ============================================================================
// Workspace DAG invariants (`guppy` + `target-spec`)
// ============================================================================

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

#[test]
fn core_crates_zero_dioxus_and_platform_dependencies() {
    let idx = repo_index();
    let mut violations = dag::check_core_zero_dioxus(&idx.graph);
    violations.extend(dag::check_core_zero_platform_crates(&idx.graph));
    assert_no_violations("Framework- & platform-agnostic packages/core/*", &violations);
}

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

#[test]
fn module_root_spec_purity_and_wasm32_dev_dep_gating() {
    let idx = repo_index();
    let violations = dag::check_module_root_spec_and_wasm32_dev_deps(&idx.graph);
    assert_no_violations(
        "Module root spec purity, no web+skia feature mix, and real target-spec wasm32 dev-dep gating",
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
    assert_staged_violations(
        Stage::Post1B1D,
        "Every workspace lib crate wires README.md into rustdoc doctests",
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
    assert_staged_violations(
        Stage::Post1B1D,
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

// ============================================================================
// Positive & negative unit tests (incl. critical `#[cfg(test)]` soundness)
// ============================================================================

#[test]
fn unit_cfg_test_attr_soundness_rejects_any_not_and_test_utils() {
    let parse_attrs = |code: &str| {
        let file = syn::parse_file(code).expect("valid rust snippet");
        match file.items.into_iter().next().expect("one item") {
            syn::Item::Fn(f) => f.attrs,
            _ => panic!("expected fn item in test snippet"),
        }
    };
    // MUST skip:
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(test)] fn f() {}")));
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(all(test, feature = \"extra\"))] fn f() {}")));
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(all(unix, all(test, not(windows))))] fn f() {}")));
    // MUST NOT skip (critical soundness fix!):
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(any(test, feature = \"extra\"))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(not(test))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(feature = \"test-utils\")] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(all(not(test), unix))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(all(any(test, unix), feature = \"foo\"))] fn f() {}")));
}

#[test]
fn unit_guppy_dag_and_wasm32_target_spec_evaluation() {
    let lib_target = |name: &str, dir: &str| {
        format!(
            r#"[{{"kind":["lib"],"crate_types":["lib"],"name":"{name}","src_path":"{dir}/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true}}]"#
        )
    };
    let synthetic = format!(
        r#"{{
        "version": 1,
        "workspace_root": "/repo",
        "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/modules/cam#0.1.0",
            "path+file:///repo/hosts/skia-shell#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/engine#0.1.0",
                "name": "nve-engine",
                "version": "0.1.0",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml",
                "targets": {t_engine},
                "features": {{}},
                "dependencies": [
                    {{ "name": "dioxus-core", "req": "^0.7", "kind": "build", "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "nve-cam", "req": "*", "kind": null, "optional": true, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/modules/cam" }},
                    {{ "name": "nve-skia-shell", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/hosts/skia-shell" }},
                    {{ "name": "criterion", "req": "^0.8", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": "cfg(any(target_arch = \"wasm32\", unix))" }},
                    {{ "name": "skia-safe", "req": "^0.1", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": "cfg(not(target_arch = \"wasm32\"))" }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/modules/cam#0.1.0",
                "name": "nve-cam",
                "version": "0.1.0",
                "manifest_path": "/repo/packages/modules/cam/Cargo.toml",
                "targets": {t_cam},
                "features": {{ "web": [], "skia": [] }},
                "dependencies": [
                    {{ "name": "web-sys", "req": "^0.3", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{
                "id": "path+file:///repo/hosts/skia-shell#0.1.0",
                "name": "nve-skia-shell",
                "version": "0.1.0",
                "manifest_path": "/repo/hosts/skia-shell/Cargo.toml",
                "targets": {t_skia},
                "features": {{}},
                "dependencies": []
            }},
            {{ "id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "name": "dioxus-core", "version": "0.7.0", "manifest_path": "/cargo/dioxus-core/Cargo.toml", "targets": {t_dioxus}, "features": {{}}, "dependencies": [] }},
            {{ "id": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "name": "web-sys", "version": "0.3.0", "manifest_path": "/cargo/web-sys/Cargo.toml", "targets": {t_websys}, "features": {{}}, "dependencies": [] }},
            {{ "id": "registry+https://github.com/rust-lang/crates.io-index#criterion@0.8.0", "name": "criterion", "version": "0.8.0", "manifest_path": "/cargo/criterion/Cargo.toml", "targets": {t_crit}, "features": {{}}, "dependencies": [] }},
            {{ "id": "registry+https://github.com/rust-lang/crates.io-index#skia-safe@0.1.0", "name": "skia-safe", "version": "0.1.0", "manifest_path": "/cargo/skia-safe/Cargo.toml", "targets": {t_skiasafe}, "features": {{}}, "dependencies": [] }}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/engine#0.1.0",
                    "dependencies": [
                        "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0",
                        "path+file:///repo/packages/modules/cam#0.1.0",
                        "path+file:///repo/hosts/skia-shell#0.1.0",
                        "registry+https://github.com/rust-lang/crates.io-index#criterion@0.8.0",
                        "registry+https://github.com/rust-lang/crates.io-index#skia-safe@0.1.0"
                    ],
                    "deps": [
                        {{ "name": "dioxus_core", "pkg": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "dep_kinds": [{{ "kind": "build", "target": null }}] }},
                        {{ "name": "nve_cam", "pkg": "path+file:///repo/packages/modules/cam#0.1.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "nve_skia_shell", "pkg": "path+file:///repo/hosts/skia-shell#0.1.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "criterion", "pkg": "registry+https://github.com/rust-lang/crates.io-index#criterion@0.8.0", "dep_kinds": [{{ "kind": "dev", "target": "cfg(any(target_arch = \"wasm32\", unix))" }}] }},
                        {{ "name": "skia_safe", "pkg": "registry+https://github.com/rust-lang/crates.io-index#skia-safe@0.1.0", "dep_kinds": [{{ "kind": "dev", "target": "cfg(not(target_arch = \"wasm32\"))" }}] }}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/modules/cam#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0"],
                    "deps": [{{ "name": "web_sys", "pkg": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "dep_kinds": [{{ "kind": null, "target": null }}] }}]
                }},
                {{ "id": "path+file:///repo/hosts/skia-shell#0.1.0", "dependencies": [], "deps": [] }},
                {{ "id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "dependencies": [], "deps": [] }},
                {{ "id": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "dependencies": [], "deps": [] }},
                {{ "id": "registry+https://github.com/rust-lang/crates.io-index#criterion@0.8.0", "dependencies": [], "deps": [] }},
                {{ "id": "registry+https://github.com/rust-lang/crates.io-index#skia-safe@0.1.0", "dependencies": [], "deps": [] }}
            ]
        }}
    }}"#,
        t_engine = lib_target("nve_engine", "/repo/packages/core/engine"),
        t_cam = lib_target("nve_cam", "/repo/packages/modules/cam"),
        t_skia = lib_target("nve_skia_shell", "/repo/hosts/skia-shell"),
        t_dioxus = lib_target("dioxus_core", "/cargo/dioxus-core"),
        t_websys = lib_target("web_sys", "/cargo/web-sys"),
        t_crit = lib_target("criterion", "/cargo/criterion"),
        t_skiasafe = lib_target("skia_safe", "/cargo/skia-safe"),
    );
    let graph = PackageGraph::from_json(&synthetic).expect("valid synthetic guppy graph");
    assert!(!dag::check_core_zero_dioxus(&graph).is_empty());
    assert!(!dag::check_core_zero_platform_crates(&graph).is_empty());
    assert!(!dag::check_core_backend_module_deps(&graph).is_empty());
    assert!(dag::check_packages_to_hosts_deps(&graph)
        .iter()
        .any(|v| v.snippet.contains("nve-engine -> nve-skia-shell")));

    let mod_v = dag::check_module_root_spec_and_wasm32_dev_deps(&graph);
    // `criterion` with `cfg(any(target_arch = "wasm32", unix))` MUST fail; `skia-safe` with `not(wasm32)` MUST pass!
    assert!(mod_v.iter().any(|v| v.snippet.contains("criterion")));
    assert!(!mod_v.iter().any(|v| v.snippet.contains("skia-safe")));
    assert!(mod_v.iter().any(|v| v.reason.contains("mix `web` and `skia`")));
}

#[test]
fn unit_syn_ast_checks_positive_and_negative() {
    let core_a_src = "pub enum TextAlign { Left, Center, Right }\npub enum Display { Flex, None }\n#[cfg(any(test, feature = \"extra\"))] pub struct ExposedInProd { pub node_id: u32 }\n#[cfg(test)] pub struct OnlyTest { pub node_id: u32 }";
    let core_b_src = "pub enum TextAlign { Start, End }\npub enum PatchAlign { Start, End }\nimpl From<PatchAlign> for TextAlign { fn from(_: PatchAlign) -> Self { Self::Start } }";
    let backend_src = r#"
        pub fn decode_align(raw: u8) -> TextAlign {
            match raw { 0 => TextAlign::Left, 1 => TextAlign::Center, 2 => TextAlign::Right, _ => TextAlign::Left }
        }
        pub fn to_skia_align(a: TextAlign) -> skia_safe::textlayout::TextAlign { unreachable!() }
        impl TextAlign {
            pub fn fwd(self) -> &'static str { match self { Self::Left => "l", Self::Center => "c", Self::Right => "r" } }
            pub fn rev(s: &str) -> Option<Self> { match s { "l" => Some(Self::Left), "c" => Some(Self::Center), "r" => Some(Self::Right), _ => None } }
        }
    "#;
    let ast_a = syn::parse_file(core_a_src).unwrap();
    let ast_b = syn::parse_file(core_b_src).unwrap();
    let ast_bk = syn::parse_file(backend_src).unwrap();

    let mut v_fields = Vec::new();
    ast_layout::scan_bare_primitive_domain_fields("a.rs", core_a_src, &ast_a, &mut v_fields);
    assert_eq!(v_fields.len(), 1, "cfg(any(test, ...)) must be scanned, cfg(test) skipped: {v_fields:?}");

    let mut v_enum = Vec::new();
    ast_enums::scan_duplicate_enums_and_converters(
        &[("core/a.rs".into(), core_a_src, &ast_a), ("core/b.rs".into(), core_b_src, &ast_b)],
        &[("backends/skia.rs".into(), backend_src, &ast_bk)],
        &mut v_enum,
    );
    assert_eq!(v_enum.len(), 4, "2 dup TextAlign + 1 impl From + 1 decode_align (not to_skia_align): {v_enum:?}");
    assert!(!v_enum.iter().any(|v| v.reason.contains("to_skia_align")));

    let mut v_match = Vec::new();
    ast_enums::scan_match_tables("backends/skia.rs", backend_src, &ast_bk, &mut v_match);
    assert_eq!(v_match.len(), 2, "expected 1 mirrored fwd/rev + 1 compact int decode_align: {v_match:?}");

    // Cross-file ABI layout & cache-line locks
    let abi_a = syn::parse_file("#[derive(NveAbi)] #[nve_abi(size = 64)] #[repr(C)] pub struct NveRenderNode { pub x: u32 }\n#[repr(C)] pub struct NveApi { pub f: usize }").unwrap();
    let abi_b = syn::parse_file("const _: () = { assert!(size_of::<NveApi>() == 8); assert!(align_of::<NveApi>() == 8); assert!(offset_of!(NveApi, f) == 0); };").unwrap();
    let store_a = syn::parse_file("#[repr(C)] pub struct NodeRow { pub x: u32 }\nconst _: () = { assert!(size_of::<NodeRow>() == 32); };").unwrap();
    let v_abi = ast_layout::scan_repr_c_abi_locks(
        &[("abi/a.rs".into(), &abi_a), ("abi/b.rs".into(), &abi_b)],
        &[("store/row.rs".into(), &store_a)],
    );
    assert_eq!(v_abi.len(), 1, "only missing NveEvent should fail: {v_abi:?}");
    assert!(v_abi[0].reason.contains("`NveEvent`"));

    // Personal path/endpoint & anti-stub bench unit checks
    assert!(hygiene::detect_personal_endpoint_or_path("/Users/alice/dev", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("https://box.tail.ts.net", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", true).is_none());
    assert!(hygiene::detect_personal_endpoint_or_path("127.0.0.1", false).is_none());
    assert!(bench::check_bench_not_stub("fn main() {}").is_some());
    assert!(bench::check_bench_not_stub("use std::hint::black_box; fn b(c: &mut Criterion) { black_box(1); }").is_none());
}
