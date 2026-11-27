//! Positive and negative unit tests for `rpc_dag::check_framework_rpc_server_purity`
//! and `rpc_dag::check_zero_fnv_crate_dep` using synthetic `guppy` `PackageGraph` fixtures.

use guppy::graph::PackageGraph;
use nve_arch_invariants::rpc_dag::{check_framework_rpc_server_purity, check_zero_fnv_crate_dep};

fn lib_target(name: &str, dir: &str) -> String {
    format!(
        r#"[{{"kind":["lib"],"crate_types":["lib"],"name":"{name}","src_path":"{dir}/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true}}]"#
    )
}

fn ext_pkg(name: &str, ver: &str) -> String {
    let id = format!("registry+https://github.com/rust-lang/crates.io-index#{name}@{ver}");
    let ident = name.replace('-', "_");
    let targets = lib_target(&ident, &format!("/cargo/{name}"));
    format!(
        r#"{{"id":"{id}","name":"{name}","version":"{ver}","manifest_path":"/cargo/{name}/Cargo.toml","targets":{targets},"features":{{}},"dependencies":[]}}"#
    )
}

fn ext_node(name: &str, ver: &str) -> String {
    format!(
        r#"{{"id":"registry+https://github.com/rust-lang/crates.io-index#{name}@{ver}","dependencies":[],"deps":[]}}"#
    )
}

#[test]
fn unit_framework_rpc_server_purity_positive_and_negative() {
    let bad_meta = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/framework/rpc#0.1.0",
            "path+file:///repo/packages/framework/server#0.1.0",
            "path+file:///repo/packages/framework/http-hyper#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/core/engine#0.1.0",
                "name": "nve-engine", "version": "0.1.0",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml",
                "targets": {t_engine}, "features": {{}},
                "dependencies": [
                    {{ "name": "nve-rpc", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/framework/rpc" }},
                    {{ "name": "fnv", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                "name": "nve-rpc", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/rpc/Cargo.toml",
                "targets": {t_rpc}, "features": {{}},
                "dependencies": [
                    {{ "name": "serde_json", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "tokio", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "web-sys", "req": "^0.3", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/framework/server#0.1.0",
                "name": "nve-server", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/server/Cargo.toml",
                "targets": {t_server}, "features": {{}},
                "dependencies": [
                    {{ "name": "hyper", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "dioxus-core", "req": "^0.7", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                "name": "nve-http-hyper", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/http-hyper/Cargo.toml",
                "targets": {t_hyper}, "features": {{}},
                "dependencies": [
                    {{ "name": "tokio", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "hyper", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "axum", "req": "^0.8", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "serde_json", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {p_sj}, {p_tokio}, {p_websys}, {p_hyper}, {p_dioxus}, {p_axum}, {p_fnv}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/core/engine#0.1.0",
                    "dependencies": [
                        "path+file:///repo/packages/framework/rpc#0.1.0",
                        "registry+https://github.com/rust-lang/crates.io-index#fnv@1.0.7"
                    ],
                    "deps": [
                        {{ "name": "nve_rpc", "pkg": "path+file:///repo/packages/framework/rpc#0.1.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "fnv", "pkg": "registry+https://github.com/rust-lang/crates.io-index#fnv@1.0.7", "dep_kinds": [{{ "kind": null, "target": null }}] }}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                    "dependencies": [
                        "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0"
                    ],
                    "deps": [
                        {{ "name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "tokio", "pkg": "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "web_sys", "pkg": "registry+https://github.com/rust-lang/crates.io-index#web-sys@0.3.0", "dep_kinds": [{{ "kind": null, "target": null }}] }}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/framework/server#0.1.0",
                    "dependencies": [
                        "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0"
                    ],
                    "deps": [
                        {{ "name": "hyper", "pkg": "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "dioxus_core", "pkg": "registry+https://github.com/rust-lang/crates.io-index#dioxus-core@0.7.0", "dep_kinds": [{{ "kind": null, "target": null }}] }}
                    ]
                }},
                {{
                    "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                    "dependencies": [
                        "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#axum@0.8.0",
                        "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0"
                    ],
                    "deps": [
                        {{ "name": "tokio", "pkg": "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "hyper", "pkg": "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "axum", "pkg": "registry+https://github.com/rust-lang/crates.io-index#axum@0.8.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }}
                    ]
                }},
                {n_sj}, {n_tokio}, {n_websys}, {n_hyper}, {n_dioxus}, {n_axum}, {n_fnv}
            ]
        }}
    }}"#,
        t_engine = lib_target("nve_engine", "/repo/packages/core/engine"),
        t_rpc = lib_target("nve_rpc", "/repo/packages/framework/rpc"),
        t_server = lib_target("nve_server", "/repo/packages/framework/server"),
        t_hyper = lib_target("nve_http_hyper", "/repo/packages/framework/http-hyper"),
        p_sj = ext_pkg("serde_json", "1.0.0"),
        p_tokio = ext_pkg("tokio", "1.0.0"),
        p_websys = ext_pkg("web-sys", "0.3.0"),
        p_hyper = ext_pkg("hyper", "1.0.0"),
        p_dioxus = ext_pkg("dioxus-core", "0.7.0"),
        p_axum = ext_pkg("axum", "0.8.0"),
        p_fnv = ext_pkg("fnv", "1.0.7"),
        n_sj = ext_node("serde_json", "1.0.0"),
        n_tokio = ext_node("tokio", "1.0.0"),
        n_websys = ext_node("web-sys", "0.3.0"),
        n_hyper = ext_node("hyper", "1.0.0"),
        n_dioxus = ext_node("dioxus-core", "0.7.0"),
        n_axum = ext_node("axum", "0.8.0"),
        n_fnv = ext_node("fnv", "1.0.7"),
    );

    let bad_graph = PackageGraph::from_json(&bad_meta).expect("valid synthetic guppy metadata");
    let violations = check_framework_rpc_server_purity(&bad_graph);
    assert!(
        violations.iter().any(|v| v.path.contains("core/engine")),
        "must catch core -> framework dep: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/rpc") && v.snippet.contains("serde_json")),
        "must catch serde_json in nve-rpc: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/rpc") && v.snippet.contains("tokio")),
        "must catch tokio in sans-I/O nve-rpc: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/rpc") && v.snippet.contains("web-sys")),
        "must catch web-sys in sans-I/O nve-rpc: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/server") && v.snippet.contains("hyper")),
        "must catch hyper in sans-I/O nve-server: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/server") && v.snippet.contains("dioxus-core")),
        "must catch dioxus-core in sans-I/O nve-server: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/http-hyper") && v.snippet.contains("axum")),
        "must catch runtime axum in nve-http-hyper: {violations:?}"
    );
    assert!(
        violations
            .iter()
            .any(|v| v.path.contains("framework/http-hyper") && v.snippet.contains("serde_json")),
        "must catch serde_json in nve-http-hyper: {violations:?}"
    );

    let fnv_violations = check_zero_fnv_crate_dep(&bad_graph);
    assert!(
        fnv_violations
            .iter()
            .any(|v| v.path.contains("core/engine") && v.snippet.contains("fnv")),
        "must catch fnv crate dependency: {fnv_violations:?}"
    );

    // Clean negative case: http-hyper MAY depend on tokio+hyper; rpc/http-hyper MAY have dev-only serde_json/axum.
    let clean_meta = format!(
        r#"{{
        "version": 1, "workspace_root": "/repo", "target_directory": "/repo/target",
        "workspace_members": [
            "path+file:///repo/packages/framework/rpc#0.1.0",
            "path+file:///repo/packages/framework/server#0.1.0",
            "path+file:///repo/packages/framework/http-hyper#0.1.0"
        ],
        "packages": [
            {{
                "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                "name": "nve-rpc", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/rpc/Cargo.toml",
                "targets": {t_rpc}, "features": {{}},
                "dependencies": [
                    {{ "name": "serde_json", "req": "^1", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/framework/server#0.1.0",
                "name": "nve-server", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/server/Cargo.toml",
                "targets": {t_server}, "features": {{}},
                "dependencies": [
                    {{ "name": "nve-rpc", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/framework/rpc" }}
                ]
            }},
            {{
                "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                "name": "nve-http-hyper", "version": "0.1.0",
                "manifest_path": "/repo/packages/framework/http-hyper/Cargo.toml",
                "targets": {t_hyper}, "features": {{}},
                "dependencies": [
                    {{ "name": "nve-server", "req": "*", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null, "path": "/repo/packages/framework/server" }},
                    {{ "name": "tokio", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "hyper", "req": "^1", "kind": null, "optional": false, "uses_default_features": true, "features": [], "target": null }},
                    {{ "name": "axum", "req": "^0.8", "kind": "dev", "optional": false, "uses_default_features": true, "features": [], "target": null }}
                ]
            }},
            {p_sj}, {p_tokio}, {p_hyper}, {p_axum}
        ],
        "resolve": {{
            "nodes": [
                {{
                    "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                    "dependencies": ["registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0"],
                    "deps": [{{ "name": "serde_json", "pkg": "registry+https://github.com/rust-lang/crates.io-index#serde_json@1.0.0", "dep_kinds": [{{ "kind": "dev", "target": null }}] }}]
                }},
                {{
                    "id": "path+file:///repo/packages/framework/server#0.1.0",
                    "dependencies": ["path+file:///repo/packages/framework/rpc#0.1.0"],
                    "deps": [{{ "name": "nve_rpc", "pkg": "path+file:///repo/packages/framework/rpc#0.1.0", "dep_kinds": [{{ "kind": null, "target": null }}] }}]
                }},
                {{
                    "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                    "dependencies": [
                        "path+file:///repo/packages/framework/server#0.1.0",
                        "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0",
                        "registry+https://github.com/rust-lang/crates.io-index#axum@0.8.0"
                    ],
                    "deps": [
                        {{ "name": "nve_server", "pkg": "path+file:///repo/packages/framework/server#0.1.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "tokio", "pkg": "registry+https://github.com/rust-lang/crates.io-index#tokio@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "hyper", "pkg": "registry+https://github.com/rust-lang/crates.io-index#hyper@1.0.0", "dep_kinds": [{{ "kind": null, "target": null }}] }},
                        {{ "name": "axum", "pkg": "registry+https://github.com/rust-lang/crates.io-index#axum@0.8.0", "dep_kinds": [{{ "kind": "dev", "target": null }}] }}
                    ]
                }},
                {n_sj}, {n_tokio}, {n_hyper}, {n_axum}
            ]
        }}
    }}"#,
        t_rpc = lib_target("nve_rpc", "/repo/packages/framework/rpc"),
        t_server = lib_target("nve_server", "/repo/packages/framework/server"),
        t_hyper = lib_target("nve_http_hyper", "/repo/packages/framework/http-hyper"),
        p_sj = ext_pkg("serde_json", "1.0.0"),
        p_tokio = ext_pkg("tokio", "1.0.0"),
        p_hyper = ext_pkg("hyper", "1.0.0"),
        p_axum = ext_pkg("axum", "0.8.0"),
        n_sj = ext_node("serde_json", "1.0.0"),
        n_tokio = ext_node("tokio", "1.0.0"),
        n_hyper = ext_node("hyper", "1.0.0"),
        n_axum = ext_node("axum", "0.8.0"),
    );

    let clean_graph =
        PackageGraph::from_json(&clean_meta).expect("valid clean synthetic guppy metadata");
    let clean_violations = check_framework_rpc_server_purity(&clean_graph);
    assert!(
        clean_violations.is_empty(),
        "clean framework graph must have 0 violations: {clean_violations:?}"
    );
    assert!(
        check_zero_fnv_crate_dep(&clean_graph).is_empty(),
        "clean graph must have 0 fnv violations"
    );
}
