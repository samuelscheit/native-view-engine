//! Shared std-only utilities for `arch_invariants`: repo traversal, `git ls-files`,
//! violation formatting, and lightweight `Cargo.toml` manifest parsing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use crate::rust_lex::{
    contains_ident, parse_rust_structs, production_code_prefix, strip_rust_comments_and_strings,
};
use crate::Stage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: String,
    pub line: usize,
    pub snippet: String,
    pub reason: String,
}

impl Violation {
    pub fn new(
        path: impl Into<String>,
        line: usize,
        snippet: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let raw = snippet.into();
        let trimmed = raw.trim();
        let snippet = if trimmed.len() > 140 {
            let mut end = 140;
            while !trimmed.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &trimmed[..end])
        } else {
            trimmed.to_string()
        };
        Self {
            path: path.into(),
            line,
            snippet,
            reason: reason.into(),
        }
    }
}

pub fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(3)
        .expect("packages/tools/cli must sit 3 levels below repo root")
        .to_path_buf()
}

pub fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn is_self_invariant_file(rel: &str) -> bool {
    rel == "packages/tools/cli/tests/arch_invariants.rs"
        || rel.starts_with("packages/tools/cli/tests/arch_invariants/")
}

pub fn assert_no_violations(invariant: &str, violations: &[Violation]) {
    if violations.is_empty() {
        return;
    }
    let mut msg = format!(
        "\nArchitectural invariant violated: {invariant} ({} violation(s)):\n",
        violations.len()
    );
    for v in violations.iter().take(60) {
        if v.line > 0 {
            msg.push_str(&format!("  - {}:{}: {}\n", v.path, v.line, v.reason));
        } else {
            msg.push_str(&format!("  - {}: {}\n", v.path, v.reason));
        }
        if !v.snippet.is_empty() {
            msg.push_str(&format!("      snippet: {}\n", v.snippet));
        }
    }
    if violations.len() > 60 {
        msg.push_str(&format!(
            "  ... and {} more violation(s)\n",
            violations.len() - 60
        ));
    }
    panic!("{msg}");
}

pub fn assert_staged_violations(stage: Stage, invariant: &str, violations: &[Violation]) {
    if stage.is_enforced() {
        assert_no_violations(&format!("{invariant} [{}]", stage.label()), violations);
    }
}

pub fn git_tracked_files(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("ls-files")
        .current_dir(root)
        .output()
        .expect("failed to run `git ls-files`");
    assert!(
        output.status.success(),
        "`git ls-files` exited with {:?}",
        output.status
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| l.replace('\\', "/"))
        .collect()
}

pub fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "target" || name_str == ".git" || name_str == "node_modules" {
            continue;
        }
        if path.is_dir() {
            walk_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

pub fn find_files_named(dir: &Path, exact_name: &str) -> Vec<PathBuf> {
    let mut all = Vec::new();
    walk_files(dir, &mut all);
    all.into_iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some(exact_name))
        .collect()
}

pub fn find_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut all = Vec::new();
    walk_files(dir, &mut all);
    all.into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
        .collect()
}

pub fn is_test_or_bench_rs(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| *p == "tests" || *p == "benches") {
        return true;
    }
    let file = parts.last().copied().unwrap_or("");
    file == "tests.rs" || file.ends_with("_test.rs") || file.ends_with("_tests.rs")
}

pub fn is_binary_ext(rel: &str) -> bool {
    const EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".wasm", ".ttf", ".otf", ".woff",
        ".woff2", ".pdf", ".zip", ".gz", ".tar", ".mp4", ".mov", ".icns", ".jar", ".class",
        ".so", ".dylib", ".dll", ".a", ".o", ".rlib", ".svg", ".ppm",
    ];
    let lower = rel.to_ascii_lowercase();
    EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Lightweight parsed `Cargo.toml` focusing on `[package]`, `[features]`, and dependency tables.
#[derive(Debug, Default)]
pub struct ParsedManifest {
    pub package_name: Option<String>,
    pub has_bench_table: bool,
    pub features: BTreeMap<String, (usize, String)>,
    /// Regular `[dependencies]` and `[target.<cfg>.dependencies]`: `(dep_name, line, section, rhs)`.
    pub runtime_deps: Vec<(String, usize, String, String)>,
    /// `[dev-dependencies]` and `[target.<cfg>.dev-dependencies]`.
    pub dev_deps: Vec<(String, usize, String, String)>,
}

pub fn parse_cargo_toml(content: &str) -> ParsedManifest {
    let mut out = ParsedManifest::default();
    let mut section = String::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("[[") && line.ends_with("]]") {
            let tbl = line[2..line.len() - 2].trim();
            if tbl == "bench" {
                out.has_bench_table = true;
            }
            section = line.to_string();
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim().trim_matches('"').to_string();
        let val = rhs.trim().to_string();
        if section == "package" && key == "name" {
            out.package_name = Some(val.trim_matches('"').to_string());
        } else if section == "features" {
            out.features.insert(key, (line_no, val));
        } else if section == "dependencies"
            || (section.starts_with("target.") && section.ends_with(".dependencies"))
        {
            out.runtime_deps.push((key, line_no, section.clone(), val));
        } else if section == "dev-dependencies"
            || (section.starts_with("target.") && section.ends_with(".dev-dependencies"))
        {
            out.dev_deps.push((key, line_no, section.clone(), val));
        }
    }
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_double => escaped = true,
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '#' if !in_single && !in_double => return &line[..i],
            _ => {}
        }
    }
    line
}
