//! Positive & negative unit tests for `nve-arch-invariants` scanners (`guppy` DAG, `syn` AST,
//! `#[cfg(test)]` soundness, per-node `downcast_ref`, Dart/TypeScript 53-bit, and multi-file benches).

use guppy::graph::PackageGraph;
use nve_arch_invariants::{
    ast_enums, ast_layout, bench, dag, hot_path, hygiene, is_cfg_test_attr, purity,
};

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
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(all(test, feature = \"extra\"))] fn f() {}")));
    assert!(is_cfg_test_attr(&parse_attrs("#[cfg(all(unix, all(test, not(windows))))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(any(test, feature = \"extra\"))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(not(test))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(feature = \"test-utils\")] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(all(not(test), unix))] fn f() {}")));
    assert!(!is_cfg_test_attr(&parse_attrs("#[cfg(all(any(test, unix), feature = \"foo\"))] fn f() {}")));
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
    assert!(dag::check_packages_to_hosts_deps(&graph)
        .iter()
        .any(|v| v.snippet.contains("nve-engine -> nve-skia-shell")));

    let mod_v = dag::check_module_root_spec_and_wasm32_dev_deps(&graph);
    assert!(mod_v.iter().any(|v| v.snippet.contains("criterion")));
    assert!(!mod_v.iter().any(|v| v.snippet.contains("skia-safe")));
    assert!(mod_v.iter().any(|v| v.reason.contains("mix `web` and `skia`")));
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
        v_sj.iter().any(|v| v.snippet.contains("nve-core-macro -> serde_json")),
        "proc-macro core crate dep on serde_json must be caught: {v_sj:?}"
    );

    // 2. Normal core crate depending on proc-macro `dioxus-rsx-macro` must be caught.
    let v_dx = dag::check_core_zero_dioxus(&graph);
    assert!(
        v_dx.iter().any(|v| v.snippet.contains("nve-core-app -> dioxus-rsx-macro")),
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

    let abi_a = syn::parse_file("#[derive(NveAbi)] #[nve_abi(size = 64)] #[repr(C)] pub struct NveRenderNode { pub x: u32 }\n#[repr(C)] pub struct NveApi { pub f: usize }").unwrap();
    let abi_b = syn::parse_file("const _: () = { assert!(size_of::<NveApi>() == 8); assert!(align_of::<NveApi>() == 8); assert!(offset_of!(NveApi, f) == 0); };").unwrap();
    let store_a = syn::parse_file("#[repr(C)] pub struct NodeRow { pub x: u32 }\nconst _: () = { assert!(size_of::<NodeRow>() == 32); };").unwrap();
    let v_abi = ast_layout::scan_repr_c_abi_locks(
        &[("abi/a.rs".into(), &abi_a), ("abi/b.rs".into(), &abi_b)],
        &[("store/row.rs".into(), &store_a)],
    );
    assert_eq!(v_abi.len(), 1, "only missing NveEvent should fail: {v_abi:?}");
    assert!(v_abi[0].reason.contains("`NveEvent`"));

    assert!(hygiene::detect_personal_endpoint_or_path("/Users/alice/dev", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("https://box.tail.ts.net", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", false).is_some());
    assert!(hygiene::detect_personal_endpoint_or_path("192.168.1.5", true).is_none());
    assert!(hygiene::detect_personal_endpoint_or_path("127.0.0.1", false).is_none());
    assert!(bench::check_bench_not_stub("fn main() {}").is_some());
    assert!(
        bench::check_bench_not_stub("use std::hint::black_box; fn b(c: &mut Criterion) { let _ = 1; }")
            .is_some(),
        "unused black_box import without call must fail anti-stub check"
    );
    assert!(bench::check_bench_not_stub("use std::hint::black_box; fn b(c: &mut Criterion) { black_box(1); }").is_none());

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
    purity::scan_platform_purity("packages/core/abi/src/a.rs", inner_test_src, &inner_ast, &mut v);
    hot_path::scan_hot_path_ast("packages/core/store/src/a.rs", inner_test_src, &inner_ast, true, &mut v);
    hot_path::scan_downcast_ref_ast("packages/backends/web/src/a.rs", inner_test_src, &inner_ast, &mut v);
    assert!(v.is_empty(), "inner #![cfg(test)] must skip entire file: {v:?}");

    let nested_hash = "mod inner { impl Foo { const MY_SCHEMA_HASH: u64 = 0xDEAD_BEEF; } }";
    let nested_ast = syn::parse_file(nested_hash).unwrap();
    let mut vh = Vec::new();
    ast_layout::scan_hardcoded_schema_hashes("a.rs", nested_hash, &nested_ast, &mut vh);
    assert_eq!(vh.len(), 1, "expected nested impl SCHEMA_HASH violation: {vh:?}");

    let web_src = "impl WebBackend { pub fn new() { #[cfg(feature = \"camera\")] let _ = 1; } }";
    let web_ast = syn::parse_file(web_src).unwrap();
    let mut vw = Vec::new();
    assert!(dag::scan_web_backend_ast("web.rs", web_src, &web_ast, &mut vw));
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
    hot_path::scan_hot_path_ast("packages/core/store/src/store.rs", hot_src, &hot_ast, true, &mut vhot);
    assert_eq!(vhot.len(), 2, "expected from_utf8 + format! violations only in allowlisted store.rs: {vhot:?}");

    let non_allow_src = "fn new() { let _ = format!(\"alloc\"); }";
    let non_allow_ast = syn::parse_file(non_allow_src).unwrap();
    let mut v_non_allow = Vec::new();
    hot_path::scan_hot_path_ast("packages/core/store/src/str_arena.rs", non_allow_src, &non_allow_ast, true, &mut v_non_allow);
    assert_eq!(v_non_allow.len(), 1, "arbitrary fn new() must not be exempted: {v_non_allow:?}");

    let trait_src = "trait T { fn try_str(&self) -> Option<&str>; }\nfn hot(b: &[u8]) { let _ = std::str::from_utf8(b); }";
    let trait_ast = syn::parse_file(trait_src).unwrap();
    let mut v_trait = Vec::new();
    hot_path::scan_hot_path_ast("packages/core/store/src/span_pool.rs", trait_src, &trait_ast, true, &mut v_trait);
    assert_eq!(v_trait.len(), 1, "trait decl must not exempt next fn: {v_trait:?}");

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
    assert_eq!(vdc.len(), 3, "expected 3 prod downcast_ref violations only: {vdc:?}");

    assert!(hot_path::line_has_fnv1a("use fnv::FnvHashMap;"));
    assert!(hot_path::line_has_fnv1a("const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;"));
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
    purity::scan_table_and_codec_hygiene("packages/core/tables/src/t.rs", tbl_src, &tbl_ast, &mut vt);
    assert_eq!(vt.len(), 6, "expected bits + default + split bool + Unset + SET_ + encode_bytes: {vt:?}");

    // Dart & TypeScript 53-bit codegen checks (positive + negative).
    let mut vd = Vec::new();
    purity::scan_dart_53bit_safety("dart/a.rs", "const int H = 0x1234567890ABCDEF;", &mut vd);
    assert_eq!(vd.len(), 1, "expected Dart 53-bit violation: {vd:?}");

    let mut vts_bad = Vec::new();
    purity::scan_ts_53bit_safety(
        "typescript/a.rs",
        "const id = Number(view.getBigUint64(0, true));\nconst signed = Number(view.getBigInt64(0, true));\nconst x = hi * 4294967296 + lo;\nconst y = hi << 32;",
        &mut vts_bad,
    );
    assert_eq!(vts_bad.len(), 4, "expected 4 TS 53-bit violations: {vts_bad:?}");

    let mut vts_ok = Vec::new();
    purity::scan_ts_53bit_safety(
        "typescript/a.rs",
        "const id: bigint = view.getBigUint64(0, true);\nconst hi = BigInt(h) << 32n;",
        &mut vts_ok,
    );
    assert!(vts_ok.is_empty(), "safe BigInt TS must pass: {vts_ok:?}");

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
        v_sj.iter().any(|x| x.snippet.contains("nve-foo -> third-party -> serde_json")),
        "CargoSet must catch macOS-gated optional third-party serde_json: {v_sj:?}"
    );
}

#[test]
fn unit_handle_hygiene_and_bare_handle_params_positive_and_negative() {
    // 1. Privacy lock in id.rs: catches pub, pub(crate), pub(super), and missing structs.
    let bad_id_rs = r#"
        pub struct NodeId(pub(crate) u32);
        pub struct SurfaceId(pub(super) u32);
    "#;
    let ast = syn::parse_file(bad_id_rs).unwrap();
    let mut v = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/abi/src/id.rs", bad_id_rs, &ast, &mut v);
    assert_eq!(v.len(), 2, "must catch pub(crate) and pub(super) tuple fields: {v:?}");

    let missing_id_rs = "pub struct NodeId(u32);";
    let ast_miss = syn::parse_file(missing_id_rs).unwrap();
    let mut v_miss = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/abi/src/id.rs", missing_id_rs, &ast_miss, &mut v_miss);
    assert_eq!(v_miss.len(), 1, "must fail if SurfaceId struct is missing: {v_miss:?}");

    let good_id_rs = "pub struct NodeId(u32); pub struct SurfaceId(u32);";
    let ast_ok = syn::parse_file(good_id_rs).unwrap();
    let mut v_ok = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/abi/src/id.rs", good_id_rs, &ast_ok, &mut v_ok);
    assert!(v_ok.is_empty(), "valid private handles in id.rs must pass: {v_ok:?}");

    // 2. StoreNodeId anywhere (including inside fn) + raw tuple/struct ctors (ignoring enum variants).
    let bad_user = r#"
        type StoreNodeId = u32;
        enum LocalEnum { NodeId(u32), SurfaceId(u32) }
        fn f() {
            use nve_abi::NodeId as StoreNodeId;
            let _a = NodeId(1);
            let _b = nve_abi::SurfaceId(2);
            let _c = NodeId { 0: 3 };
            let _d = crate::id::SurfaceId { 0: 4 };
            let _ok1 = LocalEnum::NodeId(5);
            let _ok2 = other_mod::LocalEnum::SurfaceId(6);
        }
    "#;
    let ast_u = syn::parse_file(bad_user).unwrap();
    let mut vu = Vec::new();
    ast_layout::scan_handle_hygiene("packages/core/engine/src/foo.rs", bad_user, &ast_u, &mut vu);
    assert_eq!(
        vu.len(),
        6,
        "expected type StoreNodeId + nested use StoreNodeId + 2 tuple ctors + 2 struct ctors (and 0 enum-variant FPs): {vu:?}"
    );

    // 3. Bare & wrapped primitive handle params across fn, nested fn, extern "C", and bare fn types.
    let bad_params = r#"
        extern "C" {
            fn c_entry(surface_id: u32, node_id: *const u32);
        }
        type Callback = fn(frame_id: u64, interface_id: Option<u32>);
        fn outer(surface_id: Option<u32>, node_id: &u32, frame_id: &mut u64) {
            fn inner(node_id: &[u32], surface_id: Vec<u32>, interface_id: *mut u32) {}
        }
        #[wasm_bindgen]
        pub fn js_boundary(surface_id: u32, node_id: u32) {}
        pub fn typed_ok(surface_id: SurfaceId, node_id: Option<NodeId>, frame_id: FrameId) {}
    "#;
    let ast_p = syn::parse_file(bad_params).unwrap();
    let mut vp = Vec::new();
    ast_layout::scan_bare_handle_params("packages/core/engine/src/p.rs", bad_params, &ast_p, &mut vp);
    assert_eq!(
        vp.len(),
        10,
        "expected 2 extern + 2 bare fn + 3 outer + 3 nested inner violations (0 for wasm_bindgen/typed): {vp:?}"
    );
}
