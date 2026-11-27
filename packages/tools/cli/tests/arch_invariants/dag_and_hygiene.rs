//! Active-today repo hygiene checks: open-only BACKLOG, generated-file & artifact rules,
//! buzzword ban, personal endpoint/path monotonic ratchet, crate README check, and monotonic
//! 650-line production file size ratchet.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::support::{
    assert_no_violations, find_files_named, is_binary_ext, is_self_invariant_file,
    is_test_or_bench_rs, rel_path, repo_index, Violation, DOC_EXCLUDES,
};

/// Strictly sorted monotonic allowlist of existing production `.rs` files in `packages/` over
/// 650 lines: `(path, max_lines)`. Fails if ANY allowlisted file grows above `max_lines` OR
/// shrinks to `<= 650` lines (forcing the allowlist strictly downward to zero).
const EXISTING_OVERSIZE_ALLOWLIST: &[(&str, usize)] = &[
    ("packages/backends/web/src/snapshot.rs", 690),
    ("packages/core/interop/src/gesture.rs", 927),
    ("packages/core/interop/src/surface.rs", 947),
];

/// Monotonic ratchet of pre-existing personal-path hits: `(path, max_hits)`.
/// Fails if hits exceed `max_hits` OR drop to 0 (stale allowlist).
const PENDING_CLEANUP_PERSONAL_PATHS: &[(&str, usize)] =
    &[("packages/core/codegen/tests/codegen_tests.rs", 1)];

const MAX_PROD_LINES: usize = 650;

const PERSONAL_SCAN_EXTS: &[&str] = &[
    ".rs",
    ".ts",
    ".js",
    ".mjs",
    ".swift",
    ".kt",
    ".kts",
    ".java",
    ".dart",
    ".cpp",
    ".hpp",
    ".h",
    ".c",
    ".mm",
    ".sh",
    ".py",
    ".toml",
    ".json",
    ".yml",
    ".yaml",
    ".properties",
    ".mdx",
    ".md",
];

/// Enforces [INV:backlog-open-only].
#[test]
fn backlog_is_open_only() {
    let idx = repo_index();
    let content = idx
        .read_text("docs/BACKLOG.md")
        .expect("docs/BACKLOG.md must exist");
    let mut violations = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [X]")
            || trimmed.starts_with("* [x]")
            || trimmed.starts_with("* [X]")
        {
            violations.push(Violation::new(
                "docs/BACKLOG.md",
                line_idx + 1,
                line,
                "docs/BACKLOG.md must stay open-only; move completed items to docs/CHANGELOG.md",
            ));
        }
    }
    assert_no_violations("Open-only docs/BACKLOG.md", &violations);
}

/// Enforces [INV:generated-files-hygiene].
#[test]
fn generated_files_naming_and_zero_tracked_artifacts() {
    let idx = repo_index();
    let mut violations = Vec::new();

    for rel in &idx.tracked {
        if rel.contains(".generated.") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Generated `*.generated.*` files must be gitignored and never tracked in git",
            ));
        }
        if rel.contains("dev/nve/android/generated/") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Generated Java files under `dev/nve/android/generated/` must be gitignored and never tracked in git",
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
                "Build output directories (`cxx/build*`, `hosts/**/{build,.build,.gradle,.dart_tool}`) must never be tracked in git",
            ));
        }

        if is_binary_ext(rel) || rel == "Cargo.lock" || is_self_invariant_file(rel) {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        for (line_idx, line) in content.lines().take(25).enumerate() {
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
                        line_idx + 1,
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
    rel.starts_with("hosts/")
        && rel
            .split('/')
            .any(|seg| matches!(seg, "build" | ".build" | ".gradle" | ".dart_tool"))
}

#[test]
fn zero_marketing_buzzword_zero_json() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let needle = ["zero", "-", "json"].concat();

    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        for (line_idx, line) in content.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&needle) {
                violations.push(Violation::new(
                    rel,
                    line_idx + 1,
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
    let idx = repo_index();
    let pending: BTreeMap<&str, usize> = PENDING_CLEANUP_PERSONAL_PATHS.iter().copied().collect();
    let mut pending_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut violations = Vec::new();

    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || is_self_invariant_file(rel)
            || rel == "docs/CHANGELOG.md"
            || !PERSONAL_SCAN_EXTS.iter().any(|ext| rel.ends_with(ext))
        {
            continue;
        }
        let is_test_or_doc = is_test_or_bench_rs(rel)
            || rel.contains("/tests/")
            || rel.contains("examples/")
            || rel.ends_with(".md")
            || rel.ends_with(".mdx");

        let scan_text = if rel.ends_with(".rs") && !is_test_or_doc {
            let Some(cached) = idx.read_rust(rel) else {
                continue;
            };
            cached.prod_raw
        } else {
            let Some(content) = idx.read_text(rel) else {
                continue;
            };
            content
        };

        for (line_idx, line) in scan_text.lines().enumerate() {
            if let Some(reason) = detect_personal_endpoint_or_path(line, is_test_or_doc) {
                if let Some(&max_hits) = pending.get(rel.as_str()) {
                    let count = pending_counts.entry(rel.clone()).or_insert(0);
                    *count += 1;
                    if *count > max_hits {
                        violations.push(Violation::new(
                            rel,
                            line_idx + 1,
                            line,
                            format!(
                                "Personal-path hits in `{rel}` grew to {count} (ratchet ceiling {max_hits}): {reason}"
                            ),
                        ));
                    }
                } else {
                    violations.push(Violation::new(rel, line_idx + 1, line, reason));
                }
            }
        }
    }

    for &(p, max_hits) in PENDING_CLEANUP_PERSONAL_PATHS {
        let actual = pending_counts.get(p).copied().unwrap_or(0);
        if actual < max_hits {
            violations.push(Violation::new(
                p,
                0,
                format!("actual={actual}, allowlist={max_hits}"),
                "PENDING_CLEANUP_PERSONAL_PATHS entry dropped below its ceiling — lower or delete it to ratchet down",
            ));
        }
    }

    assert_no_violations(
        "No hardcoded personal endpoints, hosts, or home paths (monotonic ratchet)",
        &violations,
    );
}

pub fn detect_personal_endpoint_or_path(line: &str, is_test_or_doc: bool) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains(".ts.net")
        || lower.contains("ngrok.io")
        || lower.contains("trycloudflare.com")
    {
        return Some("Hardcoded personal tunnel/Tailscale endpoint is forbidden");
    }
    if has_user_home_prefix(line, "/Users/")
        || has_user_home_prefix(line, "/home/")
        || has_windows_user_home(line)
    {
        return Some(
            "Hardcoded local user home directory path (`/Users/...`, `/home/...`, or `C:\\Users\\...`) is forbidden",
        );
    }
    if !is_test_or_doc && contains_rfc1918_ip(line) {
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

fn has_windows_user_home(line: &str) -> bool {
    for prefix in ["C:\\Users\\", "c:\\Users\\", "C:/Users/", "c:/Users/"] {
        if has_user_home_prefix(line, prefix) {
            return true;
        }
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
    let idx = repo_index();
    let manifests = find_files_named(&idx.root.join("packages"), "Cargo.toml");
    assert!(!manifests.is_empty(), "expected crates under packages/");
    let mut violations = Vec::new();

    for manifest in manifests {
        let crate_dir = manifest.parent().expect("Cargo.toml has parent dir");
        let readme = crate_dir.join("README.md");
        let rel = rel_path(&idx.root, &readme);
        match idx.read_text(&rel) {
            None => violations.push(Violation::new(
                rel,
                0,
                "",
                "Every workspace crate under packages/ must have a factual README.md",
            )),
            Some(content) => {
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

/// Enforces [INV:max-650-lines].
#[test]
fn production_rust_files_max_650_lines_with_ratchet() {
    let idx = repo_index();
    assert!(
        EXISTING_OVERSIZE_ALLOWLIST.windows(2).all(|w| w[0].0 < w[1].0),
        "EXISTING_OVERSIZE_ALLOWLIST must stay strictly sorted and deduplicated"
    );
    let allowlist: BTreeMap<&str, usize> = EXISTING_OVERSIZE_ALLOWLIST.iter().copied().collect();
    let mut still_oversize = BTreeSet::new();
    let mut violations = Vec::new();

    let rs_files = idx.rs_files_under("packages");
    assert!(!rs_files.is_empty(), "expected Rust files under packages/");

    for rel in rs_files {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let Some(content) = idx.read_text(&rel) else {
            continue;
        };
        let line_count = content.lines().count();
        if line_count > MAX_PROD_LINES {
            if let Some(&max_lines) = allowlist.get(rel.as_str()) {
                still_oversize.insert(rel.clone());
                if line_count > max_lines {
                    violations.push(Violation::new(
                        rel,
                        0,
                        format!("{line_count} lines (ratchet ceiling {max_lines})"),
                        "Allowlisted oversize file grew past its monotonic ceiling; split instead of growing",
                    ));
                }
            } else {
                violations.push(Violation::new(
                    rel,
                    0,
                    format!("{line_count} lines (limit {MAX_PROD_LINES})"),
                    "Production Rust file exceeds 650 lines; split into focused submodules",
                ));
            }
        }
    }

    for &(listed, _max) in EXISTING_OVERSIZE_ALLOWLIST {
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
        "Production Rust file size <= 650 lines (strict monotonic ratchet)",
        &violations,
    );
}
