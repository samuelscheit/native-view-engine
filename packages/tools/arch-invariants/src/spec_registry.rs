//! Mechanical spec-to-test invariant registry guard.
//!
//! Ensures every promised architectural invariant tagged `[INV:<id>]` in `AGENTS.md`,
//! `docs/BACKLOG.md`, or `docs/architecture.md` is backed by either:
//! - a non-ignored `#[test]` in `packages/tools/arch-invariants/` (`tests/` or `src/`)
//!   referencing `[INV:<id>]` in its doc/leading comments or body, or
//! - a unit-tested `ast-grep` rule under `lints/ast-grep/rules/` whose `id` matches `<id>`
//!   (or carries `# [INV:<id>]`) and is exercised in `lints/ast-grep/tests/`.

use std::collections::{BTreeMap, BTreeSet};

use crate::{rel_path, walk_files_no_symlinks, RepoIndex, Violation};

pub const SPEC_DOCS: &[&str] = &["AGENTS.md", "docs/BACKLOG.md", "docs/architecture.md"];

const NON_GUARD_TEST_FILES: &[&str] = &[
    "packages/tools/arch-invariants/tests/unit_tests.rs",
    "packages/tools/arch-invariants/tests/rpc_server_purity_tests.rs",
    "packages/tools/arch-invariants/tests/spec_registry_tests.rs",
    "packages/tools/arch-invariants/src/spec_registry.rs",
];

/// Runs the spec-to-guard coverage check across the live repository index.
pub fn check_repo_spec_invariants(idx: &RepoIndex) -> (usize, Vec<Violation>) {
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
) -> (usize, Vec<Violation>) {
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

    let mut covered = BTreeSet::new();
    for (_rel, src) in guard_sources {
        covered.extend(extract_test_covered_invariant_ids(src));
    }
    covered.extend(extract_tested_ast_grep_invariant_ids(
        ast_grep_rules,
        ast_grep_tests,
    ));

    for (id, locs) in &spec_refs {
        if !covered.contains(id) {
            for (rel, line_no, snippet) in locs {
                violations.push(Violation::new(
                    rel,
                    *line_no,
                    snippet,
                    format!(
                        "Promised invariant `[INV:{id}]` has no backing non-ignored `#[test]` in `packages/tools/arch-invariants/` or unit-tested `ast-grep` rule"
                    ),
                ));
            }
        }
    }

    (spec_refs.len(), violations)
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

fn is_valid_inv_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Enforcement strength of a `#[test]` guard backing an `[INV:<id>]` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardStrength {
    /// Enforced via a shrink-only monotonic ratchet (`assert_staged_ratchet`; regressions fail by default).
    Ratcheted,
    /// Enforced at zero tolerance (`assert_no_violations`, `assert!`, `assert_eq!`, `assert_ne!`, `panic!`).
    Enforced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyGuardKind {
    Enforced,
    Ratcheted,
    Unenforced,
}

/// Parses a `#[test]` function body with `syn` so comments/strings cannot spoof assertions,
/// rejecting empty tests and any test that invokes an unforced `assert_staged_violations(...)`.
fn classify_test_body(body: &str) -> BodyGuardKind {
    let Ok(block) = syn::parse_str::<syn::Block>(body) else {
        return BodyGuardKind::Unenforced;
    };
    struct AssertScan {
        has_strict: bool,
        has_ratchet: bool,
        has_unforced_staged: bool,
    }
    impl<'ast> syn::visit::Visit<'ast> for AssertScan {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(ep) = &*node.func
                && let Some(seg) = ep.path.segments.last()
            {
                match seg.ident.to_string().as_str() {
                    "assert_no_violations" => self.has_strict = true,
                    "assert_staged_ratchet" => self.has_ratchet = true,
                    "assert_staged_violations" => self.has_unforced_staged = true,
                    _ => {}
                }
            }
            syn::visit::visit_expr_call(self, node);
        }

        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if let Some(seg) = node.path.segments.last()
                && matches!(
                    seg.ident.to_string().as_str(),
                    "assert" | "assert_eq" | "assert_ne" | "panic"
                )
            {
                self.has_strict = true;
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut scan = AssertScan {
        has_strict: false,
        has_ratchet: false,
        has_unforced_staged: false,
    };
    syn::visit::visit_block(&mut scan, &block);
    if scan.has_unforced_staged {
        BodyGuardKind::Unenforced
    } else if scan.has_strict {
        BodyGuardKind::Enforced
    } else if scan.has_ratchet {
        BodyGuardKind::Ratcheted
    } else {
        BodyGuardKind::Unenforced
    }
}

/// Extracts `[INV:<id>]` tags attached to non-ignored `#[test]` functions whose body contains
/// a real enforcing assertion (either zero-tolerance or shrink-only ratcheted; never empty or
/// unforced `assert_staged_violations`), recording the strongest backing guard per ID.
pub fn extract_test_invariant_guards(src: &str) -> BTreeMap<String, GuardStrength> {
    let mut guards: BTreeMap<String, GuardStrength> = BTreeMap::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut line_offsets = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        line_offsets.push(offset);
        offset += line.len();
    }
    if line_offsets.len() <= lines.len() {
        line_offsets.push(offset);
    }

    let bytes = src.as_bytes();
    let mut i = 0;
    while i < lines.len() {
        if !is_test_attr_line(lines[i]) {
            i += 1;
            continue;
        }

        // Walk upward over contiguous comments/attributes attached to this test item.
        let mut header_start = i;
        while header_start > 0 {
            let prev = lines[header_start - 1].trim();
            if prev.starts_with("//") || prev.starts_with("#[") {
                header_start -= 1;
            } else {
                break;
            }
        }

        // Walk downward across attributes/comments until the `fn` signature and opening `{`.
        let mut is_ignored = false;
        let mut j = header_start;
        let mut fn_line = None;
        while j < lines.len() {
            let t = lines[j].trim();
            if t.starts_with("#[ignore") {
                is_ignored = true;
            }
            if j >= i
                && (t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("async fn "))
            {
                fn_line = Some(j);
                break;
            }
            if j > i && !t.starts_with("#[") && !t.starts_with("//") && !t.is_empty() {
                break;
            }
            j += 1;
        }

        let Some(fn_idx) = fn_line else {
            i += 1;
            continue;
        };
        if is_ignored {
            i = fn_idx + 1;
            continue;
        }

        let search_from = line_offsets.get(fn_idx).copied().unwrap_or(src.len());
        if let Some(brace_rel) = src[search_from..].find('{') {
            let brace_abs = search_from + brace_rel;
            let end_abs = scan_balanced_braces(bytes, brace_abs);
            let body = &src[brace_abs..end_abs];
            let strength = match classify_test_body(body) {
                BodyGuardKind::Enforced => Some(GuardStrength::Enforced),
                BodyGuardKind::Ratcheted => Some(GuardStrength::Ratcheted),
                BodyGuardKind::Unenforced => None,
            };
            if let Some(strength) = strength {
                let mut tags = BTreeSet::new();
                for &hdr in &lines[header_start..fn_idx] {
                    collect_valid_inv_tags(hdr, &mut tags);
                }
                collect_valid_inv_tags(body, &mut tags);
                for id in tags {
                    guards
                        .entry(id)
                        .and_modify(|cur| *cur = (*cur).max(strength))
                        .or_insert(strength);
                }
            }
            let next_line = line_offsets.partition_point(|&off| off < end_abs);
            i = next_line.max(fn_idx + 1);
        } else {
            i = fn_idx + 1;
        }
    }

    guards
}

/// Extracts `[INV:<id>]` tags backed by enforcing (strict or ratcheted) non-ignored `#[test]` fns.
pub fn extract_test_covered_invariant_ids(src: &str) -> BTreeSet<String> {
    extract_test_invariant_guards(src).into_keys().collect()
}

/// Scans from an opening `{` to its matching `}` while skipping Rust line/block comments,
/// normal strings, raw strings (`r#"..."#`), and char literals so braces inside them never skew depth.
fn scan_balanced_braces(bytes: &[u8], open_idx: usize) -> usize {
    let mut i = open_idx;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                let mut nest = 1usize;
                while i + 1 < bytes.len() && nest > 0 {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        nest += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        nest -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'r' | b'b' => {
                let start = if bytes[i] == b'b' && i + 1 < bytes.len() && bytes[i + 1] == b'r' {
                    i + 2
                } else if bytes[i] == b'r' {
                    i + 1
                } else {
                    i += 1;
                    continue;
                };
                let mut hashes = 0usize;
                while start + hashes < bytes.len() && bytes[start + hashes] == b'#' {
                    hashes += 1;
                }
                if start + hashes < bytes.len() && bytes[start + hashes] == b'"' {
                    i = start + hashes + 1;
                    while i < bytes.len() {
                        if bytes[i] == b'"'
                            && i + hashes < bytes.len()
                            && bytes[i + 1..=i + hashes].iter().all(|&b| b == b'#')
                        {
                            i += 1 + hashes;
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = (i + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                // Skip char literals `'x'` or `'\''` without mis-parsing lifetimes `'a`.
                if i + 2 < bytes.len() && bytes[i + 1] == b'\\' {
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' {
                        j += 1;
                    }
                    i = if j < bytes.len() && bytes[j] == b'\'' { j + 1 } else { i + 1 };
                } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

fn is_test_attr_line(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact == "#[test]" || compact == "#[core::prelude::v1::test]"
}

fn collect_valid_inv_tags(text: &str, out: &mut BTreeSet<String>) {
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
