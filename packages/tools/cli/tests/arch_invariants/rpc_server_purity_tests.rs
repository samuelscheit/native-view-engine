//! Positive and negative unit tests for `check_framework_rpc_server_purity`
//! (`nve-rpc`, `nve-server`, `nve-http-hyper` crate-DAG layer boundaries).

use crate::arch_inv::crate_dag::check_framework_rpc_server_purity;
use crate::json_meta::WorkspaceGraph;

#[test]
fn unit_framework_rpc_server_purity_positive_and_negative() {
    let bad_meta = r#"{
        "workspace_root": "/repo",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/framework/rpc#0.1.0",
            "path+file:///repo/packages/framework/server#0.1.0",
            "path+file:///repo/packages/framework/http-hyper#0.1.0"
        ],
        "packages": [
            {
                "id": "path+file:///repo/packages/core/engine#0.1.0",
                "name": "nve-engine",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "nve-rpc", "kind": null, "optional": false, "target": null, "path": "/repo/packages/framework/rpc" }
                ]
            },
            {
                "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                "name": "nve-rpc",
                "manifest_path": "/repo/packages/framework/rpc/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "serde_json", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "tokio", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "web-sys", "kind": null, "optional": false, "target": null, "path": null }
                ]
            },
            {
                "id": "path+file:///repo/packages/framework/server#0.1.0",
                "name": "nve-server",
                "manifest_path": "/repo/packages/framework/server/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "hyper", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "dioxus-core", "kind": null, "optional": false, "target": null, "path": null }
                ]
            },
            {
                "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                "name": "nve-http-hyper",
                "manifest_path": "/repo/packages/framework/http-hyper/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "tokio", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "hyper", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "axum", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "serde_json", "kind": null, "optional": false, "target": null, "path": null }
                ]
            }
        ],
        "resolve": { "nodes": [] }
    }"#;
    let bad_graph = WorkspaceGraph::parse_metadata_json(bad_meta).expect("valid synthetic metadata");
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

    // Clean negative case: http-hyper MAY depend on tokio+hyper; rpc/server may have dev-only serde_json/tokio.
    let clean_meta = r#"{
        "workspace_root": "/repo",
        "workspace_members": [
            "path+file:///repo/packages/framework/rpc#0.1.0",
            "path+file:///repo/packages/framework/server#0.1.0",
            "path+file:///repo/packages/framework/http-hyper#0.1.0"
        ],
        "packages": [
            {
                "id": "path+file:///repo/packages/framework/rpc#0.1.0",
                "name": "nve-rpc",
                "manifest_path": "/repo/packages/framework/rpc/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "serde_json", "kind": "dev", "optional": false, "target": null, "path": null }
                ]
            },
            {
                "id": "path+file:///repo/packages/framework/server#0.1.0",
                "name": "nve-server",
                "manifest_path": "/repo/packages/framework/server/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "nve-rpc", "kind": null, "optional": false, "target": null, "path": "/repo/packages/framework/rpc" }
                ]
            },
            {
                "id": "path+file:///repo/packages/framework/http-hyper#0.1.0",
                "name": "nve-http-hyper",
                "manifest_path": "/repo/packages/framework/http-hyper/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "nve-server", "kind": null, "optional": false, "target": null, "path": "/repo/packages/framework/server" },
                    { "name": "tokio", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "hyper", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "axum", "kind": "dev", "optional": false, "target": null, "path": null }
                ]
            }
        ],
        "resolve": { "nodes": [] }
    }"#;
    let clean_graph =
        WorkspaceGraph::parse_metadata_json(clean_meta).expect("valid clean synthetic metadata");
    let clean_violations = check_framework_rpc_server_purity(&clean_graph);
    assert!(
        clean_violations.is_empty(),
        "clean framework graph must have 0 violations: {clean_violations:?}"
    );
}
