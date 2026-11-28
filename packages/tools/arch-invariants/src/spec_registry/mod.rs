//! Mechanical spec-to-test invariant registry guard.
//!
//! Ensures every promised architectural invariant tagged `[INV:<id>]` in `AGENTS.md`,
//! `docs/BACKLOG.md`, or `docs/architecture.md` is backed by either:
//! - an active, non-ignored, non-`#[should_panic]` `#[test]` in `packages/tools/arch-invariants/`
//!   (`tests/` or `src/`) verified via full `syn::parse_file` AST inspection and containing a
//!   reachable, non-tautological assertion referencing `[INV:<id>]` in its doc/leading comments
//!   or body, or
//! - a unit-tested `ast-grep` rule under `lints/ast-grep/rules/` whose `id` matches `<id>`
//!   (or carries `# [INV:<id>]`) and is exercised in `lints/ast-grep/tests/`.

mod ast_scan;

use std::collections::{BTreeMap, BTreeSet};

use crate::{rel_path, walk_files_no_symlinks, RepoIndex, Violation};

pub use ast_scan::extract_test_invariant_guards;

pub const SPEC_DOCS: &[&str] = &["AGENTS.md", "docs/BACKLOG.md", "docs/architecture.md"];

const NON_GUARD_TEST_FILES: &[&str] = &[
    "packages/tools/arch-invariants/tests/unit_tests.rs",
    "packages/tools/arch-invariants/tests/rpc_server_purity_tests.rs",
    "packages/tools/arch-invariants/tests/spec_registry_tests.rs",
    "packages/tools/arch-invariants/src/spec_registry/mod.rs",
    "packages/tools/arch-invariants/src/spec_registry/ast_scan.rs",
];

/// Enforcement strength of a mechanical guard backing an `[INV:<id>]` promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardStrength {
    /// Enforced via a shrink-only monotonic ratchet (`assert_staged_ratchet`; regressions fail by default).
    Ratcheted,
    /// Enforced at zero tolerance (pure strict `#[test]` with zero ratchets, or unit-tested `ast-grep` rule).
    Enforced,
}

/// Mechanical coverage summary for all `[INV:<id>]` promises found in `SPEC_DOCS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCoverageReport {
    /// Distinct valid `[INV:<id>]` tags discovered in spec docs.
    pub spec_count: usize,
    /// Strongest backing guard strength (`Enforced` vs `Ratcheted`) for every covered spec ID.
    pub covered: BTreeMap<String, GuardStrength>,
    /// Uncovered or malformed `[INV:...]` violations.
    pub violations: Vec<Violation>,
}

impl SpecCoverageReport {
    pub fn enforced_count(&self) -> usize {
        self.covered
            .values()
            .filter(|&&s| s == GuardStrength::Enforced)
            .count()
    }

    pub fn ratcheted_count(&self) -> usize {
        self.covered
            .values()
            .filter(|&&s| s == GuardStrength::Ratcheted)
            .count()
    }
}

/// Runs the spec-to-guard coverage check across the live repository index.
pub fn check_repo_spec_invariants(idx: &RepoIndex) -> SpecCoverageReport {
    let mut spec_docs = Vec::new();
    for &doc in SPEC_DOCS {
        if let Some(content) = idx.read_text(doc) {
            spec_docs.push((doc.to_string(), content));
        }
    }
    assert!(
        !spec_docs.is_empty(),
        "expected spec docs (AGENTS.md / docs/BACKLOG.md / docs/architecture.md) to exist"
    );

    let mut guard_sources = Vec::new();
    for dir in [
        "packages/tools/arch-invariants/tests",
        "packages/tools/arch-invariants/src",
    ] {
        for rel in idx.rs_files_under(dir) {
            if NON_GUARD_TEST_FILES.contains(&rel.as_str()) {
                continue;
            }
            if let Some(content) = idx.read_text(&rel) {
                guard_sources.push((rel, content));
            }
        }
    }

    let ast_grep_rules = collect_yaml_files(&idx.root, "lints/ast-grep/rules");
    let ast_grep_tests = collect_yaml_files(&idx.root, "lints/ast-grep/tests");

    check_spec_invariant_coverage(&spec_docs, &guard_sources, &ast_grep_rules, &ast_grep_tests)
}

fn collect_yaml_files(root: &std::path::Path, dir: &str) -> Vec<(String, String)> {
    let target = root.join(dir);
    if !target.exists() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    walk_files_no_symlinks(&target, &mut paths);
    paths
        .into_iter()
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml")))
        .filter_map(|p| {
            let rel = rel_path(root, &p);
            std::fs::read_to_string(&p).ok().map(|text| (rel, text))
        })
        .collect()
}

pub fn check_spec_invariant_coverage(
    spec_docs: &[(String, String)],
    guard_sources: &[(String, String)],
    ast_grep_rules: &[(String, String)],
    ast_grep_tests: &[(String, String)],
) -> SpecCoverageReport {
    let mut violations = Vec::new();
    let mut spec_refs: BTreeMap<String, Vec<(String, usize, String)>> = BTreeMap::new();

    for (rel, content) in spec_docs {
        let (refs, malformed) = extract_spec_invariant_tags(rel, content);
        violations.extend(malformed);
        for (id, line_no, snippet) in refs {
            spec_refs
                .entry(id)
                .or_default()
                .push((rel.clone(), line_no, snippet));
        }
    }

    let mut all_guards: BTreeMap<String, GuardStrength> = BTreeMap::new();
    for (_rel, src) in guard_sources {
        for (id, strength) in extract_test_invariant_guards(src) {
            all_guards
                .entry(id)
                .and_modify(|cur| *cur = (*cur).max(strength))
                .or_insert(strength);
        }
    }
    for id in extract_tested_ast_grep_invariant_ids(ast_grep_rules, ast_grep_tests) {
        all_guards.insert(id, GuardStrength::Enforced);
    }

    let mut covered = BTreeMap::new();
    for (id, locs) in &spec_refs {
        if let Some(&strength) = all_guards.get(id) {
            covered.insert(id.clone(), strength);
        } else {
            for (rel, line_no, snippet) in locs {
                violations.push(Violation::new(
                    rel,
                    *line_no,
                    snippet,
                    format!(
                        "Promised invariant `[INV:{id}]` has no backing active `#[test]` in `packages/tools/arch-invariants/` or unit-tested `ast-grep` rule"
                    ),
                ));
            }
        }
    }

    SpecCoverageReport {
        spec_count: spec_refs.len(),
        covered,
        violations,
    }
}

/// Extracts valid `[INV:<id>]` tags from Markdown while skipping fenced code blocks and
/// inline backtick spans. Reports any malformed `[INV:...]` tag outside code spans.
pub fn extract_spec_invariant_tags(
    rel: &str,
    markdown: &str,
) -> (Vec<(String, usize, String)>, Vec<Violation>) {
    let mut valid = Vec::new();
    let mut malformed = Vec::new();
    let mut in_fence = false;

    for (idx, line) in markdown.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        let stripped = strip_inline_backticks(line);
        let mut pos = 0;
        while let Some(rel_idx) = stripped[pos..].find("[INV:") {
            let start = pos + rel_idx;
            let after_prefix = start + "[INV:".len();
            if let Some(close_rel) = stripped[after_prefix..].find(']') {
                let raw_id = &stripped[after_prefix..after_prefix + close_rel];
                if is_valid_inv_id(raw_id) {
                    valid.push((raw_id.to_string(), idx + 1, line.to_string()));
                } else {
                    malformed.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        format!(
                            "Malformed invariant tag `[INV:{raw_id}]`; use lowercase kebab-case `[INV:a-z0-9-]`"
                        ),
                    ));
                }
                pos = after_prefix + close_rel + 1;
            } else {
                malformed.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    "Unclosed `[INV:` invariant tag",
                ));
                break;
            }
        }
    }

    (valid, malformed)
}

fn strip_inline_backticks(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let mut ticks = 0;
            while i + ticks < bytes.len() && bytes[i + ticks] == b'`' {
                ticks += 1;
            }
            let delim = &line[i..i + ticks];
            let content_start = i + ticks;
            if let Some(end_rel) = line[content_start..].find(delim) {
                i = content_start + end_rel + ticks;
                out.push(' ');
                continue;
            }
            out.push_str(delim);
            i = content_start;
        } else {
            let ch = line[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub(super) fn is_valid_inv_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub(super) fn collect_valid_inv_tags(text: &str, out: &mut BTreeSet<String>) {
    let mut pos = 0;
    while let Some(rel_idx) = text[pos..].find("[INV:") {
        let after = pos + rel_idx + "[INV:".len();
        let Some(close_rel) = text[after..].find(']') else {
            break;
        };
        let cand = &text[after..after + close_rel];
        if is_valid_inv_id(cand) {
            out.insert(cand.to_string());
        }
        pos = after + close_rel + 1;
    }
}

/// Extracts invariant IDs from `ast-grep` YAML rule documents that have a matching rule test
/// (`id: <rule_id>`) in `lints/ast-grep/tests/`.
pub fn extract_tested_ast_grep_invariant_ids(
    rule_files: &[(String, String)],
    test_files: &[(String, String)],
) -> BTreeSet<String> {
    let mut tested_rule_ids = BTreeSet::new();
    for (_rel, content) in test_files {
        for line in content.lines() {
            if let Some(id) = parse_yaml_id_line(line) {
                tested_rule_ids.insert(id.to_string());
            }
        }
    }

    let mut covered = BTreeSet::new();
    for (_rel, content) in rule_files {
        for doc in content.split("\n---") {
            let mut doc_rule_id = None;
            let mut doc_tags = BTreeSet::new();
            for line in doc.lines() {
                if doc_rule_id.is_none()
                    && let Some(id) = parse_yaml_id_line(line)
                {
                    doc_rule_id = Some(id.to_string());
                }
                if line.trim_start().starts_with('#') {
                    collect_valid_inv_tags(line, &mut doc_tags);
                }
            }
            if let Some(rule_id) = doc_rule_id
                && tested_rule_ids.contains(&rule_id)
            {
                if is_valid_inv_id(&rule_id) {
                    covered.insert(rule_id);
                }
                covered.extend(doc_tags);
            }
        }
    }

    covered
}

fn parse_yaml_id_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("id:")?;
    let val = rest.trim().trim_matches('"').trim_matches('\'');
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}
