//! Positive & negative unit tests for `nve-arch-invariants` scanners (`guppy` DAG, `syn` AST,
//! `#[cfg(test)]` soundness, per-node `downcast_ref`, Dart/TypeScript 53-bit, and multi-file benches).

use guppy::graph::PackageGraph;
use nve_arch_invariants::{
    ast_enums, ast_layout, bench, codegen_53bit, dag, hot_path, hygiene, is_cfg_test_attr,
    native_build, purity,
};

#[test]
fn unit_native_source_reason_flags_cxx_sources_and_cc_build_scripts_only() {
    let flagged = |rel: &str, src: Option<&str>| native_build::native_source_reason(rel, src).is_some();
    assert!(flagged("packages/core/x/c_api/shim.cpp", None));
    assert!(flagged("packages/core/x/c_api/SHIM.CPP", None));
    assert!(flagged("packages/core/x/src/a.cc", None));
    assert!(flagged("packages/core/x/src/a.mm", None));
    assert!(flagged("packages/core/x/src/trampoline.S", None));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { cc::Build::new(); }")));
    assert!(flagged("packages/core/x/scripts/gen.rs", Some("fn main() { cmake::build(\"x\"); }")));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { cxx_build::bridge(\"src/lib.rs\"); }")));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { bindgen::Builder::default(); }")));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { autocxx_build::Builder::new(\"src/lib.rs\", &[]); }")));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { std::process::Command::new(\"clang\"); }")));
    assert!(flagged("packages/core/x/build.rs", Some("fn main() { std::process::Command::new(\"cmake\"); }")));
    assert!(!flagged("packages/core/x/build.rs", Some("fn main() { println!(\"cargo::rerun-if-changed=x\"); }")));
    assert!(!flagged("packages/core/x/src/lib.rs", None));
    assert!(!flagged("packages/core/x/include/api.h", None));
    assert!(!flagged("packages/core/x/README.md", None));
    assert!(!flagged("packages/core/x/tests/fixtures/driver.cpp", None));
    assert!(flagged("packages/core/x/tests/driver.cpp", None));
    assert!(flagged("packages/core/x/src/tests/fixtures.cpp", None));
}

#[test]
fn unit_native_build_graph_and_core_to_backends_dag_checks() {
    let json = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/bad-core#0.1.0",
            "path+file:///repo/packages/tools/cxx-helper#0.1.0",
            "path+file:///repo/packages/backends/headless#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/bad-core#0.1.0", "name": "nve-bad-core", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/bad-core/Cargo.toml", "targets": {t_core}, "features": {{}},
                "links": "badnative",
                "dependencies": [
                    {{"name": "nve-cxx-helper", "req": "*", "kind": "build", "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/tools/cxx-helper"}},
                    {{"name": "nve-backend-headless", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/backends/headless"}},
                    {{"name": "foo-sys", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/tools/cxx-helper#0.1.0", "name": "nve-cxx-helper", "version": "0.1.0",
                "manifest_path": "/repo/packages/tools/cxx-helper/Cargo.toml", "targets": {t_helper}, "features": {{}},
                "dependencies": [
                    {{"name": "cmake", "req": "^0.1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/backends/headless#0.1.0", "name": "nve-backend-headless", "version": "0.1.0",
                "manifest_path": "/repo/packages/backends/headless/Cargo.toml", "targets": {t_bk}, "features": {{}},
                "dependencies": []
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#cmake@0.1.50", "name": "cmake", "version": "0.1.50", "manifest_path": "/cargo/cmake/Cargo.toml", "targets": {t_cmake}, "features": {{}}, "dependencies": []}},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#foo-sys@1.0.0", "name": "foo-sys", "version": "1.0.0", "manifest_path": "/cargo/foo-sys/Cargo.toml", "targets": {t_foosys}, "features": {{}}, "links": "foo", "dependencies": []}}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/bad-core#0.1.0",
                    "dependencies": [
                        "path+file:///repo/packages/tools/cxx-helper#0.1.0",
                        "path+file:///repo/packages/backends/headless#0.1.0",
                        "registry+https://github.com/rust-lang/crates.io-index#foo-sys@1.0.0"
                    ],
                    "deps": [
                        {{"name": "nve_cxx_helper", "pkg": "path+file:///repo/packages/tools/cxx-helper#0.1.0", "dep_kinds": [{{"kind": "build", "target": null}}]}},
                        {{"name": "nve_backend_headless", "pkg": "path+file:///repo/packages/backends/headless#0.1.0", "dep_kinds": [{{"kind": null, "target": null}}]}},
                        {{"name": "foo_sys", "pkg": "registry+https://github.com/rust-lang/crates.io-index#foo-sys@1.0.0", "dep_kinds": [{{"kind": null, "target": null}}]}}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/tools/cxx-helper#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#cmake@0.1.50"],
                    "deps": [{{"name": "cmake", "pkg": "registry+https://github.com/rust-lang/crates.io-index#cmake@0.1.50", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{"id": "path+file:///repo/packages/backends/headless#0.1.0", "dependencies": [], "deps": []}},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#cmake@0.1.50", "dependencies": [], "deps": []}},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#foo-sys@1.0.0", "dependencies": [], "deps": []}}
            ]
        }}
    }}"#,
        t_core = lib_target("nve_bad_core", "/repo/packages/core/bad-core"),
        t_helper = lib_target("nve_cxx_helper", "/repo/packages/tools/cxx-helper"),
        t_bk = lib_target("nve_backend_headless", "/repo/packages/backends/headless"),
        t_cmake = lib_target("cmake", "/cargo/cmake"),
        t_foosys = lib_target("foo_sys", "/cargo/foo-sys"),
    );
    let graph = PackageGraph::from_json(&json).expect("valid synthetic native_build graph");

    let v_native = native_build::check_core_zero_native_cxx_graph(&graph);
    assert!(
        v_native.iter().any(|v| v.snippet.contains("links = \"badnative\"")),
        "expected `links` violation: {v_native:?}"
    );
    assert!(
        v_native
            .iter()
            .any(|v| v.snippet == "nve-bad-core -> nve-cxx-helper -> cmake"),
        "expected transitive workspace native-build violation: {v_native:?}"
    );
    assert!(
        v_native
            .iter()
            .any(|v| v.snippet == "nve-bad-core -> foo-sys (links=foo)"),
        "expected direct external `*-sys` (`links`) violation: {v_native:?}"
    );

    let v_dag = dag::check_core_backend_module_deps(&graph);
    assert!(
        v_dag
            .iter()
            .any(|v| v.snippet == "nve-bad-core -> nve-backend-headless"
                && v.reason.contains("packages/backends/*")),
        "expected core -> backends violation: {v_dag:?}"
    );
}

fn lib_target(name: &str, dir: &str) -> String {
    format!(
        r#"[{{"kind":["lib"],"crate_types":["lib"],"name":"{name}","src_path":"{dir}/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true}}]"#
    )
}

fn proc_macro_target(name: &str, dir: &str) -> String {
    format!(
        r#"[{{"kind":["proc-macro"],"crate_types":["proc-macro"],"name":"{name}","src_path":"{dir}/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true}}]"#
    )
}

#[test]
fn unit_cfg_test_attr_soundness_rejects_any_not_and_test_utils() {
    let parse_attrs = |code: &str| {
        let file = syn::parse_file(code).expect("valid rust snippet");
        match file.items.into_iter().next().expect("one item") {
            syn::Item::Fn(f) => f.attrs,
            _ => panic!("expected fn item in test snippet"),
        }
    };
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(test)] fn f() {}")));
    assert!(is_cfg_test_attr(&parse_attrs(
        "#[cfg(all(test, feature = \"extra\"))] fn f() {}"
    )));
    assert!(is_cfg_test_attr(&parse_attrs(
        "#[cfg(all(unix, all(test, not(windows))))] fn f() {}"
    )));
    assert!(!is_cfg_test_attr(&parse_attrs(
        "#[cfg(any(test, feature = \"extra\"))] fn f() {}"
    )));
    assert!(!is_cfg_test_attr(&parse_attrs(
        "#[cfg(not(test))] fn f() {}"
    )));
    assert!(!is_cfg_test_attr(&parse_attrs(
        "#[cfg(feature = \"test-utils\")] fn f() {}"
    )));
    assert!(!is_cfg_test_attr(&parse_attrs(
        "#[cfg(all(not(test), unix))] fn f() {}"
    )));
    assert!(!is_cfg_test_attr(&parse_attrs(
        "#[cfg(all(any(test, unix), feature = \"foo\"))] fn f() {}"
    )));
}

#[test]
fn unit_guppy_dag_and_wasm32_target_spec_evaluation() {
    let synthetic = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/modules/cam#0.1.0",
            "path+file:///repo/hosts/skia-shell#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/engine#0.1.0", "name": "nve-engine", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml", "targets": {t_engine}, "features": {{}},
                "dependencies": [
                    {{ "name": "dioxus-core", "req": "^0.7", "kind": "build", "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "nve-cam", "req": "*", "kind": null, "optional": true, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/modules/cam" }},
                    {{ "name": "nve-skia-shell", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/hosts/skia-shell" }},
                    {{ "name": "criterion", "req": "^0.8", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": "cfg(any(target_arch = \"wasm32\", unix))" }},
                    {{ "name": "skia-safe", "req": "^0.1", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": "cfg(not(target_arch = \"wasm32\"))" }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/modules/cam#0.1.0", "name": "nve-cam", "version": "0.1.0",
                "manifest_path": "/repo/packages/modules/cam/Cargo.toml", "targets": {t_cam},
                "features": {{ "web": [], "skia": [] }},
                "dependencies": [
                    {{ "name": "web-sys", "req": "^0.3", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{ "id": "path+file:///repo/hosts/skia-shell#0.1.0", "name": "nve-skia-shell", "version": "0.1.0", "manifest_path": "/repo/hosts/skia-shell/Cargo.toml", "targets": {t_skia}, "features": {{}}, "dependencies": [] }},
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
    assert!(
        dag::check_packages_to_hosts_deps(&graph)
            .iter()
            .any(|v| v.snippet.contains("nve-engine -> nve-skia-shell"))
    );

    let mod_v = dag::check_module_root_spec_and_wasm32_dev_deps(&graph);
    assert!(mod_v.iter().any(|v| v.snippet.contains("criterion")));
    assert!(!mod_v.iter().any(|v| v.snippet.contains("skia-safe")));
    assert!(
        mod_v
            .iter()
            .any(|v| v.reason.contains("mix `web` and `skia`"))
    );
}

#[test]
fn unit_guppy_dag_catches_proc_macros_and_wasm32_workspace_mediated_platform_chains() {
    let json = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/core-macro#0.1.0",
            "path+file:///repo/packages/core/core-app#0.1.0",
            "path+file:///repo/packages/core/core-helper#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/core-macro#0.1.0", "name": "nve-core-macro", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/core-macro/Cargo.toml", "targets": {t_cmacro}, "features": {{}},
                "dependencies": [
                    {{"name": "serde_json", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/core/core-app#0.1.0", "name": "nve-core-app", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/core-app/Cargo.toml", "targets": {t_capp}, "features": {{}},
                "dependencies": [
                    {{"name": "dioxus-rsx-macro", "req": "^0.7", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}},
                    {{"name": "nve-core-helper", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": "cfg(target_arch = \"wasm32\")", "path": "/repo/packages/core/core-helper"}},
                    {{"name": "getrandom", "req": "^0.3", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": "cfg(target_arch = \"wasm32\")"}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/core/core-helper#0.1.0", "name": "nve-core-helper", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/core-helper/Cargo.toml", "targets": {t_chelper}, "features": {{}},
                "dependencies": [
                    {{"name": "web-sys", "req": "^0.3", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": "cfg(target_arch = \"wasm32\")"}}
                ]
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-rsx-macro@0.7.0", "name": "dioxus-rsx-macro", "version": "0.7.0", "manifest_path": "/cargo/dioxus-rsx-macro/Cargo.toml", "targets": {t_dmacro}, "features": {{}}, "dependencies": []}},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "name": "serde_json", "version": "1.0.0", "manifest_path": "/cargo/serde_json/Cargo.toml", "targets": {t_sj}, "features": {{}}, "dependencies": []}},
            {{
                "id": "registry+https://github.com/rust-lang/crates.io-index#getrandom@0.3.0", "name": "getrandom", "version": "0.3.0",
                "manifest_path": "/cargo/getrandom/Cargo.toml", "targets": {t_getrand}, "features": {{}},
                "dependencies": [
                    {{"name": "wasm-bindgen", "req": "^0.2", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": "cfg(target_arch = \"wasm32\")"}}
                ]
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "name": "web-sys", "version": "0.3.0", "manifest_path": "/cargo/web-sys/Cargo.toml", "targets": {t_websys}, "features": {{}}, "dependencies": []}},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.0", "name": "wasm-bindgen", "version": "0.2.0", "manifest_path": "/cargo/wasm-bindgen/Cargo.toml", "targets": {t_wbg}, "features": {{}}, "dependencies": []}}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/core-macro#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0"],
                    "deps": [{{"name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{
                    "id": "path+file:///repo/packages/core/core-app#0.1.0",
                    "dependencies": [
                        "registry+https://github.com/rust-lang/crates.io-index#dioxus-rsx-macro@0.7.0",
                        "path+file:///repo/packages/core/core-helper#0.1.0",
                        "registry+https://github.com/rust-lang/crates.io-index#getrandom@0.3.0"
                    ],
                    "deps": [
                        {{"name": "dioxus_rsx_macro", "pkg": "registry+https://github.com/rust-lang/crates.io-index#dioxus-rsx-macro@0.7.0", "dep_kinds": [{{"kind": null, "target": null}}]}},
                        {{"name": "nve_core_helper", "pkg": "path+file:///repo/packages/core/core-helper#0.1.0", "dep_kinds": [{{"kind": null, "target": "cfg(target_arch = \"wasm32\")"}}]}},
                        {{"name": "getrandom", "pkg": "registry+https://github.com/rust-lang/crates.io-index#getrandom@0.3.0", "dep_kinds": [{{"kind": null, "target": "cfg(target_arch = \"wasm32\")"}}]}}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/core/core-helper#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0"],
                    "deps": [{{"name": "web_sys", "pkg": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "dep_kinds": [{{"kind": null, "target": "cfg(target_arch = \"wasm32\")"}}]}}]
                }},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-rsx-macro@0.7.0", "dependencies": [], "deps": []}},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dependencies": [], "deps": []}},
                {{
                    "id": "registry+https://github.com/rust-lang/crates.io-index#getrandom@0.3.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.0"],
                    "deps": [{{"name": "wasm_bindgen", "pkg": "registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.0", "dep_kinds": [{{"kind": null, "target": "cfg(target_arch = \"wasm32\")"}}]}}]
                }},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "dependencies": [], "deps": []}},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.0", "dependencies": [], "deps": []}}
            ]
        }}
    }}"#,
        t_cmacro = proc_macro_target("nve_core_macro", "/repo/packages/core/core-macro"),
        t_capp = lib_target("nve_core_app", "/repo/packages/core/core-app"),
        t_chelper = lib_target("nve_core_helper", "/repo/packages/core/core-helper"),
        t_dmacro = proc_macro_target("dioxus_rsx_macro", "/cargo/dioxus-rsx-macro"),
        t_sj = lib_target("serde_json", "/cargo/serde_json"),
        t_getrand = lib_target("getrandom", "/cargo/getrandom"),
        t_websys = lib_target("web_sys", "/cargo/web-sys"),
        t_wbg = lib_target("wasm_bindgen", "/cargo/wasm-bindgen"),
    );
    let graph = PackageGraph::from_json(&json).expect("valid proc-macro + wasm32 graph");

    // 1. Proc-macro core crate depending on banned `serde_json` must be caught.
    let v_sj = dag::check_zero_serde_json(&graph);
    assert!(
        v_sj.iter()
            .any(|v| v.snippet.contains("nve-core-macro -> serde_json")),
        "proc-macro core crate dep on serde_json must be caught: {v_sj:?}"
    );

    // 2. Normal core crate depending on proc-macro `dioxus-rsx-macro` must be caught.
    let v_dx = dag::check_core_zero_dioxus(&graph);
    assert!(
        v_dx.iter()
            .any(|v| v.snippet.contains("nve-core-app -> dioxus-rsx-macro")),
        "normal core crate dep on proc-macro dioxus-* must be caught: {v_dx:?}"
    );

    // 3. Workspace-mediated `core -> workspace -> web-sys` behind `cfg(wasm32)` must be caught,
    //    while third-party `getrandom -> wasm-bindgen` behind `cfg(wasm32)` is allowed.
    let v_plat = dag::check_core_zero_platform_crates(&graph);
    assert!(
        v_plat
            .iter()
            .any(|v| v.snippet == "nve-core-app -> nve-core-helper -> web-sys"),
        "workspace-mediated wasm32 web-sys chain must be caught: {v_plat:?}"
    );
    assert!(
        !v_plat.iter().any(|v| v.snippet.contains("getrandom")),
        "third-party wasm32 shim getrandom -> wasm-bindgen must not false-positive: {v_plat:?}"
    );
}

#[test]
fn unit_guppy_dag_catches_iterative_workspace_proc_macro_and_build_chains() {
    let json = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/core-app#0.1.0",
            "path+file:///repo/packages/core/core-derive#0.1.0",
            "path+file:///repo/packages/tools/build-helper#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/core-app#0.1.0", "name": "nve-core-app", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/core-app/Cargo.toml", "targets": {t_app}, "features": {{}},
                "dependencies": [
                    {{"name": "nve-core-derive", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/core/core-derive"}},
                    {{"name": "nve-build-helper", "req": "*", "kind": "build", "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/tools/build-helper"}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/core/core-derive#0.1.0", "name": "nve-core-derive", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/core-derive/Cargo.toml", "targets": {t_derive}, "features": {{}},
                "dependencies": [
                    {{"name": "serde_json", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/tools/build-helper#0.1.0", "name": "nve-build-helper", "version": "0.1.0",
                "manifest_path": "/repo/packages/tools/build-helper/Cargo.toml", "targets": {t_helper}, "features": {{}},
                "dependencies": [
                    {{"name": "dioxus-core", "req": "^0.7", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "name": "serde_json", "version": "1.0.0", "manifest_path": "/cargo/serde_json/Cargo.toml", "targets": {t_sj}, "features": {{}}, "dependencies": []}},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "name": "dioxus-core", "version": "0.7.0", "manifest_path": "/cargo/dioxus-core/Cargo.toml", "targets": {t_dx}, "features": {{}}, "dependencies": []}}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/core-app#0.1.0",
                    "dependencies": [
                        "path+file:///repo/packages/core/core-derive#0.1.0",
                        "path+file:///repo/packages/tools/build-helper#0.1.0"
                    ],
                    "deps": [
                        {{"name": "nve_core_derive", "pkg": "path+file:///repo/packages/core/core-derive#0.1.0", "dep_kinds": [{{"kind": null, "target": null}}]}},
                        {{"name": "nve_build_helper", "pkg": "path+file:///repo/packages/tools/build-helper#0.1.0", "dep_kinds": [{{"kind": "build", "target": null}}]}}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/core/core-derive#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0"],
                    "deps": [{{"name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{
                    "id": "path+file:///repo/packages/tools/build-helper#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0"],
                    "deps": [{{"name": "dioxus_core", "pkg": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dependencies": [], "deps": []}},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "dependencies": [], "deps": []}}
            ]
        }}
    }}"#,
        t_app = lib_target("nve_core_app", "/repo/packages/core/core-app"),
        t_derive = proc_macro_target("nve_core_derive", "/repo/packages/core/core-derive"),
        t_helper = lib_target("nve_build_helper", "/repo/packages/tools/build-helper"),
        t_sj = lib_target("serde_json", "/cargo/serde_json"),
        t_dx = lib_target("dioxus_core", "/cargo/dioxus-core"),
    );
    let graph = PackageGraph::from_json(&json).expect("valid iterative workspace chain graph");

    let v_sj = dag::check_zero_serde_json(&graph);
    assert!(
        v_sj.iter()
            .any(|v| v.snippet == "nve-core-app -> nve-core-derive -> serde_json"),
        "expected full workspace proc-macro chain `nve-core-app -> nve-core-derive -> serde_json`: {v_sj:?}"
    );

    let v_dx = dag::check_core_zero_dioxus(&graph);
    assert!(
        v_dx.iter()
            .any(|v| v.snippet == "nve-core-app -> nve-build-helper -> dioxus-core"),
        "expected full workspace build-dep chain `nve-core-app -> nve-build-helper -> dioxus-core`: {v_dx:?}"
    );
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
            pub fn fwd(self) -> &'static str { match self { Self::Left => "l", Self::Center => "c", Self::Right => "r", Self::Justify => "j" } }
            pub fn rev(s: &str) -> Option<Self> { match s { "l" => Some(Self::Left), "c" => Some(Self::Center), "r" => Some(Self::Right), "j" => Some(Self::Justify), _ => None } }
        }
    "#;
    let ast_a = syn::parse_file(core_a_src).unwrap();
    let ast_b = syn::parse_file(core_b_src).unwrap();
    let ast_bk = syn::parse_file(backend_src).unwrap();

    let mut v_fields = Vec::new();
    ast_layout::scan_bare_primitive_domain_fields("a.rs", core_a_src, &ast_a, &mut v_fields);
    assert_eq!(
        v_fields.len(),
        1,
        "cfg(any(test, ...)) must be scanned, cfg(test) skipped: {v_fields:?}"
    );

    let mut v_enum = Vec::new();
    ast_enums::scan_duplicate_enums_and_converters(
        &[
            ("core/a.rs".into(), core_a_src, &ast_a),
            ("core/b.rs".into(), core_b_src, &ast_b),
            ("backends/skia.rs".into(), backend_src, &ast_bk),
        ],
        &mut v_enum,
    );
    assert_eq!(
        v_enum.len(),
        4,
        "2 dup TextAlign + 1 impl From + 1 decode_align (not to_skia_align): {v_enum:?}"
    );
    assert!(!v_enum.iter().any(|v| v.reason.contains("to_skia_align")));

    let mut v_match = Vec::new();
    ast_enums::scan_match_tables("backends/skia.rs", backend_src, &ast_bk, &mut v_match);
    assert_eq!(v_match.len(), 1, "expected 1 compact int decode_align: {v_match:?}");

    let dups = ast_enums::scan_duplicate_enum_match_arms(&[("backends/skia.rs", backend_src, &ast_bk)]);
    assert_eq!(dups.len(), 1, "expected 1 mirrored fwd/rev from fingerprint scanner: {dups:?}");

    let abi_a = syn::parse_file("#[derive(NveAbi)] #[nve_abi(size = 32)] #[repr(C)] pub struct NveNodeRow { pub x: u32 }\n#[repr(C)] pub struct NveApi { pub f: usize }").unwrap();
    let abi_b = syn::parse_file("const _: () = { assert!(size_of::<NveApi>() == 8); assert!(align_of::<NveApi>() == 8); assert!(offset_of!(NveApi, f) == 0); };").unwrap();
    let store_a = syn::parse_file("#[repr(C)] pub struct NodeRow { pub x: u32 }\nconst _: () = { assert!(size_of::<NodeRow>() == 32); };").unwrap();
    let v_abi = ast_layout::scan_repr_c_abi_locks(
        &[("abi/a.rs".into(), &abi_a), ("abi/b.rs".into(), &abi_b)],
        &[("store/row.rs".into(), &store_a)],
    );
    assert_eq!(
        v_abi.len(),
        1,
        "only missing NveEvent should fail: {v_abi:?}"
    );
    assert!(v_abi[0].reason.contains("`NveEvent`"));

    assert!(hygiene::detect_personal_endpoint_or_path("/Users/alice/dev", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("https://box.tail.ts.net", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", true).is_none());
    assert!(hygiene::detect_personal_endpoint_or_path("127.0.0.1", false).is_none());
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("packages/core/engine/src/mmap_arena/mod.rs"),
        Some("mmap_arena")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("packages/backends/skia/src/reloc_scene.rs"),
        Some("reloc_scene")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path(
            "cxx/nve-fabric-bridge/include/nve_snapshot.hpp"
        ),
        Some("nve_snapshot")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("hosts/skia-shell/src/window/instant_boot.rs"),
        Some("instant_boot")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("packages/core/engine/src/Mmap-Arena/mod.rs"),
        Some("mmap_arena")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("packages/core/engine/src/reloc/node.rs"),
        Some("/reloc/")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("packages/core/engine/src/reloc.rs"),
        Some("/reloc/")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("cxx/nve-fabric-bridge/include/RELOC.hpp"),
        Some("/reloc/")
    );
    assert_eq!(
        hygiene::is_banned_legacy_persistence_path("cxx/tests/test-mmap-snapshot.cpp"),
        Some("mmap_snapshot")
    );
    assert!(
        hygiene::is_banned_legacy_persistence_path("packages/core/engine/src/persistence.rs")
            .is_none()
    );
    assert!(
        hygiene::is_banned_legacy_persistence_path(
            "packages/backends/headless/benches/frame0_boot.rs"
        )
        .is_none()
    );
    assert!(
        hygiene::is_banned_legacy_persistence_path("packages/core/store/src/relocated_rows.rs")
            .is_none()
    );
    assert_eq!(
        hygiene::detect_banned_legacy_persistence_symbol("pub struct MmapOffsetArena;"),
        Some("MmapOffsetArena")
    );
    assert_eq!(
        hygiene::detect_banned_legacy_persistence_symbol("let n: RelocSceneNode = x;"),
        Some("RelocSceneNode")
    );
    assert_eq!(
        hygiene::detect_banned_legacy_persistence_symbol("use crate::RelocSceneRoot;"),
        Some("RelocSceneRoot")
    );
    assert_eq!(
        hygiene::detect_banned_legacy_persistence_symbol("RelocatableStateArena::open()"),
        Some("RelocatableStateArena")
    );
    assert!(
        hygiene::detect_banned_legacy_persistence_symbol("Frame0File::boot_into_replica").is_none()
    );
    assert!(hygiene::detect_banned_legacy_persistence_symbol("MyRelocSceneNodeHelper").is_none());

    let memmap_json = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/core/dioxus#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/engine#0.1.0", "name": "nve-engine", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml", "targets": {t_eng}, "features": {{}},
                "dependencies": [
                    {{"name": "helper-crate", "req": "^0.1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/core/dioxus#0.1.0", "name": "nve-dioxus", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/dioxus/Cargo.toml", "targets": {t_dx}, "features": {{}},
                "dependencies": [
                    {{"name": "subsecond", "req": "^0.7", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "registry+https://github.com/rust-lang/crates.io-index#helper-crate@0.1.0", "name": "helper-crate", "version": "0.1.0",
                "manifest_path": "/cargo/helper-crate/Cargo.toml", "targets": {t_hlp}, "features": {{}},
                "dependencies": [
                    {{"name": "memmap2", "req": "^0.9", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{
                "id": "registry+https://github.com/rust-lang/crates.io-index#subsecond@0.7.0", "name": "subsecond", "version": "0.7.0",
                "manifest_path": "/cargo/subsecond/Cargo.toml", "targets": {t_sub}, "features": {{}},
                "dependencies": [
                    {{"name": "memmap2", "req": "^0.9", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0", "name": "memmap2", "version": "0.9.0", "manifest_path": "/cargo/memmap2/Cargo.toml", "targets": {t_mm}, "features": {{}}, "dependencies": []}}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/engine#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#helper-crate@0.1.0"],
                    "deps": [{{"name": "helper_crate", "pkg": "registry+https://github.com/rust-lang/crates.io-index#helper-crate@0.1.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{
                    "id": "path+file:///repo/packages/core/dioxus#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#subsecond@0.7.0"],
                    "deps": [{{"name": "subsecond", "pkg": "registry+https://github.com/rust-lang/crates.io-index#subsecond@0.7.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{
                    "id": "registry+https://github.com/rust-lang/crates.io-index#helper-crate@0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0"],
                    "deps": [{{"name": "memmap2", "pkg": "registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{
                    "id": "registry+https://github.com/rust-lang/crates.io-index#subsecond@0.7.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0"],
                    "deps": [{{"name": "memmap2", "pkg": "registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#memmap2@0.9.0", "dependencies": [], "deps": []}}
            ]
        }}
    }}"#,
        t_eng = lib_target("nve_engine", "/repo/packages/core/engine"),
        t_dx = lib_target("nve_dioxus", "/repo/packages/core/dioxus"),
        t_hlp = lib_target("helper_crate", "/cargo/helper-crate"),
        t_sub = lib_target("subsecond", "/cargo/subsecond"),
        t_mm = lib_target("memmap2", "/cargo/memmap2"),
    );
    let mm_graph = PackageGraph::from_json(&memmap_json).expect("valid memmap2 synthetic graph");
    let mm_v = dag::check_no_memmap2_dep(&mm_graph);
    assert!(
        mm_v.iter()
            .any(|v| v.snippet == "nve-engine -> helper-crate -> memmap2"),
        "transitive memmap2 in packages/core/engine must fail check_no_memmap2_dep: {mm_v:?}"
    );
    assert!(
        !mm_v.iter().any(|v| v.snippet.contains("nve-dioxus")),
        "transitive subsecond -> memmap2 in packages/core/dioxus must not false-positive: {mm_v:?}"
    );
    assert!(bench::check_bench_not_stub("fn main() {}").is_some());
    assert!(
        bench::check_bench_not_stub(
            "use std::hint::black_box; fn b(c: &mut Criterion) { let _ = 1; }"
        )
        .is_some(),
        "unused black_box import without call must fail anti-stub check"
    );
    assert!(
        bench::check_bench_not_stub(
            "use std::hint::black_box; fn b(c: &mut Criterion) { black_box(1); }"
        )
        .is_none()
    );

    // Multi-file benchmark group: both files have inner doc comments + inner `#![allow(dead_code)]`.
    let bench_main = "//! Main bench\n#![allow(dead_code)]\nmod sub;\ncriterion_main!(benches);\n";
    let bench_sub = "//! Sub bench\n#![allow(dead_code)]\nuse std::hint::black_box;\nfn run() { black_box(42); }\n";
    assert!(
        bench::check_bench_group_not_stub(&[("main.rs", bench_main), ("sub.rs", bench_sub)])
            .is_none(),
        "multi-file benchmark with inner attrs in each file must parse and pass"
    );
    assert!(
        bench::check_bench_group_not_stub(&[("main.rs", "fn broken(")])
            .is_some_and(|msg| msg.contains("Failed to parse")),
        "unparseable benchmark file must fail loudly"
    );
}

#[test]
fn unit_restored_active_and_staged_scanners_positive_and_negative() {
    let inner_test_src = "#![cfg(test)]\npub const BAD_SCHEMA_HASH: u64 = 0x1234;\npub fn write_css() {}\npub fn hot() { let _ = std::str::from_utf8(b\"x\"); let _ = x.downcast_ref::<u32>(); }";
    let inner_ast = syn::parse_file(inner_test_src).unwrap();
    let mut v = Vec::new();
    ast_layout::scan_hardcoded_schema_hashes("a.rs", inner_test_src, &inner_ast, &mut v);
    purity::scan_platform_purity(
        "packages/core/abi/src/a.rs",
        inner_test_src,
        &inner_ast,
        &mut v,
    );
    hot_path::scan_hot_path_ast(
        "packages/core/store/src/a.rs",
        inner_test_src,
        &inner_ast,
        true,
        &mut v,
    );
    hot_path::scan_downcast_ref_ast(
        "packages/backends/web/src/a.rs",
        inner_test_src,
        &inner_ast,
        &mut v,
    );
    assert!(
        v.is_empty(),
        "inner #![cfg(test)] must skip entire file: {v:?}"
    );

    let nested_hash = "mod inner { impl Foo { const MY_SCHEMA_HASH: u64 = 0xDEAD_BEEF; } }";
    let nested_ast = syn::parse_file(nested_hash).unwrap();
    let mut vh = Vec::new();
    ast_layout::scan_hardcoded_schema_hashes("a.rs", nested_hash, &nested_ast, &mut vh);
    assert_eq!(
        vh.len(),
        1,
        "expected nested impl SCHEMA_HASH violation: {vh:?}"
    );

    let web_src = "impl WebBackend { pub fn new() { #[cfg(feature = \"camera\")] let _ = 1; } }";
    let web_ast = syn::parse_file(web_src).unwrap();
    let mut vw = Vec::new();
    assert!(dag::scan_web_backend_ast(
        "web.rs", web_src, &web_ast, &mut vw
    ));
    assert_eq!(vw.len(), 1, "expected camera cfg violation: {vw:?}");

    let hot_src = r#"
        fn new() { let _ = String::new(); let _ = std::str::from_utf8(b"ok"); }
        fn hot_step(b: &[u8]) {
            debug_assert!(std::str::from_utf8(b).is_ok());
            let _ = std::str::from_utf8(b);
            let _ = format!("hi");
        }
    "#;
    let hot_ast = syn::parse_file(hot_src).unwrap();
    let mut vhot = Vec::new();
    hot_path::scan_hot_path_ast(
        "packages/core/store/src/store.rs",
        hot_src,
        &hot_ast,
        true,
        &mut vhot,
    );
    assert_eq!(
        vhot.len(),
        2,
        "expected from_utf8 + format! violations only in allowlisted store.rs: {vhot:?}"
    );

    let non_allow_src = "fn new() { let _ = format!(\"alloc\"); }";
    let non_allow_ast = syn::parse_file(non_allow_src).unwrap();
    let mut v_non_allow = Vec::new();
    hot_path::scan_hot_path_ast(
        "packages/core/store/src/str_arena.rs",
        non_allow_src,
        &non_allow_ast,
        true,
        &mut v_non_allow,
    );
    assert_eq!(
        v_non_allow.len(),
        1,
        "arbitrary fn new() must not be exempted: {v_non_allow:?}"
    );

    let trait_src = "trait T { fn try_str(&self) -> Option<&str>; }\nfn hot(b: &[u8]) { let _ = std::str::from_utf8(b); }";
    let trait_ast = syn::parse_file(trait_src).unwrap();
    let mut v_trait = Vec::new();
    hot_path::scan_hot_path_ast(
        "packages/core/store/src/span_pool.rs",
        trait_src,
        &trait_ast,
        true,
        &mut v_trait,
    );
    assert_eq!(
        v_trait.len(),
        1,
        "trait decl must not exempt next fn: {v_trait:?}"
    );

    // Per-node downcast_ref AST check (catches method w/ and w/o turbofish + Any::downcast_ref, skips #[cfg(test)]).
    let dc_src = r#"
        fn prod(a: &dyn std::any::Any) {
            let _ = a.downcast_ref::<u32>();
            let _: Option<&u32> = a.downcast_ref();
            let _ = std::any::Any::downcast_ref::<u32>(a);
        }
        #[cfg(test)]
        fn ok_in_test(a: &dyn std::any::Any) { let _ = a.downcast_ref::<u32>(); }
    "#;
    let dc_ast = syn::parse_file(dc_src).unwrap();
    let mut vdc = Vec::new();
    hot_path::scan_downcast_ref_ast("packages/backends/web/src/x.rs", dc_src, &dc_ast, &mut vdc);
    assert_eq!(
        vdc.len(),
        3,
        "expected 3 prod downcast_ref violations only: {vdc:?}"
    );

    assert!(hot_path::line_has_fnv1a("use fnv::FnvHashMap;"));
    assert!(hot_path::line_has_fnv1a(
        "const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;"
    ));
    assert!(!hot_path::line_has_fnv1a("use nve_hash::xxh64;"));

    let tbl_src = r#"
        #[derive(NveTable)]
        pub struct BadTable {
            #[bits(16)] pub small: u16,
            #[default(u32::MAX)] pub sentinel: u32,
            pub has_color: bool,
            pub color: u32,
        }
        pub enum BadAlign { Unset, Left }
        pub const SET_COLOR: u32 = 1;
        #[repr(C)] pub struct Pod { pub x: u32 }
        impl Pod { pub fn encode_bytes(&self) {} }
    "#;
    let tbl_ast = syn::parse_file(tbl_src).unwrap();
    let mut vt = Vec::new();
    purity::scan_table_and_codec_hygiene(
        "packages/core/tables/src/t.rs",
        tbl_src,
        &tbl_ast,
        &mut vt,
    );
    assert_eq!(
        vt.len(),
        6,
        "expected bits + default + split bool + Unset + SET_ + encode_bytes: {vt:?}"
    );

    // Dart 53-bit codegen check: every line alone, `true` = must be flagged.
    for (line, unsafe_) in [
        ("const int H = 0x1234567890ABCDEF;", true),
        ("const int W = 0x12345678901234;", true),
        ("static const int A = 0x{hash:016X};", true),
        ("static const int B = 0x{hash:X};", true),
        ("static const int C = 0x{hash};", true),
        ("final int D = 0x{lo:08X} + 0x{hi:x};", true),
        ("static const int U = 0X{h:016X};", true),
        ("const int E = 0x{hi:08X}{lo:08X};", true),
        ("const int F = 0x{hi:08X}00000000;", true),
        ("const int G = {h:#018X};", true),
        ("const int I = {h:#x};", true),
        ("const int J = 0x{h:016X}; // not BigInt", true),
        ("const int L = 0x1234567890ABCDEF; // BigInt.parse elsewhere", true),
        ("const int L2 = 0x1234567890ABCDEF; /* BigInt */", true),
        ("const int L3 = /* BigInt */ 0x{h:016X};", true),
        ("const int T = 0x10000000{lo:08X};", true),
        ("const int V = 0x{hi:08X}_{lo:08X};", true),
        ("const int X = {h:>#018X};", true),
        ("const int Y = {h:0<#018x};", true),
        (r#"w(out, "const int A = 0x{a:08X}; // 0x{wide:016X} inside the emitted text");"#, true),
        (r#"w(out, "// head\nconst int ID = 0x{id:016X};");"#, true),
        (r#"w(out, "const url = 'https://x.dev'; const int ID = 0x{id:016X};");"#, true),
        (r##"w(out, r#"const int R = 0x{r:016X}; " // "x"#); // BigInt"##, true),
        (r##"w(out, cr#"const int C = 0x{c:016X}; " // "x"#); // BigInt"##, true),
        (r#"w(out, cr"const int C2 = 0x{c:016X}; \"); // BigInt"#, true),
        (r#"if cond { w(out, "const int A = 0x{a:016X};"); }"#, true),
        (r#"fn f<'a>(x: &'a str) { w(out, "const int Q = 0x{q:016X};"); }"#, true),
        (r#"if cond { w(out, "const int A = 0x{a:08X}; const int B = 0x{b:X};"); }"#, true),
        (r#"w(out, "const int Z = 0x{z:016X};", '"'); // BigInt"#, true),
        ("  static const int interfaceId = 0x{interface_id:08X};", false),
        ("const int K = 0x{k:04x};", false),
        ("const int S = 0x{s:02X};", false),
        ("const int M = {m:#010X};", false),
        ("const int M2 = {m:>#010X};", false),
        (r#"if cond { w(out, "const int A = 0x{a:08X};"); }"#, false),
        ("const int M3 = 0x{m:08X}; /* 0x{wide:016X} */", false),
        (r#"w(out, "const String u = 'https://x.dev/{path}';"); // 0x{wide:016X}"#, false),
        (r#"fn f<'a>(x: &'a str) { w(out, "const int Q = 0x{q:08X};"); } // 0x{h:016X}"#, false),
        ("const int Q = 0x{id:08X}; // 0x{wide:016X} in a comment", false),
        ("const int N = 0x1234567890123;", false),
        ("const String name = '{label}';", false),
        ("const int R = 0x{{literal}};", false),
        ("final h = BigInt.parse('0x{hash:016X}');", false),
        ("final BigInt H = BigInt.parse('1234567890ABCDEF', radix: 16); // 0x1234567890ABCDEF", false),
    ] {
        let mut vd = Vec::new();
        codegen_53bit::scan_dart_53bit_safety("dart/a.rs", line, &mut vd);
        assert_eq!(vd.len(), usize::from(unsafe_), "{line}: {vd:?}");
    }
    // Block comments and strings span lines: line 2 ends a comment (its `BigInt` is no exemption),
    // line 4 is inside a string opened on line 3 (its `//` is emitted text, not a comment).
    let mut vd = Vec::new();
    codegen_53bit::scan_dart_53bit_safety(
        "dart/a.rs",
        "/* a\n BigInt */ const int A = 0x{a:016X};\nw(out, \"head\n// x const int B = 0x{b:016X};\");\nconst int C = 0x{c:08X}; /* 0x{w:016X}\n0x{w:016X} */",
        &mut vd,
    );
    assert_eq!(vd.iter().map(|v| v.line).collect::<Vec<_>>(), [2, 4], "{vd:?}");

    let mut vts_bad = Vec::new();
    codegen_53bit::scan_ts_53bit_safety(
        "typescript/a.rs",
        "const id = Number(view.getBigUint64(0, true));\nconst signed = Number(view.getBigInt64(0, true));\nconst x = hi * 4294967296 + lo;\nconst y = hi << 32;",
        &mut vts_bad,
    );
    assert_eq!(
        vts_bad.len(),
        4,
        "expected 4 TS 53-bit violations: {vts_bad:?}"
    );

    let mut vts_ok = Vec::new();
    codegen_53bit::scan_ts_53bit_safety(
        "typescript/a.rs",
        "const id: bigint = view.getBigUint64(0, true);\nconst hi = BigInt(h) << 32n;\nconst lo = view.getUint32(0, true); // Number(getBigUint64) would lose bits",
        &mut vts_ok,
    );
    assert!(vts_ok.is_empty(), "safe BigInt TS must pass: {vts_ok:?}");
    let mut vts_comment = Vec::new();
    codegen_53bit::scan_ts_53bit_safety("typescript/a.rs", "const y = hi << 32; // BigInt", &mut vts_comment);
    assert_eq!(vts_comment.len(), 1, "a comment cannot exempt a line: {vts_comment:?}");

    // Multi-platform guppy CargoSet catches optional third-party features & macOS-only edges.
    let multi_json = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": ["path+file:///repo/packages/core/foo#0.1.0"],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/foo#0.1.0", "name": "nve-foo", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/foo/Cargo.toml", "targets": {t_foo},
                "features": {{"opt": ["third-party/json"]}},
                "dependencies": [
                    {{"name": "third-party", "req": "^1", "kind": null, "optional": false, "uses_default_features": false, "features": [], "target": "cfg(target_os = \"macos\")"}}
                ]
            }},
            {{
                "id": "registry+https://github.com/rust-lang/crates.io-index#third-party@1.0.0", "name": "third-party", "version": "1.0.0",
                "manifest_path": "/cargo/third-party/Cargo.toml", "targets": {t_tp},
                "features": {{"json": ["dep:serde_json"]}},
                "dependencies": [
                    {{"name": "serde_json", "req": "^1", "kind": null, "optional": true, "uses_default_features": true, "features": [], "target": null}}
                ]
            }},
            {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "name": "serde_json", "version": "1.0.0", "manifest_path": "/cargo/serde_json/Cargo.toml", "targets": {t_sj}, "features": {{}}, "dependencies": []}}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/foo#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#third-party@1.0.0"],
                    "deps": [{{"name": "third_party", "pkg": "registry+https://github.com/rust-lang/crates.io-index#third-party@1.0.0", "dep_kinds": [{{"kind": null, "target": "cfg(target_os = \"macos\")"}}]}}]
                }},
                {{
                    "id": "registry+https://github.com/rust-lang/crates.io-index#third-party@1.0.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0"],
                    "deps": [{{"name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{"kind": null, "target": null}}]}}]
                }},
                {{"id": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dependencies": [], "deps": []}}
            ]
        }}
    }}"#,
        t_foo = lib_target("nve_foo", "/repo/packages/core/foo"),
        t_tp = lib_target("third_party", "/cargo/third-party"),
        t_sj = lib_target("serde_json", "/cargo/serde_json"),
    );
    let multi_graph = PackageGraph::from_json(&multi_json).expect("valid multi-platform graph");
    let v_sj = dag::check_zero_serde_json(&multi_graph);
    assert!(
        v_sj.iter()
            .any(|x| x.snippet.contains("nve-foo -> third-party -> serde_json")),
        "CargoSet must catch macOS-gated optional third-party serde_json: {v_sj:?}"
    );
}

#[test]
fn unit_generated_banner_naming_exempts_codegen_templates() {
    use nve_arch_invariants::hygiene::misnamed_generated_banner;
    let banner = "/* @generated by nve-codegen. DO NOT EDIT. */\n#pragma once\n";
    assert_eq!(misnamed_generated_banner("packages/core/codegen/templates/c/nve_abi.h", banner), None);
    assert_eq!(misnamed_generated_banner("packages/core/host-cxx/include/nve/generated/nve_abi.generated.h", banner), None);
    assert_eq!(misnamed_generated_banner("packages/core/host-java/src/main/java/dev/nve/host/generated/NveAbi.java", "// @generated\n"), None);
    assert_eq!(misnamed_generated_banner("hosts/x/nve_abi.h", banner), Some((1, "/* @generated by nve-codegen. DO NOT EDIT. */")));
    assert_eq!(misnamed_generated_banner("hosts/x/plain.h", "#pragma once\n"), None);
}

#[test]
fn single_scene_abi_detects_retired_symbols_and_bridge_tokens() {
    use nve_arch_invariants::scene_abi_guard::detect_retired_scene_reference;
    assert_eq!(detect_retired_scene_reference("let n: NveRenderNode = x;"), Some("NveRenderNode"));
    assert_eq!(detect_retired_scene_reference("#include <nve_scene_abi.h> // NveSceneBuffer"), Some("NveSceneBuffer"));
    assert_eq!(detect_retired_scene_reference("cmake -S cxx/nve-fabric-bridge -B b"), Some("nve_fabric_bridge"));
    assert_eq!(detect_retired_scene_reference("target_link_libraries(x NVE_FABRIC_BRIDGE)"), Some("nve_fabric_bridge"));
    assert_eq!(detect_retired_scene_reference("nve_fabric_host_create(cfg);"), Some("nve_fabric_host"));
    assert_eq!(detect_retired_scene_reference("class NveFabricHost {};"), Some("nve_fabric_host"));
    assert_eq!(detect_retired_scene_reference("struct NveFabricBridge {};"), Some("nve_fabric_bridge"));
    assert_eq!(detect_retired_scene_reference("let x = nve_render_node_size;"), Some("nve_render_node"));
    assert_eq!(detect_retired_scene_reference("const NveFrameStats* s;"), Some("NveFrameStats"));
    assert_eq!(detect_retired_scene_reference("const m = new SceneMirror();"), Some("SceneMirror"));
    assert_eq!(detect_retired_scene_reference("m.nve_web_scene_view_ptr"), Some("nve_web_scene_view_ptr"));
    // Identifier boundaries: the canonical names and look-alikes pass.
    assert_eq!(detect_retired_scene_reference("const NveSceneView* v; SceneReader r;"), None);
    assert_eq!(detect_retired_scene_reference("MyNveRenderNodeish x;"), None);
    assert_eq!(detect_retired_scene_reference("NveRuntimeSceneViews"), None);
    // Runtime-prefixed reader names retired in Slice 2B-5 (the runtime ABI's own
    // `NveRuntimeSceneAcquire*` typedefs are different identifiers and pass).
    assert_eq!(detect_retired_scene_reference("final r = readRuntimeScene(v);"), Some("readRuntimeScene"));
    assert_eq!(detect_retired_scene_reference("extension V: RuntimeSceneViewABI {}"), Some("RuntimeSceneViewABI"));
    assert_eq!(detect_retired_scene_reference("let s: NveRuntimeScene"), Some("NveRuntimeScene"));
    assert_eq!(detect_retired_scene_reference("typedef NveRuntimeSceneAcquireNative = x;"), None);
}

#[test]
fn unit_platform_purity_flags_css_html_formatters_and_motion_in_core_only() {
    let scan = |rel: &str, src: &str| {
        let ast = syn::parse_file(src).expect("parse");
        let mut v = Vec::new();
        purity::scan_platform_purity(rel, src, &ast, &mut v);
        v
    };
    for bad in [
        "pub fn write_css_into(out: &mut String) {}",
        "impl X { pub const fn as_css(self) -> &'static str { \"\" } }",
        "impl X { pub const fn as_html_type(self) -> &'static str { \"\" } }",
        "impl X { pub const fn to_css_keyword(&self) -> &'static str { \"\" } }",
        "pub trait Fmt { fn format_css(&self); }",
        "pub fn parse_box_shadow(raw: &str) {}",
        "pub mod motion {}",
        "pub struct S { clock: MotionClock }",
    ] {
        assert_eq!(scan("packages/core/interop/src/a.rs", bad).len(), 1, "core must flag: {bad}");
        assert!(scan("packages/backends/web/src/a.rs", bad).is_empty(), "backends may: {bad}");
    }
    assert_eq!(scan("packages/core/engine/src/motion.rs", "").len(), 1, "motion-named core file");
    for good in [
        "pub fn whatwg_token() {}",
        "pub struct ReducedMotion(bool);",
        "#[cfg(test)] mod tests { fn write_css() {} }",
    ] {
        assert!(scan("packages/core/tables/src/a.rs", good).is_empty(), "allowed: {good}");
    }
}

#[test]
fn unit_no_manual_repr_enum_converters_positive_and_negative() {
    let bad_src = r#"
        #[repr(u8)]
        pub enum PubU8 { A = 0, B = 1 }
        impl PubU8 {
            pub const fn from_u8(v: u8) -> Option<Self> {
                match v { 0 => Some(Self::A), 1 => Some(Self::B), _ => None }
            }
        }
        #[repr(i32)]
        enum PrivI32 { Ok = 0, Err = -1, Bad = -2 }
        impl PrivI32 {
            const fn from_raw(code: i32) -> Self {
                match code { 0 => Self::Ok, -1 => Self::Err, -2 => Self::Bad, _ => Self::Err }
            }
        }
        const fn helper_decode(v: u8) -> Option<PubU8> {
            match v { 0 => Some(PubU8::A), 1 => Some(PubU8::B), _ => None }
        }
    "#;
    let cross_file_bad = r#"
        fn cross_decode(raw: u8) -> Option<PubU8> {
            match raw { 0 => Some(PubU8::A), 1 => Some(PubU8::B), _ => None }
        }
    "#;
    let good_src = r#"
        #[repr(u8)]
        pub enum ColorScheme { Light = 1, Dark = 2 }
        impl ColorScheme {
            pub const fn opt_from_u8(raw: u8) -> Result<Option<Self>, u8> {
                if raw == 0 {
                    Ok(None)
                } else {
                    match Self::try_from_u8(raw) {
                        Some(s) => Ok(Some(s)),
                        None => Err(raw),
                    }
                }
            }
        }
        #[repr(u8)]
        pub enum CameraFacing { Unknown = 0, Back = 1, Front = 2 }
        impl CameraFacing {
            pub const fn from_preview_u8(v: u8) -> Option<Self> {
                match Self::try_from_word(v as u32) {
                    Some(Self::Unknown) | None => None,
                    some => some,
                }
            }
        }
        pub enum RemoteAction { Move(u8), Digit(u8) }
        impl RemoteAction {
            pub fn from_code(v: u32) -> Option<Self> {
                match v { 1 => Some(Self::Move(1)), 2 => Some(Self::Digit(2)), _ => None }
            }
        }
        pub struct ColorRgba(pub u32);
        impl ColorRgba {
            pub const fn from_u32(v: u32) -> Self { Self(v) }
        }
    "#;

    let ast_bad = syn::parse_file(bad_src).unwrap();
    let ast_cross = syn::parse_file(cross_file_bad).unwrap();
    let ast_good = syn::parse_file(good_src).unwrap();

    let mut v_bad = Vec::new();
    ast_enums::scan_manual_repr_enum_converters(
        &[
            ("packages/a/src/lib.rs".into(), bad_src, &ast_bad),
            ("packages/b/src/lib.rs".into(), cross_file_bad, &ast_cross),
        ],
        &mut v_bad,
    );
    assert_eq!(
        v_bad.len(),
        4,
        "expected pub from_u8 + private signed from_raw + same-file helper + cross-file helper: {v_bad:?}"
    );

    let mut v_good = Vec::new();
    ast_enums::scan_manual_repr_enum_converters(
        &[("packages/c/src/lib.rs".into(), good_src, &ast_good)],
        &mut v_good,
    );
    assert!(
        v_good.is_empty(),
        "delegators, non-unit enums, and structs must not false-positive: {v_good:?}"
    );
}
