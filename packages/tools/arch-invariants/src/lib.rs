//! Mechanical architectural invariant scanners for `native-view-engine` (L2 hybrid lint layer).
//!
//! Complements workspace Clippy lints (`Cargo.toml` + `clippy.toml`), `ast-grep`
//! (`lints/ast-grep/`), and `cargo-deny` (`deny.toml`) with cross-crate `guppy` DAG rules,
//! cross-file `syn` AST checks, repo hygiene, and benchmark / allocation-guard coverage.
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod ast_enums;
pub mod ast_layout;
pub mod bench;
pub mod codegen_53bit;
pub mod dag;
pub mod host_runtimes;
pub mod hot_path;
pub mod hygiene;
pub mod inline_style_keys;
pub mod persistence_guard;
pub mod purity;
pub mod rpc_dag;
pub mod spec_registry;
pub mod theme_hygiene;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{OnceLock, RwLock};

use guppy::graph::PackageGraph;
use guppy::MetadataCommand;

/// Documentation files legitimately citing historical acronyms or retired buzzwords.
pub const DOC_EXCLUDES: &[&str] = &[
    "docs/CHANGELOG.md",
    "docs/BACKLOG.md",
    "docs/quality-audit.md",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Post1B1D,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Post1B1D => "Post1B1D",
        }
    }

    pub fn is_enforced(self) -> bool {
        let Ok(raw) = std::env::var("NVE_STRICT_INVARIANTS") else {
            return false;
        };
        let trimmed = raw.trim();
        if is_truthy_env(trimmed) {
            return true;
        }
        let needle = match self {
            Self::Post1B1D => "1b1d",
        };
        trimmed
            .split(',')
            .any(|tok| tok.trim().eq_ignore_ascii_case(needle))
    }
}

fn is_truthy_env(val: &str) -> bool {
    val.eq_ignore_ascii_case("1")
        || val.eq_ignore_ascii_case("true")
        || val.eq_ignore_ascii_case("all")
        || val.eq_ignore_ascii_case("yes")
}

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

/// Returns `true` ONLY for `#[cfg(test)]` and `#[cfg(all(..., test, ...))]` (direct un-negated
/// `test` identifier). NEVER skips `#[cfg(any(..., test))]`, `#[cfg(not(test))]`, or
/// `#[cfg(feature = "test-utils")]`.
pub fn is_cfg_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| cfg_meta_is_test_only(&meta))
    })
}

fn cfg_meta_is_test_only(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(p) => p.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| items.iter().any(cfg_meta_is_test_only)),
        _ => false,
    }
}

pub struct ParsedRustFile {
    pub raw: String,
    pub ast: syn::File,
}

pub struct RepoIndex {
    pub root: PathBuf,
    pub tracked: Vec<String>,
    pub graph: PackageGraph,
    text_cache: RwLock<BTreeMap<String, Option<String>>>,
}

impl RepoIndex {
    fn build() -> Self {
        let root = repo_root();
        let tracked = git_tracked_files(&root);
        let mut cmd = MetadataCommand::new();
        cmd.current_dir(&root)
            .other_options(["--offline", "--locked"]);
        let graph = PackageGraph::from_command(&mut cmd)
            .expect("guppy failed to load workspace cargo metadata");
        Self {
            root,
            tracked,
            graph,
            text_cache: RwLock::new(BTreeMap::new()),
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

    pub fn parse_rust(&self, rel: &str) -> Option<ParsedRustFile> {
        let raw = self.read_text(rel)?;
        let ast = syn::parse_file(&raw)
            .map_err(|e| {
                format!(
                    "{rel}:{}:{}: {e}",
                    e.span().start().line,
                    e.span().start().column
                )
            })
            .expect("workspace Rust file must parse with syn");
        Some(ParsedRustFile { raw, ast })
    }

    pub fn parse_rust_if_contains(&self, rel: &str, needle: &str) -> Option<ParsedRustFile> {
        let raw = self.read_text(rel)?;
        if !raw.contains(needle) {
            return None;
        }
        let ast = syn::parse_file(&raw)
            .map_err(|e| {
                format!(
                    "{rel}:{}:{}: {e}",
                    e.span().start().line,
                    e.span().start().column
                )
            })
            .expect("workspace Rust file must parse with syn");
        Some(ParsedRustFile { raw, ast })
    }

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("packages/tools/arch-invariants must sit 3 levels below repo root")
        .to_path_buf()
}

pub fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn is_self_invariant_file(rel: &str) -> bool {
    rel.starts_with("packages/tools/arch-invariants/") || rel.starts_with("lints/ast-grep/")
}

pub fn is_test_or_bench_rs(rel: &str) -> bool {
    let fname = Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    rel.contains("/tests/")
        || rel.contains("/benches/")
        || fname == "tests.rs"
        || fname == "test.rs"
        || fname.ends_with("_tests.rs")
        || fname.starts_with("tests_")
}

pub fn is_binary_ext(rel: &str) -> bool {
    const EXTS: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".bmp", ".ttf", ".otf", ".woff",
        ".woff2", ".wasm", ".pdf", ".zip", ".gz", ".tar", ".tgz", ".jar", ".class", ".dex",
        ".so", ".dylib", ".dll", ".a", ".o", ".rlib", ".svg", ".ppm",
    ];
    EXTS.iter().any(|ext| {
        rel.len() >= ext.len()
            && rel.as_bytes()[rel.len() - ext.len()..].eq_ignore_ascii_case(ext.as_bytes())
    })
}

pub fn line_at(src: &str, line_1based: usize) -> &str {
    src.lines()
        .nth(line_1based.saturating_sub(1))
        .unwrap_or("")
}

pub fn find_files_named(dir: &Path, filename: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_files_no_symlinks(dir, &mut out);
    out.into_iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some(filename))
        .collect()
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
    #[expect(clippy::panic, reason = "architectural invariant assertion failure")]
    {
        panic!("{msg}");
    }
}

pub fn assert_staged_violations(stage: Stage, invariant: &str, violations: &[Violation]) {
    if stage.is_enforced() {
        assert_no_violations(&format!("{invariant} [{}]", stage.label()), violations);
        return;
    }
    let report = std::env::var("NVE_REPORT_INVARIANTS").is_ok_and(|raw| is_truthy_env(raw.trim()));
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
        }
    }
}

fn git_tracked_files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("ls-files")
        .arg("-z")
        .current_dir(root)
        .output()
        .expect("git ls-files failed");
    assert!(out.status.success(), "git ls-files exited non-zero");
    let mut files: Vec<String> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).replace('\\', "/"))
        .collect();
    files.sort();
    files
}

pub fn walk_files_no_symlinks(dir: &Path, out: &mut Vec<PathBuf>) {
    const SKIP: &[&str] = &[
        "target",
        ".git",
        "node_modules",
        "build",
        ".build",
        ".gradle",
        ".dart_tool",
        "pkg",
        "dist",
    ];
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !SKIP.contains(&name) {
                walk_files_no_symlinks(&path, out);
            }
        } else if meta.is_file() {
            out.push(path);
        }
    }
}
