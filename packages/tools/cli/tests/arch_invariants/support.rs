//! Shared std-only utilities and `OnceLock<RepoIndex>` cache for `arch_invariants`.
//!
//! Runs `git ls-files` and `cargo metadata --format-version 1 --offline --locked` ONCE per
//! test binary invocation and caches file contents + stripped Rust views so every test runs fast.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{OnceLock, RwLock};

pub use crate::json_meta::WorkspaceGraph;
pub use crate::rust_lex::{
    contains_ident, extract_raw_string_spans, parse_rust_enums, parse_rust_structs,
    strip_cfg_test_items, strip_rust_comments_and_strings,
};
use crate::Stage;

/// Shared exclusion list for documentation files that legitimately cite historical acronyms,
/// FNV-1a history, or retired buzzwords.
pub const DOC_EXCLUDES: &[&str] = &[
    "docs/CHANGELOG.md",
    "docs/BACKLOG.md",
    "docs/quality-audit.md",
];

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

/// Cached views of a Rust source file with 1-based line alignment preserved across all views.
#[derive(Debug, Clone)]
pub struct CachedRustFile {
    pub raw: String,
    /// Original source with `#[cfg(test)]` items blanked (newlines preserved).
    pub prod_raw: String,
    /// `prod_raw` with comments and string/char literals blanked (newlines preserved).
    pub prod_clean: String,
    /// Full file with comments and string/char literals blanked (newlines preserved).
    pub all_clean: String,
}

pub struct RepoIndex {
    pub root: PathBuf,
    pub tracked: Vec<String>,
    pub graph: WorkspaceGraph,
    text_cache: RwLock<BTreeMap<String, Option<String>>>,
    rust_cache: RwLock<BTreeMap<String, Option<CachedRustFile>>>,
}

impl RepoIndex {
    fn build() -> Self {
        let root = repo_root();
        let tracked = git_tracked_files_uncached(&root);
        let graph = WorkspaceGraph::load_from_cargo(&root);
        Self {
            root,
            tracked,
            graph,
            text_cache: RwLock::new(BTreeMap::new()),
            rust_cache: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn read_text(&self, rel: &str) -> Option<String> {
        if let Ok(guard) = self.text_cache.read()
            && let Some(cached) = guard.get(rel)
        {
            return cached.clone();
        }
        let val = fs::read_to_string(self.root.join(rel)).ok();
        if let Ok(mut guard) = self.text_cache.write() {
            guard.insert(rel.to_string(), val.clone());
        }
        val
    }

    pub fn read_rust(&self, rel: &str) -> Option<CachedRustFile> {
        if let Ok(guard) = self.rust_cache.read()
            && let Some(cached) = guard.get(rel)
        {
            return cached.clone();
        }
        let val = self.read_text(rel).map(|raw| {
            let prod_raw = strip_cfg_test_items(&raw);
            let prod_clean = strip_rust_comments_and_strings(&prod_raw);
            let all_clean = strip_rust_comments_and_strings(&raw);
            CachedRustFile {
                raw,
                prod_raw,
                prod_clean,
                all_clean,
            }
        });
        if let Ok(mut guard) = self.rust_cache.write() {
            guard.insert(rel.to_string(), val.clone());
        }
        val
    }

    /// Returns sorted workspace-relative paths of `.rs` files under `dir_prefix` (e.g. `"packages/"`),
    /// combining tracked files with any untracked on-disk files while ignoring symlinks & build dirs.
    pub fn rs_files_under(&self, dir_prefix: &str) -> Vec<String> {
        let target_dir = self.root.join(dir_prefix.trim_end_matches('/'));
        if !target_dir.exists() {
            return Vec::new();
        }
        let mut all = Vec::new();
        walk_files_no_symlinks(&target_dir, &mut all);
        all.into_iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
            .map(|p| rel_path(&self.root, &p))
            .collect()
    }
}

static REPO_INDEX: OnceLock<RepoIndex> = OnceLock::new();

pub fn repo_index() -> &'static RepoIndex {
    REPO_INDEX.get_or_init(RepoIndex::build)
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
        || rel.starts_with("lints/ast-grep/")
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
        return;
    }
    let report = std::env::var("NVE_REPORT_INVARIANTS").is_ok_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "all"
        )
    });
    if report {
        eprintln!(
            "\n[NVE_REPORT_INVARIANTS][{}] {} — {} violation(s):",
            stage.label(),
            invariant,
            violations.len()
        );
        for v in violations.iter().take(40) {
            if v.line > 0 {
                eprintln!("  - {}:{}: {}", v.path, v.line, v.reason);
            } else {
                eprintln!("  - {}: {}", v.path, v.reason);
            }
            if !v.snippet.is_empty() {
                eprintln!("      snippet: {}", v.snippet);
            }
        }
        if violations.len() > 40 {
            eprintln!("  ... and {} more violation(s)", violations.len() - 40);
        }
    }
}

fn git_tracked_files_uncached(root: &Path) -> Vec<String> {
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

/// Walks files without following symlinks and skips build/dependency directories.
pub fn walk_files_no_symlinks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            "target"
                | ".git"
                | "node_modules"
                | "build"
                | ".build"
                | ".gradle"
                | "DerivedData"
                | "Pods"
                | ".dart_tool"
        ) {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            walk_files_no_symlinks(&path, out);
        } else if ft.is_file() {
            out.push(path);
        }
    }
}

pub fn find_files_named(dir: &Path, exact_name: &str) -> Vec<PathBuf> {
    let mut all = Vec::new();
    walk_files_no_symlinks(dir, &mut all);
    all.into_iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some(exact_name))
        .collect()
}

/// Returns `true` only for dedicated test/bench files (`tests/`, `benches/`, `tests.rs`,
/// `*_tests.rs`). Does NOT match `*_test.rs` (e.g., production `hit_test.rs`).
pub fn is_test_or_bench_rs(rel: &str) -> bool {
    if rel.split('/').any(|p| p == "tests" || p == "benches") {
        return true;
    }
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file == "tests.rs" || file.ends_with("_tests.rs")
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
