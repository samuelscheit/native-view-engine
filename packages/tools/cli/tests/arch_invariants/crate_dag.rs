//! Crate DAG, packaging, and module boundary invariants (`Cargo.toml` + `WebBackend::new()`).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::support::{
    assert_no_violations, assert_staged_violations, find_files_named, parse_cargo_toml, rel_path,
    repo_root, Violation,
};
use crate::Stage;

/// Approved Rust shell entry-point crates under `hosts/`.
const APPROVED_HOST_CARGO_TOMLS: &[&str] = &[
    "hosts/skia-shell/Cargo.toml",
    "hosts/web-shell/Cargo.toml",
];

/// Minimal tracked `serde_json` runtime dependency cleanup list (Track 1B-1e).
/// `packages/core/{abi,engine,dioxus}` and `packages/backends/*` and `packages/modules/*` have ZERO.
const PENDING_CLEANUP_SERDE_JSON: &[(&str, &str)] = &[
    ("packages/core/codegen/Cargo.toml", "Track 1B-1e"),
];

#[test]
fn no_sdk_library_crates_in_hosts() {
    let root = repo_root();
    let host_manifests = find_files_named(&root.join("hosts"), "Cargo.toml");
    let approved: BTreeSet<&str> = APPROVED_HOST_CARGO_TOMLS.iter().copied().collect();
    let mut violations = Vec::new();

    for manifest in &host_manifests {
        let rel = rel_path(&root, manifest);
        if !approved.contains(rel.as_str()) {
            violations.push(Violation::new(
                &rel,
                1,
                &rel,
                "hosts/ must not contain SDK library crates outside approved host shell entry points",
            ));
        }
    }

    for layer in [
        "packages/core",
        "packages/backends",
        "packages/framework",
        "packages/modules",
    ] {
        for manifest in find_files_named(&root.join(layer), "Cargo.toml") {
            let rel = rel_path(&root, &manifest);
            let content = fs::read_to_string(&manifest).unwrap_or_default();
            let parsed = parse_cargo_toml(&content);
            for (dep, line, _sec, rhs) in parsed.runtime_deps.iter().chain(parsed.dev_deps.iter()) {
                if dep.ends_with("-shell") || rhs.contains("hosts/") {
                    violations.push(Violation::new(
                        &rel,
                        *line,
                        format!("{dep} = {rhs}"),
                        "Library crates under packages/ must never depend on host shell crates in hosts/",
                    ));
                }
            }
        }
    }

    assert_no_violations("No SDK library crates in hosts/", &violations);
}

#[test]
fn core_crates_zero_dioxus_dependency() {
    let root = repo_root();
    let mut violations = Vec::new();
    for crate_dir in ["abi", "abi-macros", "engine", "hash", "store", "tables"] {
        let manifest = root.join("packages/core").join(crate_dir).join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let rel = rel_path(&root, &manifest);
        let content = fs::read_to_string(&manifest).unwrap_or_default();
        let parsed = parse_cargo_toml(&content);
        for (dep, line, sec, rhs) in &parsed.runtime_deps {
            if dep.starts_with("dioxus") {
                violations.push(Violation::new(
                    &rel,
                    *line,
                    format!("[{sec}] {dep} = {rhs}"),
                    "Core crates (`abi`, `engine`, `hash`, `store`, `tables`) must be framework-agnostic with zero `dioxus*` dependency",
                ));
            }
        }
    }
    assert_no_violations("Zero dioxus* in framework-agnostic core crates", &violations);
}

#[test]
fn core_crates_zero_platform_backend_dependencies() {
    let root = repo_root();
    let banned = ["web-sys", "js-sys", "wasm-bindgen", "skia-safe"];
    let mut violations = Vec::new();
    for manifest in find_files_named(&root.join("packages/core"), "Cargo.toml") {
        let rel = rel_path(&root, &manifest);
        let content = fs::read_to_string(&manifest).unwrap_or_default();
        let parsed = parse_cargo_toml(&content);
        for (dep, line, sec, rhs) in &parsed.runtime_deps {
            if banned.contains(&dep.as_str()) {
                violations.push(Violation::new(
                    &rel,
                    *line,
                    format!("[{sec}] {dep} = {rhs}"),
                    "packages/core/* must not depend on platform backend crates (`web-sys`, `js-sys`, `wasm-bindgen`, `skia-safe`)",
                ));
            }
        }
    }
    assert_no_violations(
        "Zero web-sys/js-sys/wasm-bindgen/skia-safe in packages/core/*",
        &violations,
    );
}

#[test]
fn core_backends_modules_zero_serde_json_runtime_dependency() {
    let root = repo_root();
    let pending: BTreeSet<&str> = PENDING_CLEANUP_SERDE_JSON.iter().map(|(p, _)| *p).collect();
    let mut seen_pending = BTreeSet::new();
    let mut violations = Vec::new();

    for layer in ["packages/core", "packages/backends", "packages/modules"] {
        for manifest in find_files_named(&root.join(layer), "Cargo.toml") {
            let rel = rel_path(&root, &manifest);
            let content = fs::read_to_string(&manifest).unwrap_or_default();
            let parsed = parse_cargo_toml(&content);
            for (dep, line, sec, rhs) in &parsed.runtime_deps {
                if dep == "serde_json" {
                    if pending.contains(rel.as_str()) {
                        seen_pending.insert(rel.clone());
                    } else {
                        violations.push(Violation::new(
                            &rel,
                            *line,
                            format!("[{sec}] {dep} = {rhs}"),
                            "Runtime `serde_json` dependency is banned in packages/{core,backends,modules}/*",
                        ));
                    }
                }
            }
        }
    }

    for (p, track) in PENDING_CLEANUP_SERDE_JSON {
        if !seen_pending.contains(*p) {
            violations.push(Violation::new(
                *p,
                0,
                *p,
                format!("`PENDING_CLEANUP_SERDE_JSON` entry ({track}) no longer depends on `serde_json` — remove it from the allowlist"),
            ));
        }
    }

    assert_no_violations(
        "Zero runtime serde_json in packages/{core,backends,modules}/*",
        &violations,
    );
}

#[test]
fn module_root_spec_purity_and_no_unscoped_skia_dev_deps() {
    let root = repo_root();
    let mut violations = Vec::new();

    for manifest in find_files_named(&root.join("packages/modules"), "Cargo.toml") {
        let rel = rel_path(&root, &manifest);
        let is_root_spec = rel.matches('/').count() == 3;
        let content = fs::read_to_string(&manifest).unwrap_or_default();
        let parsed = parse_cargo_toml(&content);
        if is_root_spec {
            for (dep, line, sec, rhs) in &parsed.runtime_deps {
                if dep.starts_with("nve-backend-") || dep == "web-sys" || dep == "skia-safe" {
                    violations.push(Violation::new(
                        &rel,
                        *line,
                        format!("[{sec}] {dep} = {rhs}"),
                        "Root spec crate under packages/modules/<cap>/ must not depend on backends/web-sys/skia-safe; place adapters in leaf sub-crates",
                    ));
                }
            }
        }
        if parsed.features.contains_key("web") && parsed.features.contains_key("skia") {
            let line = parsed.features["web"].0;
            violations.push(Violation::new(
                &rel,
                line,
                "features: both `web` and `skia` declared",
                "Module crate must not mix `web` and `skia` features in one crate; split into leaf `{web,skia}/` sub-crates",
            ));
        }
    }

    for layer in [
        "packages/core",
        "packages/backends",
        "packages/framework",
        "packages/modules",
    ] {
        for manifest in find_files_named(&root.join(layer), "Cargo.toml") {
            let rel = rel_path(&root, &manifest);
            let content = fs::read_to_string(&manifest).unwrap_or_default();
            let parsed = parse_cargo_toml(&content);
            for (dep, line, sec, rhs) in &parsed.dev_deps {
                if sec == "dev-dependencies" && (dep == "skia-safe" || dep == "nve-backend-skia") {
                    violations.push(Violation::new(
                        &rel,
                        *line,
                        format!("[{sec}] {dep} = {rhs}"),
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
    let root = repo_root();
    let violations = scan_core_backend_module_deps(&root);
    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            violations
                .iter()
                .any(|v| v.path == "packages/backends/web/Cargo.toml"),
            "Expected scanner to detect current `nve-camera` dep in packages/backends/web/Cargo.toml; if removed, flip ENFORCE_POST_NODESTORE or promote this check!"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero core/backend dependencies on packages/modules/* and zero WebBackend::new() module special-casing",
        &violations,
    );
}

fn scan_core_backend_module_deps(root: &Path) -> Vec<Violation> {
    let mut module_crates = BTreeSet::new();
    for manifest in find_files_named(&root.join("packages/modules"), "Cargo.toml") {
        if let Ok(content) = fs::read_to_string(&manifest)
            && let Some(name) = parse_cargo_toml(&content).package_name
        {
            module_crates.insert(name);
        }
    }

    let mut violations = Vec::new();
    for layer in ["packages/core", "packages/backends"] {
        for manifest in find_files_named(&root.join(layer), "Cargo.toml") {
            let rel = rel_path(root, &manifest);
            let content = fs::read_to_string(&manifest).unwrap_or_default();
            let parsed = parse_cargo_toml(&content);
            for (dep, line, sec, rhs) in &parsed.runtime_deps {
                if module_crates.contains(dep) || rhs.contains("modules/") {
                    violations.push(Violation::new(
                        &rel,
                        *line,
                        format!("[{sec}] {dep} = {rhs}"),
                        "Core crates and backends must have zero dependencies on optional packages/modules/*",
                    ));
                }
            }
        }
    }

    let web_lib = root.join("packages/backends/web/src/lib.rs");
    if let Ok(content) = fs::read_to_string(&web_lib)
        && let Some(body) = extract_fn_body(&content, "pub fn new(")
    {
        let banned_tokens = [
            "camera",
            "motion",
            "svg",
            "canvas_module",
            "register_module",
        ];
        for (offset, line) in body.1.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            for tok in banned_tokens {
                if lower.contains(tok) {
                    violations.push(Violation::new(
                        "packages/backends/web/src/lib.rs",
                        body.0 + offset,
                        line,
                        format!(
                            "WebBackend::new() must not special-case/auto-register optional module `{tok}`"
                        ),
                    ));
                }
            }
        }
    }
    violations
}

fn extract_fn_body<'a>(src: &'a str, signature_prefix: &str) -> Option<(usize, &'a str)> {
    let start_idx = src.find(signature_prefix)?;
    let start_line = src[..start_idx].bytes().filter(|b| *b == b'\n').count() + 1;
    let brace_rel = src[start_idx..].find('{')?;
    let body_start = start_idx + brace_rel;
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = body_start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start_line, &src[body_start..=i]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
