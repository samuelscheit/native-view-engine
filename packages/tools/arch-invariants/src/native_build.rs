//! [INV:core-zero-native-cxx]: Rust crates under `packages/core/*` compile zero native C/C++.
//!
//! Native toolchains (React Native Yoga, platform SDK shims) belong in opt-in
//! `packages/modules/<cap>/` crates or host build systems, never in the core layer every target
//! (including `wasm32` and zero-C++ CI) must build. Non-Rust host packages under `packages/core/`
//! (`host-cxx`, `host-swift`, …) are not Cargo crates and keep their language-native tests.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use guppy::graph::{BuildTargetId, PackageGraph};
use guppy::PackageId;

use crate::dag::{rel_manifest, workspace_rel_dir};
use crate::{rel_path, walk_files_no_symlinks, RepoIndex, Violation};

/// Build-script crates that compile or link native C/C++ sources.
pub const NATIVE_BUILD_CRATES: &[&str] = &["cc", "cmake", "cxx-build", "bindgen", "autocxx-build"];

/// Native source extensions (matched case-insensitively) banned inside a core Cargo crate directory.
pub const NATIVE_SOURCE_EXTS: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "m", "mm", "s"];

/// Build-script markers that invoke native C/C++ toolchains.
const NATIVE_BUILD_SCRIPT_MARKERS: &[&str] = &[
    "cc::Build",
    "cmake::",
    "cxx_build::",
    "bindgen::",
    "autocxx_build::",
    "Command::new(\"clang\"",
    "Command::new(\"clang++\"",
    "Command::new(\"gcc\"",
    "Command::new(\"g++\"",
    "Command::new(\"cmake\"",
    "Command::new(\"cc\"",
    "Command::new(\"c++\"",
];

/// Classifies one file inside a core Cargo crate: `Some(reason)` when it brings native C/C++ in.
///
/// Sources under a crate's `tests/fixtures/` are exempt: Cargo never compiles them, they are inputs
/// test harnesses hand to an optional system compiler to check generated headers (any build script
/// that compiled them would itself be flagged).
pub fn native_source_reason(rel: &str, contents: Option<&str>) -> Option<&'static str> {
    if rel.contains("/tests/fixtures/") {
        return None;
    }
    if let Some(ext) = Path::new(rel).extension().and_then(|e| e.to_str())
        && NATIVE_SOURCE_EXTS
            .iter()
            .any(|allowed| ext.eq_ignore_ascii_case(allowed))
    {
        return Some("native C/C++/Objective-C/assembly source inside a packages/core/* Rust crate");
    }
    if let Some(src) = contents
        && NATIVE_BUILD_SCRIPT_MARKERS.iter().any(|m| src.contains(m))
    {
        return Some("build script invokes native C/C++ toolchain inside packages/core/*");
    }
    None
}

/// Checks the Cargo dependency graph for `packages/core/*` crates:
/// - No `packages/core/*` crate may declare a native `links = "..."` key.
/// - No workspace crate reachable from `packages/core/*` over normal or build edges (excluding
///   dev-only edges) may depend on `NATIVE_BUILD_CRATES` (`cc`, `cmake`, `cxx-build`, `bindgen`,
///   `autocxx-build`) or declare a workspace `links` key.
pub fn check_core_zero_native_cxx_graph(graph: &PackageGraph) -> Vec<Violation> {
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

        if let Some(links) = pkg.links() {
            violations.push(Violation::new(
                rel_manifest(&pkg),
                1,
                format!("links = \"{links}\""),
                "packages/core/* crates must not link native C/C++ libraries (`links` key)",
            ));
        }

        // BFS over workspace normal + build edges (skipping dev-only edges) so a helper crate
        // under `packages/tools/*` or `packages/core/*` cannot smuggle `cc`/`cmake`/`bindgen`
        // into `packages/core/*`.
        let mut chains: BTreeMap<&PackageId, Vec<String>> = BTreeMap::new();
        let mut queue: VecDeque<&PackageId> = VecDeque::new();
        let mut seen_snippets: BTreeSet<String> = BTreeSet::new();
        chains.insert(pkg.id(), vec![pkg.name().to_string()]);
        queue.push_back(pkg.id());

        while let Some(curr_id) = queue.pop_front() {
            let Some(curr_chain) = chains.get(curr_id).cloned() else {
                continue;
            };
            let Ok(curr_pkg) = graph.metadata(curr_id) else {
                continue;
            };
            for link in curr_pkg.direct_links() {
                if link.dev_only() {
                    continue;
                }
                let next = link.to();
                let next_name = next.name();
                if NATIVE_BUILD_CRATES.contains(&next_name) {
                    let mut chain = curr_chain.clone();
                    chain.push(next_name.to_string());
                    let snippet = chain.join(" -> ");
                    if seen_snippets.insert(snippet.clone()) {
                        violations.push(Violation::new(
                            rel_manifest(&pkg),
                            1,
                            snippet,
                            "packages/core/* crates (and their workspace normal/build deps) must not build native C/C++; move it into an opt-in packages/modules/<cap>/ crate",
                        ));
                    }
                    continue;
                }
                if next.in_workspace() {
                    if let Some(links) = next.links() {
                        let mut chain = curr_chain.clone();
                        chain.push(format!("{next_name} (links={links})"));
                        let snippet = chain.join(" -> ");
                        if seen_snippets.insert(snippet.clone()) {
                            violations.push(Violation::new(
                                rel_manifest(&pkg),
                                1,
                                snippet,
                                "packages/core/* crates must not transitively depend on workspace crates declaring `links`",
                            ));
                        }
                    }
                    if !chains.contains_key(next.id()) {
                        let mut next_chain = curr_chain.clone();
                        next_chain.push(next_name.to_string());
                        chains.insert(next.id(), next_chain);
                        queue.push_back(next.id());
                    }
                } else if let Some(links) = next.links() {
                    let mut chain = curr_chain.clone();
                    chain.push(format!("{next_name} (links={links})"));
                    let snippet = chain.join(" -> ");
                    if seen_snippets.insert(snippet.clone()) {
                        violations.push(Violation::new(
                            rel_manifest(&pkg),
                            1,
                            snippet,
                            "packages/core/* crates (and their workspace normal/build deps) must not directly depend on native `*-sys` (`links`) crates; move them into an opt-in packages/modules/<cap>/ crate",
                        ));
                    }
                }
            }
        }
    }

    assert!(scanned > 0, "expected Cargo crates under packages/core/");
    violations
}

pub fn check_core_zero_native_cxx(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = check_core_zero_native_cxx_graph(&idx.graph);

    for pkg in idx.graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if !dir.starts_with("packages/core/") {
            continue;
        }
        // Resolve declared Cargo build scripts via `BuildTargetId::BuildScript` so custom
        // `[package] build = "scripts/gen.rs"` paths are scanned too.
        let mut build_scripts: BTreeSet<String> = BTreeSet::new();
        for target in pkg.build_targets() {
            if matches!(target.id(), BuildTargetId::BuildScript) {
                let rel = rel_path(&idx.root, target.path().as_std_path());
                build_scripts.insert(rel);
            }
        }

        let mut files = Vec::new();
        walk_files_no_symlinks(&idx.root.join(&dir), &mut files);
        let mut visited: BTreeSet<String> = BTreeSet::new();
        for path in files {
            let rel = rel_path(&idx.root, &path);
            visited.insert(rel.clone());
            let is_build = build_scripts.contains(&rel)
                || Path::new(&rel).file_name().and_then(|n| n.to_str()) == Some("build.rs");
            let contents = is_build.then(|| idx.read_text(&rel)).flatten();
            if let Some(reason) = native_source_reason(&rel, contents.as_deref()) {
                violations.push(Violation::new(&rel, 1, &rel, reason));
            }
        }
        // Also inspect custom `[package] build = "../..."` scripts living outside the crate dir.
        for build_rel in &build_scripts {
            if visited.contains(build_rel) {
                continue;
            }
            let contents = idx.read_text(build_rel);
            if let Some(reason) = native_source_reason(build_rel, contents.as_deref()) {
                violations.push(Violation::new(build_rel, 1, build_rel, reason));
            }
        }
    }
    violations
}
