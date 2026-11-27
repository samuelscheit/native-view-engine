//! Workspace dependency DAG and module-boundary checks powered by `guppy` `FeatureGraph` /
//! `CargoSet` + `target-spec` across Linux, macOS, Windows, and `wasm32-unknown-unknown`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use guppy::graph::cargo::{CargoOptions, CargoSet, InitialsPlatform};
use guppy::graph::feature::StandardFeatures;
use guppy::graph::{BuildTargetId, DependencyDirection, PackageGraph, PackageMetadata};
use guppy::platform::{EnabledTernary, Platform, PlatformSpec, TargetFeatures};
use guppy::PackageId;
use syn::visit::{self, Visit};

use crate::{is_cfg_test_attr, line_at, RepoIndex, Violation};

pub const BANNED_PLATFORM_CRATES: &[&str] = &["web-sys", "js-sys", "wasm-bindgen", "skia-safe"];

pub const APPROVED_HOST_CARGO_TOMLS: &[&str] = &[
    "hosts/skia-shell/Cargo.toml",
    "hosts/web-shell/Cargo.toml",
];

/// Pre-existing runtime `serde_json` allowlist (empty: `nve-codegen` reads no JSON since Track 1B-1e).
const PENDING_CLEANUP_SERDE_JSON: &[(&str, &str)] = &[];

pub(crate) const TARGET_TRIPLES_ALL: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "wasm32-unknown-unknown",
];

const TARGET_TRIPLES_NATIVE: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

fn platform_spec(triple: &'static str) -> PlatformSpec {
    PlatformSpec::from(
        Platform::new(triple, TargetFeatures::Unknown).expect("valid target triple"),
    )
}

pub(crate) fn workspace_rel_dir(pkg: &PackageMetadata<'_>) -> Option<String> {
    pkg.source()
        .workspace_path()
        .map(|p| p.as_str().replace('\\', "/"))
}

pub(crate) fn rel_manifest(pkg: &PackageMetadata<'_>) -> String {
    match workspace_rel_dir(pkg) {
        Some(dir) if !dir.is_empty() => format!("{dir}/Cargo.toml"),
        _ => "Cargo.toml".to_string(),
    }
}

/// Shortest transitive dependency chains reachable from `root_id` using `guppy`'s `CargoSet`
/// feature-graph resolver (`InitialsPlatform::ProcMacrosOnTarget` + `StandardFeatures::All`)
/// across every platform in `triples`, PLUS iterative expansion of workspace `[build-dependencies]`
/// and workspace proc-macro dependencies across native targets (still excluding third-party
/// build-script dependencies like `skia-bindings -> serde_json`).
pub(crate) fn transitive_chains<'g>(
    graph: &'g PackageGraph,
    root_id: &'g PackageId,
    include_dev: bool,
    triples: &[&'static str],
) -> BTreeMap<&'g PackageId, Vec<String>> {
    let fg = graph.feature_graph();
    let Ok(root_pkg) = graph.metadata(root_id) else {
        return BTreeMap::new();
    };
    let Ok(root_set) = graph.resolve_ids([root_id]) else {
        return BTreeMap::new();
    };
    let initials = root_set.to_feature_set(StandardFeatures::All);
    let features_only = fg.resolve_none();
    let host_linux = platform_spec("x86_64-unknown-linux-gnu");

    let mut active_ids: BTreeSet<&'g PackageId> = BTreeSet::new();
    active_ids.insert(root_pkg.id());

    for &triple in triples {
        let target = platform_spec(triple);
        let mut opts = CargoOptions::new();
        opts.set_include_dev(include_dev)
            .set_initials_platform(InitialsPlatform::ProcMacrosOnTarget)
            .set_target_platform(target)
            .set_host_platform(host_linux.clone());
        if let Ok(cargo_set) = CargoSet::new(initials.clone(), features_only.clone(), &opts) {
            for pid in cargo_set
                .target_features()
                .to_package_set()
                .package_ids(DependencyDirection::Forward)
            {
                active_ids.insert(pid);
            }
        }
    }

    // Iteratively expand workspace `[build-dependencies]` and workspace edges targeting
    // `proc-macro = true` crates across native platforms (while excluding third-party build
    // scripts like `skia-bindings -> serde_json`).
    let mut expanded_ws_host_roots: BTreeSet<&'g PackageId> = BTreeSet::new();
    loop {
        let mut next_roots: Vec<&'g PackageId> = Vec::new();
        for &pid in &active_ids {
            let Ok(pkg) = graph.metadata(pid) else {
                continue;
            };
            if !pkg.in_workspace() {
                continue;
            }
            for link in pkg.direct_links() {
                let is_ws_build = link.build().is_present();
                let is_ws_proc_macro = link.to().is_proc_macro()
                    && (if pid == root_id {
                        include_dev || !link.dev_only()
                    } else {
                        !link.dev_only()
                    });
                if (is_ws_build || is_ws_proc_macro)
                    && expanded_ws_host_roots.insert(link.to().id())
                {
                    next_roots.push(link.to().id());
                }
            }
        }
        if next_roots.is_empty() {
            break;
        }
        let Ok(bset) = graph.resolve_ids(next_roots) else {
            break;
        };
        let b_initials = bset.to_feature_set(StandardFeatures::All);
        for &triple in TARGET_TRIPLES_NATIVE {
            let target = platform_spec(triple);
            let mut opts = CargoOptions::new();
            opts.set_include_dev(false)
                .set_initials_platform(InitialsPlatform::ProcMacrosOnTarget)
                .set_target_platform(target)
                .set_host_platform(host_linux.clone());
            if let Ok(cargo_set) = CargoSet::new(b_initials.clone(), features_only.clone(), &opts) {
                for pid in cargo_set
                    .target_features()
                    .to_package_set()
                    .package_ids(DependencyDirection::Forward)
                {
                    active_ids.insert(pid);
                }
            }
        }
    }

    // Reconstruct shortest readable dependency chains restricted to `active_ids`.
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
            let allow_edge = if curr_id == root_id {
                include_dev || !link.dev_only()
            } else if curr_pkg.in_workspace() {
                !link.dev_only()
            } else {
                link.normal().is_present()
            };
            if !allow_edge {
                continue;
            }
            let next = link.to();
            if active_ids.contains(next.id()) && !visited.contains_key(next.id()) {
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
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), true, TARGET_TRIPLES_ALL) {
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

/// Enforces zero direct `memmap2` dependencies across the entire workspace and zero transitive
/// non-dev `memmap2` dependencies in framework-agnostic `packages/core/*` (excluding `dioxus`,
/// which transitively reaches `subsecond -> memmap2`) and `packages/backends/{headless,web}`.
pub fn check_no_memmap2_dep(graph: &PackageGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        scanned += 1;
        for link in pkg.direct_links() {
            if link.to().name() == "memmap2" {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    format!("{} -> memmap2", pkg.name()),
                    "Direct `memmap2` dependency is permanently banned workspace-wide ([INV:no-reloc-mmap-snapshot]); use plain file I/O (`Frame0File` + `StateFile`)",
                ));
            }
        }
        let ban_transitive = (dir.starts_with("packages/core/") && dir != "packages/core/dioxus")
            || dir == "packages/backends/headless"
            || dir == "packages/backends/web";
        if ban_transitive {
            for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
                if let Ok(dep) = graph.metadata(dep_id)
                    && dep.name() == "memmap2"
                {
                    violations.push(Violation::new(
                        rel_manifest(&pkg),
                        1,
                        chain.join(" -> "),
                        "Core (non-dioxus), headless, and web crates must have zero transitive `memmap2` dependency ([INV:no-reloc-mmap-snapshot])",
                    ));
                }
            }
        }
    }
    assert!(scanned > 0, "expected workspace crates for memmap2 check");
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
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
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
    let wasm32 = platform_spec("wasm32-unknown-unknown");
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
        // 1. Any banned platform crate reachable on Linux/macOS/Windows (even via third-party) fails.
        let native_chains = transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_NATIVE);
        let mut seen_snippets: BTreeSet<String> = BTreeSet::new();
        for (dep_id, chain) in &native_chains {
            if let Ok(dep) = graph.metadata(dep_id)
                && BANNED_PLATFORM_CRATES.contains(&dep.name())
            {
                let snippet = chain.join(" -> ");
                if seen_snippets.insert(snippet.clone()) {
                    violations.push(Violation::new(
                        rel_manifest(&pkg),
                        1,
                        snippet,
                        "packages/core/* must not depend on platform crates (`web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`)",
                    ));
                }
            }
        }
        // 2. Across all targets (including `wasm32`), any workspace crate reachable from `pkg`
        // (including `pkg` itself) that directly links a banned platform crate fails, catching
        // `core -> workspace -> web-sys` behind `cfg(wasm32)` while skipping third-party wasm shims.
        let all_chains = transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL);
        let ws_reachable = std::iter::once((pkg.id(), vec![pkg.name().to_string()])).chain(
            all_chains
                .into_iter()
                .filter(|(id, _)| graph.metadata(id).is_ok_and(|m| m.in_workspace())),
        );
        for (ws_id, prefix_chain) in ws_reachable {
            let Ok(ws_pkg) = graph.metadata(ws_id) else {
                continue;
            };
            for link in ws_pkg.direct_links() {
                if link.dev_only() || !BANNED_PLATFORM_CRATES.contains(&link.to().name()) {
                    continue;
                }
                let enabled_wasm = link.normal().status().enabled_on(&wasm32) != EnabledTernary::Disabled
                    || link.build().status().enabled_on(&wasm32) != EnabledTernary::Disabled;
                if enabled_wasm {
                    let mut chain = prefix_chain.clone();
                    chain.push(link.to().name().to_string());
                    let snippet = chain.join(" -> ");
                    if seen_snippets.insert(snippet.clone()) {
                        violations.push(Violation::new(
                            rel_manifest(&pkg),
                            1,
                            snippet,
                            "packages/core/* must not depend on platform crates (`web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`)",
                        ));
                    }
                }
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
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
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
        let is_core = dir.starts_with("packages/core/");
        if !is_core && !dir.starts_with("packages/backends/") {
            continue;
        }
        for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL) {
            let Some(dep_dir) = graph.metadata(dep_id).ok().and_then(|d| workspace_rel_dir(&d))
            else {
                continue;
            };
            let reason = if dep_dir.starts_with("packages/modules/") {
                "Core crates and backends must have zero dependencies on optional packages/modules/*"
            } else if is_core && dep_dir.starts_with("packages/backends/") {
                "Core crates (packages/core/*) must have zero dependencies on backends (packages/backends/*)"
            } else {
                continue;
            };
            violations.push(Violation::new(rel_manifest(&pkg), 1, chain.join(" -> "), reason));
        }
    }
    violations
}

pub fn check_module_root_spec_and_wasm32_dev_deps(graph: &PackageGraph) -> Vec<Violation> {
    let wasm32 = platform_spec("wasm32-unknown-unknown");
    let mut violations = Vec::new();

    for pkg in graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if dir.starts_with("packages/modules/") {
            let is_root_spec = dir.matches('/').count() == 2;
            if is_root_spec {
                for (dep_id, chain) in transitive_chains(graph, pkg.id(), false, TARGET_TRIPLES_ALL)
                {
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
/// optional module symbols or feature-gates `camera` / `motion` module wiring.
pub fn scan_web_backend_new_auto_reg(idx: &RepoIndex) -> Vec<Violation> {
    let web_files = idx.rs_files_under("packages/backends/web/src");
    assert!(
        !web_files.is_empty(),
        "expected Rust sources under packages/backends/web/src"
    );

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

fn cfg_meta_has_banned_module_feature(meta: &syn::Meta) -> Option<&'static str> {
    match meta {
        syn::Meta::NameValue(nv) if nv.path.is_ident("feature") => {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                match s.value().as_str() {
                    "camera" => return Some("camera"),
                    "motion" => return Some("motion"),
                    _ => {}
                }
            }
            None
        }
        syn::Meta::List(list) => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .ok()
            .and_then(|items| items.iter().find_map(cfg_meta_has_banned_module_feature)),
        _ => None,
    }
}

pub fn scan_web_backend_ast(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) -> bool {
    if is_cfg_test_attr(&ast.attrs) {
        return false;
    }

    struct NewBodyScan<'a> {
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
    impl<'ast> Visit<'ast> for NewBodyScan<'_> {
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

        fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
            if attr.path().is_ident("cfg")
                && let Ok(meta) = attr.parse_args::<syn::Meta>()
                && let Some(feat) = cfg_meta_has_banned_module_feature(&meta)
            {
                let ln = attr.pound_token.span.start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    format!(
                        "WebBackend::new() must not feature-gate optional module auto-registration (`feature = \"{feat}\"`)"
                    ),
                ));
            }
            visit::visit_attribute(self, attr);
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
                    let mut scanner = NewBodyScan { rel, raw, out };
                    for attr in &func.attrs {
                        scanner.visit_attribute(attr);
                    }
                    scanner.visit_block(&func.block);
                }
            }
        }
    }
    found
}
