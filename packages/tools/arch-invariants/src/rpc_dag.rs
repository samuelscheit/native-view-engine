//! Fullstack framework RPC/server/query layer purity and workspace-wide `fnv` crate ban,
//! powered by `guppy`'s `PackageGraph` and multi-target `CargoSet` resolver.

use guppy::graph::PackageGraph;

use crate::dag::{rel_manifest, transitive_chains, workspace_rel_dir, TARGET_TRIPLES_ALL};
use crate::Violation;

/// Bans:
/// 1. `packages/core/*` -> `packages/framework/*` dependencies (direct or transitive).
/// 2. Runtime/build `serde_json` in fullstack RPC/server/query/ssr/http-*/API crates.
/// 3. Runtime `axum` coupling in `packages/framework/http-*` and the `packages/framework/ssr`
///    orchestrator (dev-only allowed).
/// 4. Runtime/UI/platform crates (`tokio`, `hyper`, `hyper-util`, `axum`, `tower`,
///    `tower-service`, `mio`, `dioxus-core`, `dioxus`, `web-sys`, `wasm-bindgen`) in sans-I/O
///    crates (`rpc`, `server`, `query`, `examples/fullstack-issues/api`).
pub fn check_framework_rpc_server_purity(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned_fw = 0usize;

    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };

        // 1. Core crates must never depend on packages/framework/*.
        if dir.starts_with("packages/core/") {
            for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
                if let Ok(dep) = graph.metadata(dep_id)
                    && workspace_rel_dir(&dep)
                        .is_some_and(|dep_dir| dep_dir.starts_with("packages/framework/"))
                {
                    violations.push(Violation::new(
                        rel_manifest(&pkg),
                        1,
                        chain.join(" -> "),
                        "Core crate under packages/core/* must not depend on packages/framework/*",
                    ));
                }
            }
        }

        // 2. Fullstack RPC/server/query/ssr/http-* and shared example API crates ban runtime serde_json.
        let is_http_adapter = dir.starts_with("packages/framework/http-");
        let is_ssr_orchestrator = dir == "packages/framework/ssr";
        let is_query_ui = dir == "packages/framework/query-dioxus";
        let is_sans_io = matches!(
            dir.as_str(),
            "packages/framework/rpc"
                | "packages/framework/server"
                | "packages/framework/query"
                | "examples/fullstack-issues/api"
        );
        if !is_http_adapter && !is_ssr_orchestrator && !is_query_ui && !is_sans_io {
            continue;
        }
        if dir.starts_with("packages/framework/") {
            scanned_fw += 1;
        }

        let chains = transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL);
        for (dep_id, chain) in &chains {
            let Ok(dep) = graph.metadata(dep_id) else {
                continue;
            };
            let dep_name = dep.name();
            if dep_name == "serde_json" {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    "Runtime/build `serde_json` dependency is banned in fullstack RPC/server/SSR/API crates",
                ));
            }
            // 3. HTTP adapters (`packages/framework/http-*`) and the SSR orchestrator
            //    (`packages/framework/ssr`) must stay free of runtime `axum` coupling (axum is
            //    allowed only as a dev-dependency to verify tower_service / Hyper mountability).
            if (is_http_adapter || is_ssr_orchestrator) && dep_name == "axum" {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    format!(
                        "HTTP/SSR crate `{}` must not take a runtime dependency on `axum`; stay framework-agnostic on `http::Response<HttpBody>` / `tower_service::Service`",
                        pkg.name()
                    ),
                ));
            }
            // 4. Sans-I/O crates (`rpc`, `server`, `query`, shared API) must stay runtime- and
            //    UI-agnostic for native, wasm32-unknown-unknown, and wasm32-wasip2.
            if is_sans_io
                && matches!(
                    dep_name,
                    "tokio"
                        | "hyper"
                        | "hyper-util"
                        | "axum"
                        | "tower"
                        | "tower-service"
                        | "mio"
                        | "dioxus-core"
                        | "dioxus"
                        | "web-sys"
                        | "wasm-bindgen"
                )
            {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    format!(
                        "Sans-I/O crate `{}` must not depend on `{dep_name}`; place platform/runtime adapters in leaf crates",
                        pkg.name()
                    ),
                ));
            }
        }
    }

    assert!(
        scanned_fw >= 3,
        "expected >= 3 fullstack framework crates under packages/framework/ (rpc, server, http-*), found {scanned_fw}"
    );
    violations
}

/// Ensures zero workspace packages take a direct or transitive runtime/build dependency on `fnv`.
pub fn check_zero_fnv_crate_dep(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for pkg in graph.workspace().iter() {
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
            if let Ok(dep) = graph.metadata(dep_id)
                && dep.name() == "fnv"
            {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    "The `fnv` crate is banned workspace-wide; standardize on `xxh64`",
                ));
            }
        }
    }
    violations
}
