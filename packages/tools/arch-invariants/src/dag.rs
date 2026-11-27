//! Workspace dependency DAG and module-boundary checks powered by `guppy` + `target-spec`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use guppy::graph::{BuildTargetId, PackageGraph, PackageMetadata};
use guppy::platform::{EnabledTernary, Platform, PlatformSpec, TargetFeatures};
use guppy::PackageId;
use syn::visit::Visit;

use crate::{is_cfg_test_attr, line_at, RepoIndex, Violation};

pub const BANNED_PLATFORM_CRATES: &[&str] = &["web-sys", "js-sys", "wasm-bindgen", "skia-safe"];

pub const APPROVED_HOST_CARGO_TOMLS: &[&str] = &[
    "hosts/skia-shell/Cargo.toml",
    "hosts/web-shell/Cargo.toml",
];

/// Pre-existing runtime `serde_json` allowlist (Track 1B-1e).
const PENDING_CLEANUP_SERDE_JSON: &[(&str, &str)] = &[("nve-codegen", "Track 1B-1e")];

fn workspace_rel_dir(pkg: &PackageMetadata<'_>) -> Option<String> {
    pkg.source()
        .workspace_path()
        .map(|p| p.as_str().replace('\\', "/"))
}

fn rel_manifest(pkg: &PackageMetadata<'_>) -> String {
    match workspace_rel_dir(pkg) {
        Some(dir) if !dir.is_empty() => format!("{dir}/Cargo.toml"),
        _ => "Cargo.toml".to_string(),
    }
}

/// Shortest transitive dependency chains from `root_id`.
///
/// For workspace crates, follows ALL normal & build edges (including optional deps, plus root dev
/// edges when `include_dev` is true). For third-party crates, evaluates real `target-spec`
/// `PlatformSpec`s for both native host and `wasm32-unknown-unknown`, following only non-optional
/// normal runtime dependencies enabled on real targets (automatically ignoring `cfg(hax)`,
/// third-party build-script deps like `skia-bindings -> serde_json`, and optional wasm-only deps
/// like `getrandom -> js-sys / wasm-bindgen`).
fn transitive_chains<'g>(
    graph: &'g PackageGraph,
    root_id: &'g PackageId,
    include_dev: bool,
) -> BTreeMap<&'g PackageId, Vec<String>> {
    let host = PlatformSpec::from(
        Platform::new("x86_64-unknown-linux-gnu", TargetFeatures::Unknown)
            .expect("valid host target triple"),
    );
    let mut visited: BTreeMap<&'g PackageId, Vec<String>> = BTreeMap::new();
    let Ok(root_pkg) = graph.metadata(root_id) else {
        return visited;
    };
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
        let curr_is_ws = curr_pkg.in_workspace();
        for link in curr_pkg.direct_links() {
            let is_active = if curr_is_ws {
                (include_dev && curr_id == root_id) || !link.dev_only()
            } else {
                // Third-party crate: follow non-optional normal runtime deps enabled on host
                // via real `target-spec` evaluation (skips `cfg(hax)`, third-party build-script
                // deps like `skia-bindings -> serde_json`, and wasm-only shims like
                // `getrandom` / `web-time` -> `js-sys` / `wasm-bindgen`).
                link.normal().status().required_on(&host) != EnabledTernary::Disabled
            };
            if !is_active {
                continue;
            }
            let next = link.to();
            if !visited.contains_key(next.id()) {
                let mut next_chain = curr_chain.clone();
                next_chain.push(next.name().to_string());
                visited.insert(next.id(), next_chain);
                queue.push_back(next.id());
            }
        }
    }
    visited.remove(root_id);
    visited
}

pub fn check_packages_to_hosts_deps(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();

    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if dir == "hosts/skia-shell"
            && pkg
                .build_targets()
                .any(|t| matches!(t.id(), BuildTargetId::Library))
        {
            violations.push(Violation::new(
                rel_manifest(&pkg),
                1,
                "[lib]",
                "`hosts/skia-shell` must be binary-only (no `[lib]` target); spawn `nve-skia-shell` as a subprocess",
            ));
        }
        if !dir.starts_with("packages/") {
            continue;
        }
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), true) {
            let Ok(dep) = graph.metadata(dep_id) else {
                continue;
            };
            let Some(dep_dir) = workspace_rel_dir(&dep) else {
                continue;
            };
            if dep_dir.starts_with("hosts/") {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    format!(
                        "Crate `{}` under packages/ must not depend on host shell `{}` ({dep_dir})",
                        pkg.name(),
                        dep.name()
                    ),
                ));
            }
        }
    }
    violations
}

pub fn check_core_zero_dioxus(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if !dir.starts_with("packages/core/") || dir == "packages/core/dioxus" {
            continue;
        }
        scanned += 1;
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false) {
            if let Ok(dep) = graph.metadata(dep_id)
                && dep.name().starts_with("dioxus")
            {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    "Core crates outside `packages/core/dioxus` must be framework-agnostic with zero direct or transitive `dioxus*` dependency",
                ));
            }
        }
    }
    assert!(scanned > 0, "expected core crates under packages/core/");
    violations
}

pub fn check_core_zero_platform_crates(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if !dir.starts_with("packages/core/") {
            continue;
        }
        scanned += 1;
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false) {
            if let Ok(dep) = graph.metadata(dep_id)
                && BANNED_PLATFORM_CRATES.contains(&dep.name())
            {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    "packages/core/* must not depend on platform crates (`web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`)",
                ));
            }
        }
    }
    assert!(scanned > 0, "expected core crates under packages/core/");
    violations
}

pub fn check_zero_serde_json(graph: &PackageGraph) -> Vec<Violation> {
    let pending: BTreeSet<&str> = PENDING_CLEANUP_SERDE_JSON.iter().map(|(p, _)| *p).collect();
    let mut seen_pending = BTreeSet::new();
    let mut violations = Vec::new();

    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        let in_scope = dir.starts_with("packages/core/")
            || dir.starts_with("packages/backends/")
            || dir.starts_with("packages/modules/");
        if !in_scope {
            continue;
        }
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false) {
            if let Ok(dep) = graph.metadata(dep_id)
                && dep.name() == "serde_json"
            {
                if pending.contains(pkg.name()) {
                    seen_pending.insert(pkg.name().to_string());
                } else {
                    violations.push(Violation::new(
                        rel_manifest(&pkg),
                        1,
                        chain.join(" -> "),
                        "Runtime/build `serde_json` dependency is banned in packages/{core,backends,modules}/*",
                    ));
                }
            }
        }
    }

    for &(pkg_name, track) in PENDING_CLEANUP_SERDE_JSON {
        if !seen_pending.contains(pkg_name) {
            violations.push(Violation::new(
                "packages/tools/arch-invariants/src/dag.rs",
                0,
                pkg_name,
                format!(
                    "`PENDING_CLEANUP_SERDE_JSON` entry `{pkg_name}` ({track}) no longer depends on `serde_json` — remove it"
                ),
            ));
        }
    }
    violations
}

pub fn check_core_backend_module_deps(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if !dir.starts_with("packages/core/") && !dir.starts_with("packages/backends/") {
            continue;
        }
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false) {
            if let Ok(dep) = graph.metadata(dep_id)
                && workspace_rel_dir(&dep).is_some_and(|d| d.starts_with("packages/modules/"))
            {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    chain.join(" -> "),
                    "Core crates and backends must have zero dependencies on optional packages/modules/*",
                ));
            }
        }
    }
    violations
}

pub fn check_module_root_spec_and_wasm32_dev_deps(graph: &PackageGraph) -> Vec<Violation> {
    let wasm32 = PlatformSpec::from(
        Platform::new("wasm32-unknown-unknown", TargetFeatures::Unknown)
            .expect("valid wasm32-unknown-unknown triple"),
    );
    let mut violations = Vec::new();

    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if dir.starts_with("packages/modules/") {
            let is_root_spec = dir.matches('/').count() == 2;
            if is_root_spec {
                for (dep_id, chain) in transitive_chains(graph, pkg.id(), false) {
                    if let Ok(dep) = graph.metadata(dep_id)
                        && (dep.name().starts_with("nve-backend-")
                            || BANNED_PLATFORM_CRATES.contains(&dep.name()))
                    {
                        violations.push(Violation::new(
                            rel_manifest(&pkg),
                            1,
                            chain.join(" -> "),
                            "Root spec crate packages/modules/<cap>/ must not depend on backends or platform crates",
                        ));
                    }
                }
            }
            let feats: BTreeSet<&str> = pkg.named_features().collect();
            if feats.contains("web") && feats.contains("skia") {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    "features: both `web` and `skia` declared",
                    "Module crate must not mix `web` and `skia` features; split into leaf `{web,skia}/` sub-crates",
                ));
            }
        }

        let in_wasm_layer = dir.starts_with("packages/core/")
            || dir.starts_with("packages/backends/")
            || dir.starts_with("packages/framework/")
            || dir.starts_with("packages/modules/");
        if in_wasm_layer {
            for link in pkg.direct_links() {
                let dep_name = link.to().name();
                if matches!(dep_name, "skia-safe" | "nve-backend-skia" | "criterion")
                    && link.dev().status().enabled_on(&wasm32) != EnabledTernary::Disabled
                {
                    violations.push(Violation::new(
                        rel_manifest(&pkg),
                        1,
                        format!("[dev-dependencies] {dep_name}"),
                        format!(
                            "`{dep_name}` dev-dependency is enabled on `wasm32-unknown-unknown`; gate under `[target.'cfg(not(target_arch = \"wasm32\"))'.dev-dependencies]`"
                        ),
                    ));
                }
            }
        }
    }
    violations
}

/// Uses `syn` AST inspection to verify `impl WebBackend { pub fn new(...) }` never auto-registers
/// optional module symbols.
pub fn scan_web_backend_new_auto_reg(idx: &RepoIndex) -> Vec<Violation> {
    let web_files = idx.rs_files_under("packages/backends/web/src");
    assert!(!web_files.is_empty(), "expected Rust sources under packages/backends/web/src");

    let mut found_new = false;
    let mut violations = Vec::new();
    for rel in web_files {
        let Some(parsed) = idx.parse_rust_if_contains(&rel, "WebBackend") else {
            continue;
        };
        if scan_web_backend_ast(&rel, &parsed.raw, &parsed.ast, &mut violations) {
            found_new = true;
        }
    }
    assert!(
        found_new,
        "Could not locate `impl WebBackend {{ pub fn new(...) }}` under packages/backends/web/src"
    );
    violations
}

pub fn scan_web_backend_ast(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) -> bool {
    struct IdentScan<'a> {
        rel: &'a str,
        raw: &'a str,
        out: &'a mut Vec<Violation>,
    }
    const BANNED: &[&str] = &[
        "register_module",
        "register_extension",
        "nve_camera",
        "nve_motion",
        "CameraPreview",
        "MotionDriver",
    ];
    impl<'ast> Visit<'ast> for IdentScan<'_> {
        fn visit_ident(&mut self, i: &'ast syn::Ident) {
            let s = i.to_string();
            if BANNED.contains(&s.as_str()) {
                let ln = i.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    format!("WebBackend::new() must not auto-register optional module symbol `{s}`"),
                ));
            }
        }
    }

    let mut found = false;
    for item in &ast.items {
        if let syn::Item::Impl(imp) = item
            && imp.trait_.is_none()
            && !is_cfg_test_attr(&imp.attrs)
            && matches!(&*imp.self_ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "WebBackend"))
        {
            for ii in &imp.items {
                if let syn::ImplItem::Fn(func) = ii
                    && func.sig.ident == "new"
                    && !is_cfg_test_attr(&func.attrs)
                {
                    found = true;
                    IdentScan { rel, raw, out }.visit_block(&func.block);
                }
            }
        }
    }
    found
}
