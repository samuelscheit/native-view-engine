//! Fullstack framework RPC/server/query layer purity and workspace-wide `fnv` crate ban,
//! powered by `guppy`'s `PackageGraph` and multi-target `CargoSet` resolver.

use std::collections::{BTreeMap, VecDeque};

use guppy::graph::PackageGraph;
use guppy::PackageId;

use crate::dag::{rel_manifest, transitive_chains, workspace_rel_dir};
use crate::Violation;

const RPC_TARGET_TRIPLES: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "wasm32-unknown-unknown",
    "wasm32-wasip2",
    "aarch64-linux-android",
    "aarch64-apple-ios",
];

/// Combines multi-target `CargoSet` resolution (`RPC_TARGET_TRIPLES`, including `wasm32-wasip2`,
/// Android, and iOS) with a full BFS over all declared non-dev (`normal` + `build`) links
/// regardless of `cfg(...)` target predicate so target-gated dependencies (e.g.
/// `[target.'cfg(target_os = "wasi")'.dependencies]`) and transitive build-script edges cannot
/// bypass purity checks.
fn non_dev_chains_all_cfgs<'g>(
    graph: &'g PackageGraph,
    root_id: &'g PackageId,
) -> BTreeMap<&'g PackageId, Vec<String>> {
    let mut chains = transitive_chains(graph, root_id, false, RPC_TARGET_TRIPLES);
    let Ok(root_pkg) = graph.metadata(root_id) else {
        return chains;
    };
    let mut visited: BTreeMap<&'g PackageId, Vec<String>> = BTreeMap::new();
    let mut queue: VecDeque<&'g PackageId> = VecDeque::new();
    visited.insert(root_pkg.id(), vec![root_pkg.name().to_string()]);
    queue.push_back(root_pkg.id());

    while let Some(curr_id) = queue.pop_front() {
        let Some(curr_chain) = visited.get(curr_id).cloned() else {
            continue;
        };
        let Ok(curr_pkg) = graph.metadata(curr_id) else {
            continue;
        };
        for link in curr_pkg.direct_links() {
            if link.dev_only() {
                continue;
            }
            let next_id = link.to().id();
            if visited.contains_key(next_id) {
                continue;
            }
            let mut next_chain = curr_chain.clone();
            next_chain.push(link.to().name().to_string());
            visited.insert(next_id, next_chain);
            queue.push_back(next_id);
        }
    }
    visited.remove(root_pkg.id());
    for (dep_id, chain) in visited {
        chains.entry(dep_id).or_insert(chain);
    }
    chains
}

/// Bans:
/// 1. `packages/core/*` -> `packages/framework/*` dependencies (direct or transitive).
/// 2. Runtime/build `serde_json` in fullstack RPC/server/query/http-*/API crates.
/// 3. Runtime `axum` coupling in `packages/framework/http-*` (dev-only allowed).
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
            for (dep_id, chain) in non_dev_chains_all_cfgs(graph, pkg.id()) {
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

        // 2. Fullstack RPC/server/query/http-* and shared example API crates ban runtime serde_json.
        let is_http_adapter = dir.starts_with("packages/framework/http-");
        let is_query_ui = dir == "packages/framework/query-dioxus";
        let is_sans_io = matches!(
            dir.as_str(),
            "packages/framework/rpc"
                | "packages/framework/server"
                | "packages/framework/query"
                | "examples/fullstack-issues/api"
        );
        if !is_http_adapter && !is_query_ui && !is_sans_io {
            continue;
        }
        if dir.starts_with("packages/framework/") {
            scanned_fw += 1;
        }

        let chains = non_dev_chains_all_cfgs(graph, pkg.id());
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
                    "Runtime/build `serde_json` dependency is banned in fullstack RPC/server/API crates",
                ));
            }
            // 3. HTTP adapters (`packages/framework/http-*`) must stay free of runtime `axum` coupling
            //    (axum is allowed only as a dev-dependency to verify tower_service mountability).
            if is_http_adapter && dep_name == "axum" {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    format!(
                        "HTTP adapter `{}` must not take a runtime dependency on `axum`; expose `tower_service::Service` instead",
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
        for (dep_id, chain) in non_dev_chains_all_cfgs(graph, pkg.id()) {
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
