//! Active-today repo hygiene checks: open-only BACKLOG, generated-file & artifact rules,
//! buzzword ban, personal endpoint/path guard, crate README check, and 650-line size ratchet.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::support::{
    assert_no_violations, find_files_named, find_rs_files, git_tracked_files, is_binary_ext,
    is_self_invariant_file, is_test_or_bench_rs, production_code_prefix, rel_path, repo_root,
    Violation,
};

/// Strictly sorted minimal allowlist of existing production `.rs` files in `packages/` over 650 lines.
/// Ratchets strictly downward: if a listed file shrinks to <= 650 lines or is deleted, the test fails
/// until its entry is removed here.
const EXISTING_OVERSIZE_ALLOWLIST: &[&str] = &[
    "packages/backends/skia/src/gpu/vulkan.rs",
    "packages/backends/web/src/snapshot.rs",
    "packages/core/interop/src/gesture.rs",
    "packages/core/interop/src/surface.rs",
];

/// Minimal tracked personal-path cleanup list in existing tests (ratchets strictly to zero).
const PENDING_CLEANUP_PERSONAL_PATHS: &[(&str, &str)] = &[
    ("packages/core/codegen/tests/codegen_tests.rs", "Track 1B-1e"),
];

const MAX_PROD_LINES: usize = 650;

#[test]
fn backlog_is_open_only() {
    let root = repo_root();
    let path = root.join("docs/BACKLOG.md");
    let content = fs::read_to_string(&path).expect("docs/BACKLOG.md must exist");
    let mut violations = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- [x]") || trimmed.starts_with("- [X]") {
            violations.push(Violation::new(
                "docs/BACKLOG.md",
                idx + 1,
                line,
                "docs/BACKLOG.md must stay open-only; move completed items to docs/CHANGELOG.md",
            ));
        }
    }
    assert_no_violations("Open-only docs/BACKLOG.md", &violations);
}

#[test]
fn generated_files_naming_and_zero_tracked_artifacts() {
    let root = repo_root();
    let tracked = git_tracked_files(&root);
    let mut violations = Vec::new();

    for rel in &tracked {
        if rel.contains(".generated.") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Generated `*.generated.*` files must be gitignored and never tracked in git",
            ));
        }
        if rel.ends_with(".trace.json") || rel.ends_with(".perfetto.json") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Trace artifacts (`*.trace.json` / `*.perfetto.json`) must never be tracked in git",
            ));
        }
        if rel.starts_with("cxx/build") || is_hosts_build_artifact(rel) {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Build output directories (`cxx/build*`, `hosts/*/build`) must never be tracked in git",
            ));
        }

        if is_binary_ext(rel) || rel == "Cargo.lock" || is_self_invariant_file(rel) {
            continue;
        }
        let Ok(content) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (idx, line) in content.lines().take(25).enumerate() {
            let trimmed = line.trim();
            let is_comment_header = trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
                || trimmed.starts_with('#')
                || trimmed.starts_with("<!--");
            if is_comment_header && trimmed.contains("@generated") {
                let in_java_gen = rel.contains("dev/nve/android/generated/");
                let has_dot_generated = Path::new(rel)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".generated."));
                if !in_java_gen && !has_dot_generated {
                    violations.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        "Non-Java `@generated` file must include `.generated.` in its filename (and remain untracked)",
                    ));
                }
                break;
            }
        }
    }
    assert_no_violations(
        "Generated-file naming & zero tracked generated/build/trace artifacts",
        &violations,
    );
}

fn is_hosts_build_artifact(rel: &str) -> bool {
    let mut parts = rel.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some("hosts"), Some(_), Some("build"))
    )
}

#[test]
fn zero_marketing_buzzword_zero_json() {
    let root = repo_root();
    let tracked = git_tracked_files(&root);
    let mut violations = Vec::new();
    let needle = ["zero", "-", "json"].concat();

    for rel in &tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || rel == "docs/CHANGELOG.md"
            || rel == "docs/BACKLOG.md"
            || rel == "docs/quality-audit.md"
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Ok(content) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&needle) {
                violations.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    "Marketing buzzword `Zero-JSON` is banned in manifests, Rust code, and active docs",
                ));
            }
        }
    }
    assert_no_violations("Zero marketing buzzword `Zero-JSON`", &violations);
}

#[test]
fn no_hardcoded_personal_endpoints_or_paths() {
    let root = repo_root();
    let tracked = git_tracked_files(&root);
    let pending: BTreeSet<&str> = PENDING_CLEANUP_PERSONAL_PATHS
        .iter()
        .map(|(p, _)| *p)
        .collect();
    let mut seen_pending = BTreeSet::new();
    let mut violations = Vec::new();

    for rel in &tracked {
        if is_binary_ext(rel) || is_self_invariant_file(rel) {
            continue;
        }
        let is_code = rel.ends_with(".rs")
            || rel.ends_with(".ts")
            || rel.ends_with(".js")
            || rel.ends_with(".mjs")
            || rel.ends_with(".swift")
            || rel.ends_with(".kt")
            || rel.ends_with(".java")
            || rel.ends_with(".dart")
            || rel.ends_with(".cpp")
            || rel.ends_with(".hpp")
            || rel.ends_with(".h")
            || rel.ends_with(".mm")
            || rel.ends_with(".sh");
        if !is_code {
            continue;
        }
        let Ok(content) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let is_test =
            is_test_or_bench_rs(rel) || rel.contains("/tests/") || rel.contains("examples/");
        let scan_text = if rel.ends_with(".rs") && !is_test {
            production_code_prefix(&content)
        } else {
            &content
        };

        for (idx, line) in scan_text.lines().enumerate() {
            if let Some(reason) = detect_personal_endpoint_or_path(line, is_test) {
                if pending.contains(rel.as_str()) {
                    seen_pending.insert(rel.clone());
                } else {
                    violations.push(Violation::new(rel, idx + 1, line, reason));
                }
            }
        }
    }

    for (p, track) in PENDING_CLEANUP_PERSONAL_PATHS {
        if !seen_pending.contains(*p) {
            violations.push(Violation::new(
                *p,
                0,
                *p,
                format!("`PENDING_CLEANUP_PERSONAL_PATHS` entry ({track}) is now clean — remove it from the allowlist"),
            ));
        }
    }

    assert_no_violations(
        "No hardcoded personal endpoints, hosts, or home paths",
        &violations,
    );
}

fn detect_personal_endpoint_or_path(line: &str, is_test: bool) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains(".ts.net") || lower.contains("ngrok.io") || lower.contains("trycloudflare.com")
    {
        return Some("Hardcoded personal tunnel/Tailscale endpoint is forbidden");
    }
    if has_user_home_prefix(line, "/Users/") || has_user_home_prefix(line, "/home/") {
        return Some(
            "Hardcoded local user home directory path (`/Users/...` or `/home/...`) is forbidden",
        );
    }
    if !is_test && contains_rfc1918_ip(line) {
        return Some("Hardcoded private RFC1918 IP address in production code is forbidden");
    }
    None
}

fn has_user_home_prefix(line: &str, prefix: &str) -> bool {
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(prefix) {
        let rest = &line[pos + idx + prefix.len()..];
        let seg: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect();
        if !seg.is_empty() && !matches!(seg.as_str(), "Shared" | "runner" | "user_placeholder") {
            return true;
        }
        pos += idx + prefix.len();
    }
    false
}

fn contains_rfc1918_ip(line: &str) -> bool {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if !bytes[i].is_ascii_digit() {
            continue;
        }
        if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.') {
            continue;
        }
        let end = bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .count()
            + i;
        let cand = &line[i..end];
        let octets: Vec<u16> = cand
            .split('.')
            .filter_map(|s| if s.is_empty() { None } else { s.parse::<u16>().ok() })
            .collect();
        if octets.len() == 4 && octets.iter().all(|o| *o <= 255) {
            let (a, b) = (octets[0], octets[1]);
            if a == 10 || (a == 192 && b == 168) || (a == 172 && (16..=31).contains(&b)) {
                return true;
            }
        }
    }
    false
}

#[test]
fn every_packages_crate_has_non_empty_readme() {
    let root = repo_root();
    let manifests = find_files_named(&root.join("packages"), "Cargo.toml");
    assert!(!manifests.is_empty(), "expected crates under packages/");
    let mut violations = Vec::new();

    for manifest in manifests {
        let crate_dir = manifest.parent().expect("Cargo.toml has parent dir");
        let readme = crate_dir.join("README.md");
        let rel = rel_path(&root, &readme);
        match fs::read_to_string(&readme) {
            Err(_) => violations.push(Violation::new(
                rel,
                0,
                "",
                "Every workspace crate under packages/ must have a factual README.md",
            )),
            Ok(content) => {
                let non_empty = content.lines().filter(|l| !l.trim().is_empty()).count();
                if non_empty < 5 {
                    violations.push(Violation::new(
                        rel,
                        1,
                        format!("{non_empty} non-empty lines"),
                        "Crate README.md must be non-empty (>= 5 non-empty lines: role, API, invariants, verification)",
                    ));
                }
            }
        }
    }
    assert_no_violations(
        "Every crate under packages/ has a non-empty README.md",
        &violations,
    );
}

#[test]
fn production_rust_files_max_650_lines_with_ratchet() {
    let root = repo_root();
    assert!(
        EXISTING_OVERSIZE_ALLOWLIST.windows(2).all(|w| w[0] < w[1]),
        "EXISTING_OVERSIZE_ALLOWLIST must stay strictly sorted and deduplicated"
    );
    let allowlist: BTreeSet<&str> = EXISTING_OVERSIZE_ALLOWLIST.iter().copied().collect();
    let mut still_oversize = BTreeSet::new();
    let mut violations = Vec::new();

    for file in find_rs_files(&root.join("packages")) {
        let rel = rel_path(&root, &file);
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let content = fs::read_to_string(&file).unwrap_or_default();
        let line_count = content.lines().count();
        if line_count > MAX_PROD_LINES {
            if allowlist.contains(rel.as_str()) {
                still_oversize.insert(rel);
            } else {
                violations.push(Violation::new(
                    rel,
                    line_count,
                    format!("{line_count} lines (limit {MAX_PROD_LINES})"),
                    "Production Rust file exceeds 650 lines; split into focused submodules",
                ));
            }
        }
    }

    for &listed in EXISTING_OVERSIZE_ALLOWLIST {
        if !still_oversize.contains(listed) {
            violations.push(Violation::new(
                listed,
                0,
                listed,
                "Allowlisted file is now <= 650 lines (or removed) — delete it from EXISTING_OVERSIZE_ALLOWLIST to ratchet down",
            ));
        }
    }

    assert_no_violations(
        "Production Rust file size <= 650 lines (strict ratchet)",
        &violations,
    );
}

#[test]
fn unit_personal_endpoint_detector_catches_leaks_and_allows_clean_code() {
    assert!(detect_personal_endpoint_or_path("let p = \"/Users/alice/dev\";", false).is_some());
    assert!(
        detect_personal_endpoint_or_path("let u = \"https://box.tail1234.ts.net\";", false)
            .is_some()
    );
    assert!(detect_personal_endpoint_or_path("let h = \"192.168.1.42:8080\";", false).is_some());
    assert!(detect_personal_endpoint_or_path("let h = \"192.168.1.42:8080\";", true).is_none());
    assert!(detect_personal_endpoint_or_path("let h = \"127.0.0.1:8080\";", false).is_none());
}
