//! Crate DAG, packaging, and module boundary invariants powered by `WorkspaceGraph`
//! (`cargo metadata --format-version 1 --offline --locked`).

use std::collections::BTreeSet;

use crate::json_meta::{DepKind, WorkspaceGraph};
use crate::support::{
    assert_no_violations, assert_staged_violations, contains_ident, find_files_named, rel_path,
    repo_index, strip_rust_comments_and_strings, Violation,
};
use crate::Stage;

/// Shared platform-specific crates forbidden in core crates and module root specs.
pub const BANNED_PLATFORM_CRATES: &[&str] = &["web-sys", "js-sys", "wasm-bindgen", "skia-safe"];

/// Approved Rust shell entry-point crates under `hosts/`.
const APPROVED_HOST_CARGO_TOMLS: &[&str] = &[
    "hosts/skia-shell/Cargo.toml",
    "hosts/web-shell/Cargo.toml",
];

/// Exact-pair ratchet for pre-existing `packages/** -> hosts/**` dependencies.
/// Fails if ANY new pair is added OR if `nve-cli -> nve-skia-shell` is removed without updating here.
const PENDING_CLEANUP_HOST_DEPS: &[(&str, &str)] = &[("nve-cli", "nve-skia-shell")];

/// Minimal tracked `serde_json` runtime dependency cleanup list (Track 1B-1e).
const PENDING_CLEANUP_SERDE_JSON: &[(&str, &str)] = &[("nve-codegen", "Track 1B-1e")];

#[test]
fn no_sdk_library_crates_in_hosts() {
    let idx = repo_index();
    let host_manifests = find_files_named(&idx.root.join("hosts"), "Cargo.toml");
    let approved: BTreeSet<&str> = APPROVED_HOST_CARGO_TOMLS.iter().copied().collect();
    let mut violations = Vec::new();

    for manifest in &host_manifests {
        let rel = rel_path(&idx.root, manifest);
        if !approved.contains(rel.as_str()) {
            violations.push(Violation::new(
                &rel,
                1,
                &rel,
                "hosts/ must not contain SDK library crates outside approved host shell entry points",
            ));
        }
    }

    violations.extend(check_packages_to_hosts_deps(&idx.graph));
    assert_no_violations("No SDK library crates in hosts/ & no packages/** -> hosts/**", &violations);
}

pub fn check_packages_to_hosts_deps(graph: &WorkspaceGraph) -> Vec<Violation> {
    let pending: BTreeSet<(&str, &str)> = PENDING_CLEANUP_HOST_DEPS.iter().copied().collect();
    let mut seen_pending: BTreeSet<(String, String)> = BTreeSet::new();
    let mut violations = Vec::new();

    for pkg in graph.workspace_packages() {
        if !pkg.rel_dir.starts_with("packages/") {
            continue;
        }
        for (dep_pkg, chain) in graph.transitive_workspace_deps(&pkg.id, true) {
            if dep_pkg.rel_dir.starts_with("hosts/") {
                let pair = (pkg.name.as_str(), dep_pkg.name.as_str());
                if pending.contains(&pair) {
                    seen_pending.insert((pkg.name.clone(), dep_pkg.name.clone()));
                } else {
                    violations.push(Violation::new(
                        &pkg.rel_manifest,
                        1,
                        chain.join(" -> "),
                        format!(
                            "Crate `{}` under packages/ must not depend on host shell `{}` ({})",
                            pkg.name, dep_pkg.name, dep_pkg.rel_dir
                        ),
                    ));
                }
            }
        }
    }

    for &(src, dst) in PENDING_CLEANUP_HOST_DEPS {
        if !seen_pending.contains(&(src.to_string(), dst.to_string())) {
            violations.push(Violation::new(
                "packages/tools/cli/tests/arch_invariants/crate_dag.rs",
                0,
                format!("{src} -> {dst}"),
                "Stale `PENDING_CLEANUP_HOST_DEPS` entry — remove it to ratchet down",
            ));
        }
    }
    violations
}

#[test]
fn core_crates_zero_dioxus_dependency() {
    let idx = repo_index();
    let violations = check_core_zero_dioxus(&idx.graph);
    assert_no_violations("Zero dioxus* in packages/core/* (excluding packages/core/dioxus)", &violations);
}

pub fn check_core_zero_dioxus(graph: &WorkspaceGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in graph.workspace_packages() {
        if !pkg.rel_dir.starts_with("packages/core/") || pkg.rel_dir == "packages/core/dioxus" {
            continue;
        }
        scanned += 1;
        for (dep, chain) in graph.transitive_non_dev_crates(&pkg.id) {
            if dep.starts_with("dioxus") {
                violations.push(Violation::new(
                    &pkg.rel_manifest,
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

#[test]
fn core_crates_zero_platform_backend_dependencies() {
    let idx = repo_index();
    let violations = check_core_zero_platform_crates(&idx.graph);
    assert_no_violations(
        "Zero web-sys/js-sys/wasm-bindgen/skia-safe in packages/core/*",
        &violations,
    );
}

pub fn check_core_zero_platform_crates(graph: &WorkspaceGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for pkg in graph.workspace_packages() {
        if !pkg.rel_dir.starts_with("packages/core/") {
            continue;
        }
        scanned += 1;
        for (dep, chain) in graph.transitive_non_dev_crates(&pkg.id) {
            if BANNED_PLATFORM_CRATES.contains(&dep.as_str()) {
                violations.push(Violation::new(
                    &pkg.rel_manifest,
                    1,
                    chain.join(" -> "),
                    "packages/core/* must not depend (directly or transitively) on platform backend crates (`web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`)",
                ));
            }
        }
    }
    assert!(scanned > 0, "expected core crates under packages/core/");
    violations
}

#[test]
fn core_backends_modules_zero_serde_json_runtime_dependency() {
    let idx = repo_index();
    let violations = check_zero_serde_json(&idx.graph);
    assert_no_violations(
        "Zero runtime/build serde_json in packages/{core,backends,modules}/*",
        &violations,
    );
}

pub fn check_zero_serde_json(graph: &WorkspaceGraph) -> Vec<Violation> {
    let pending: BTreeSet<&str> = PENDING_CLEANUP_SERDE_JSON.iter().map(|(p, _)| *p).collect();
    let mut seen_pending = BTreeSet::new();
    let mut violations = Vec::new();

    for pkg in graph.workspace_packages() {
        let in_scope = pkg.rel_dir.starts_with("packages/core/")
            || pkg.rel_dir.starts_with("packages/backends/")
            || pkg.rel_dir.starts_with("packages/modules/");
        if !in_scope {
            continue;
        }
        let closure = graph.transitive_non_dev_crates(&pkg.id);
        if let Some(chain) = closure.get("serde_json") {
            if pending.contains(pkg.name.as_str()) {
                seen_pending.insert(pkg.name.clone());
            } else {
                violations.push(Violation::new(
                    &pkg.rel_manifest,
                    1,
                    chain.join(" -> "),
                    "Runtime/build `serde_json` dependency (direct or transitive) is banned in packages/{core,backends,modules}/*",
                ));
            }
        }
    }

    for &(pkg_name, track) in PENDING_CLEANUP_SERDE_JSON {
        if !seen_pending.contains(pkg_name) {
            violations.push(Violation::new(
                "packages/tools/cli/tests/arch_invariants/crate_dag.rs",
                0,
                pkg_name,
                format!("`PENDING_CLEANUP_SERDE_JSON` entry `{pkg_name}` ({track}) no longer depends on `serde_json` — remove it from the allowlist"),
            ));
        }
    }
    violations
}

#[test]
fn zero_fnv_crate_dependency_workspace_wide() {
    let idx = repo_index();
    let violations = check_zero_fnv_crate_dep(&idx.graph);
    assert_no_violations("Zero `fnv` crate dependency across workspace", &violations);
}

pub fn check_zero_fnv_crate_dep(graph: &WorkspaceGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for pkg in graph.workspace_packages() {
        let closure = graph.transitive_non_dev_crates(&pkg.id);
        if let Some(chain) = closure.get("fnv") {
            violations.push(Violation::new(
                &pkg.rel_manifest,
                1,
                chain.join(" -> "),
                "The `fnv` crate is banned workspace-wide; standardize on `xxh64`",
            ));
        }
    }
    violations
}

#[test]
fn module_root_spec_purity_and_no_unscoped_skia_dev_deps() {
    let idx = repo_index();
    let mut violations = Vec::new();

    for pkg in idx.graph.workspace_packages() {
        if pkg.rel_dir.starts_with("packages/modules/") {
            // `packages/modules/<cap>` has exactly 2 slashes -> root spec crate.
            let is_root_spec = pkg.rel_dir.matches('/').count() == 2;
            if is_root_spec {
                for (dep, chain) in idx.graph.transitive_non_dev_crates(&pkg.id) {
                    if dep.starts_with("nve-backend-")
                        || BANNED_PLATFORM_CRATES.contains(&dep.as_str())
                    {
                        violations.push(Violation::new(
                            &pkg.rel_manifest,
                            1,
                            chain.join(" -> "),
                            "Root spec crate under packages/modules/<cap>/ must not depend on backends or platform crates; place adapters in leaf sub-crates",
                        ));
                    }
                }
            }
            if pkg.features.contains("web") && pkg.features.contains("skia") {
                violations.push(Violation::new(
                    &pkg.rel_manifest,
                    1,
                    "features: both `web` and `skia` declared",
                    "Module crate must not mix `web` and `skia` features in one crate; split into leaf `{web,skia}/` sub-crates",
                ));
            }
        }

        let in_wasm_layer = pkg.rel_dir.starts_with("packages/core/")
            || pkg.rel_dir.starts_with("packages/backends/")
            || pkg.rel_dir.starts_with("packages/framework/")
            || pkg.rel_dir.starts_with("packages/modules/");
        if in_wasm_layer {
            for dep in &pkg.declared_deps {
                if dep.kind == DepKind::Dev
                    && dep.target.is_none()
                    && (dep.name == "skia-safe" || dep.name == "nve-backend-skia")
                {
                    violations.push(Violation::new(
                        &pkg.rel_manifest,
                        1,
                        format!("[dev-dependencies] {}", dep.name),
                        "Unscoped skia dev-dependency breaks `wasm32 --all-targets`; gate under `[target.'cfg(not(target_arch = \"wasm32\"))'.dev-dependencies]`",
                    ));
                }
            }
        }
    }

    assert_no_violations(
        "Module root spec purity & no unscoped skia dev-deps",
        &violations,
    );
}

#[test]
fn no_core_or_backend_depends_on_modules_and_no_web_backend_auto_reg() {
    let idx = repo_index();
    let mut violations = check_core_backend_module_deps(&idx.graph);
    violations.extend(scan_web_backend_new_auto_reg());

    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            violations
                .iter()
                .any(|v| v.path == "packages/backends/web/Cargo.toml"),
            "Expected scanner to detect current `nve-camera` dep in packages/backends/web/Cargo.toml; if removed, promote this check!"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero core/backend dependencies on packages/modules/* and zero WebBackend::new() module special-casing",
        &violations,
    );
}

pub fn check_core_backend_module_deps(graph: &WorkspaceGraph) -> Vec<Violation> {
    let mut violations = Vec::new();
    for pkg in graph.workspace_packages() {
        if !pkg.rel_dir.starts_with("packages/core/")
            && !pkg.rel_dir.starts_with("packages/backends/")
        {
            continue;
        }
        for (dep_pkg, chain) in graph.transitive_workspace_deps(&pkg.id, false) {
            if dep_pkg.rel_dir.starts_with("packages/modules/") {
                violations.push(Violation::new(
                    &pkg.rel_manifest,
                    1,
                    chain.join(" -> "),
                    "Core crates and backends must have zero dependencies on optional packages/modules/*",
                ));
            }
        }
    }
    violations
}

fn scan_web_backend_new_auto_reg() -> Vec<Violation> {
    let idx = repo_index();
    let web_files = idx.rs_files_under("packages/backends/web/src");
    assert!(
        !web_files.is_empty(),
        "expected Rust sources under packages/backends/web/src"
    );

    let mut found_new = false;
    let mut violations = Vec::new();
    for rel in web_files {
        let Some(cached) = idx.read_rust(&rel) else {
            continue;
        };
        if let Some((start_line, orig_body, clean_body)) =
            extract_impl_fn_body(&cached.prod_raw, "WebBackend", "pub fn new(")
        {
            found_new = true;
            scan_web_backend_new_body(&rel, start_line, &orig_body, &clean_body, &mut violations);
        }
    }
    assert!(
        found_new,
        "Could not locate `impl WebBackend {{ pub fn new(...) }}` under packages/backends/web/src — update invariant anchor if renamed"
    );
    violations
}

pub fn scan_web_backend_new_body(
    rel: &str,
    start_line: usize,
    orig_body: &str,
    clean_body: &str,
    out: &mut Vec<Violation>,
) {
    let banned_idents = [
        "register_module",
        "register_extension",
        "nve_camera",
        "nve_motion",
        "CameraPreview",
        "MotionDriver",
    ];
    for (offset, (orig, clean)) in orig_body.lines().zip(clean_body.lines()).enumerate() {
        let line_no = start_line + offset;
        for ident in banned_idents {
            if contains_ident(clean, ident) {
                out.push(Violation::new(
                    rel,
                    line_no,
                    orig,
                    format!("WebBackend::new() must not special-case/auto-register module symbol `{ident}`"),
                ));
            }
        }
        // Check `#[cfg(feature = "camera")]` or similar feature-gated module wiring inside `new()`
        if orig.contains("feature") && (orig.contains("\"camera\"") || orig.contains("\"motion\"")) {
            out.push(Violation::new(
                rel,
                line_no,
                orig,
                "WebBackend::new() must not feature-gate optional module auto-registration",
            ));
        }
    }
}

/// Locates `impl ... <impl_type> { ... <fn_sig> ... { body } }` using comment/string-stripped
/// source so braces inside comments or strings never corrupt block matching.
pub fn extract_impl_fn_body(
    src: &str,
    impl_type: &str,
    fn_sig: &str,
) -> Option<(usize, String, String)> {
    let clean = strip_rust_comments_and_strings(src);
    let bytes = clean.as_bytes();
    let mut pos = 0usize;

    while let Some(rel_idx) = clean[pos..].find("impl") {
        let impl_start = pos + rel_idx;
        let before_ok = impl_start == 0
            || !(bytes[impl_start - 1].is_ascii_alphanumeric() || bytes[impl_start - 1] == b'_');
        let after = impl_start + 4;
        let after_ok = after < bytes.len()
            && !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if !before_ok || !after_ok {
            pos = impl_start + 4;
            continue;
        }
        let Some(brace_rel) = clean[impl_start..].find('{') else {
            break;
        };
        let header = &clean[impl_start..impl_start + brace_rel];
        let impl_body_start = impl_start + brace_rel;
        let impl_body_end = scan_balanced_braces(bytes, impl_body_start);
        // Must be an inherent `impl WebBackend` (not `impl Trait for WebBackend` unless desired,
        // and header must mention `impl_type` as a whole ident).
        if !contains_ident(header, "for") && contains_ident(header, impl_type) {
            let clean_impl = &clean[impl_body_start..impl_body_end];
            if let Some(fn_rel) = clean_impl.find(fn_sig) {
                let fn_abs = impl_body_start + fn_rel;
                let fn_brace_rel = clean[fn_abs..impl_body_end].find('{')?;
                let fn_body_start = fn_abs + fn_brace_rel;
                let fn_body_end = scan_balanced_braces(bytes, fn_body_start);
                let start_line = src[..fn_body_start].bytes().filter(|&b| b == b'\n').count() + 1;
                return Some((
                    start_line,
                    src[fn_body_start..fn_body_end].to_string(),
                    clean[fn_body_start..fn_body_end].to_string(),
                ));
            }
        }
        pos = impl_body_end;
    }
    None
}

fn scan_balanced_braces(bytes: &[u8], open_idx: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open_idx;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}
