//! [INV:core-zero-native-cxx]: Rust crates under `packages/core/*` compile zero native C/C++.
//!
//! Native toolchains (React Native Yoga, platform SDK shims) belong in opt-in
//! `packages/modules/<cap>/` crates or host build systems, never in the core layer every target
//! (including `wasm32` and zero-C++ CI) must build. Non-Rust host packages under `packages/core/`
//! (`host-cxx`, `host-swift`, …) are not Cargo crates and keep their language-native tests.

use std::path::Path;

use crate::dag::{rel_manifest, workspace_rel_dir};
use crate::{rel_path, walk_files_no_symlinks, RepoIndex, Violation};

/// Build-script crates that compile or link native C/C++ sources.
pub const NATIVE_BUILD_CRATES: &[&str] = &["cc", "cmake", "cxx-build", "bindgen", "autocxx-build"];

/// Native source extensions banned inside a core Cargo crate directory.
pub const NATIVE_SOURCE_EXTS: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "m", "mm"];

/// Classifies one file inside a core Cargo crate: `Some(reason)` when it brings native C/C++ in.
pub fn native_source_reason(rel: &str, contents: Option<&str>) -> Option<&'static str> {
    let ext = Path::new(rel).extension().and_then(|e| e.to_str()).unwrap_or("");
    if NATIVE_SOURCE_EXTS.contains(&ext) {
        return Some("native C/C++/Objective-C source inside a packages/core/* Rust crate");
    }
    let is_build_script = Path::new(rel).file_name().and_then(|n| n.to_str()) == Some("build.rs");
    if is_build_script && contents.is_some_and(|src| src.contains("cc::Build") || src.contains("cmake::")) {
        return Some("build.rs compiles native C/C++ (`cc::Build` / `cmake::`) inside packages/core/*");
    }
    None
}

pub fn check_core_zero_native_cxx(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in idx.graph.workspace().iter() {
        let Some(dir) = workspace_rel_dir(&pkg) else {
            continue;
        };
        if !dir.starts_with("packages/core/") {
            continue;
        }
        scanned += 1;
        for link in pkg.direct_links() {
            let name = link.to().name();
            if link.build().is_present() && NATIVE_BUILD_CRATES.contains(&name) {
                violations.push(Violation::new(
                    rel_manifest(&pkg),
                    1,
                    format!("[build-dependencies] {name}"),
                    "packages/core/* crates must not build native C/C++; move it into an opt-in packages/modules/<cap>/ crate",
                ));
            }
        }
        let mut files = Vec::new();
        walk_files_no_symlinks(&idx.root.join(&dir), &mut files);
        for path in files {
            let rel = rel_path(&idx.root, &path);
            let contents = rel.ends_with("build.rs").then(|| idx.read_text(&rel)).flatten();
            if let Some(reason) = native_source_reason(&rel, contents.as_deref()) {
                violations.push(Violation::new(&rel, 1, &rel, reason));
            }
        }
    }
    assert!(scanned > 0, "expected Cargo crates under packages/core/");
    violations
}
