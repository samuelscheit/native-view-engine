//! Mechanical spec-to-test invariant registry guard.
//!
//! Ensures every promised architectural invariant tagged `[INV:<id>]` in `AGENTS.md`,
//! `docs/BACKLOG.md`, or `docs/architecture.md` is backed by either:
//! - a non-ignored `#[test]` in `packages/tools/cli/tests/arch_invariants/` referencing
//!   `[INV:<id>]` in its doc/leading comments or body, or
//! - a unit-tested `ast-grep` rule under `lints/ast-grep/rules/` whose `id` matches `<id>`
//!   (or carries `# [INV:<id>]`) and is exercised in `lints/ast-grep/tests/`.

use std::collections::{BTreeMap, BTreeSet};

use crate::rust_lex::scan_balanced;
use crate::support::{assert_no_violations, rel_path, repo_index, walk_files_no_symlinks, Violation};

const SPEC_DOCS: &[&str] = &["AGENTS.md", "docs/BACKLOG.md", "docs/architecture.md"];

const NON_GUARD_TEST_FILES: &[&str] = &[
    "packages/tools/cli/tests/arch_invariants/unit_tests.rs",
    "packages/tools/cli/tests/arch_invariants/rpc_server_purity_tests.rs",
    "packages/tools/cli/tests/arch_invariants/spec_registry.rs",
];

#[test]
fn spec_invariant_ids_have_mechanical_guards() {
    let idx = repo_index();
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
    for rel in idx.rs_files_under("packages/tools/cli/tests/arch_invariants") {
        if NON_GUARD_TEST_FILES.contains(&rel.as_str()) {
            continue;
        }
        if let Some(content) = idx.read_text(&rel) {
            guard_sources.push((rel, content));
        }
    }

    let ast_grep_rules = collect_yaml_files(&idx.root, "lints/ast-grep/rules");
    let ast_grep_tests = collect_yaml_files(&idx.root, "lints/ast-grep/tests");

    let (spec_count, violations) =
        check_spec_invariant_coverage(&spec_docs, &guard_sources, &ast_grep_rules, &ast_grep_tests);
    assert!(
        spec_count > 0,
        "expected at least one `[INV:<id>]` tag in SPEC_DOCS so the registry guard cannot pass vacuously"
    );
    assert_no_violations(
        "Every promised `[INV:<id>]` in AGENTS.md / docs/BACKLOG.md / docs/architecture.md has a backing mechanical guard",
        &violations,
    );
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
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yml" | "yaml")
            )
        })
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
            spec_refs.entry(id).or_default().push((rel.clone(), line_no, snippet));
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
                        "Promised invariant `[INV:{id}]` has no backing non-ignored `#[test]` in `arch_invariants/` or unit-tested `ast-grep` rule"
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

/// Extracts `[INV:<id>]` tags attached to non-ignored `#[test]` functions (in contiguous
/// leading comments/attributes or inside the test function body).
pub fn extract_test_covered_invariant_ids(src: &str) -> BTreeSet<String> {
    let mut covered = BTreeSet::new();
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
            if j >= i && (t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("async fn ")) {
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

        // Collect tags from the contiguous header comments.
        for &hdr in &lines[header_start..fn_idx] {
            collect_valid_inv_tags(hdr, &mut covered);
        }

        // Collect tags inside the `#[test]` function body `{ ... }`.
        let search_from = line_offsets.get(fn_idx).copied().unwrap_or(src.len());
        if let Some(brace_rel) = src[search_from..].find('{') {
            let brace_abs = search_from + brace_rel;
            let (end_abs, _) = scan_balanced(bytes, brace_abs, b'{', b'}');
            collect_valid_inv_tags(&src[brace_abs..end_abs], &mut covered);
            // Advance past the function body.
            let next_line = line_offsets.partition_point(|&off| off < end_abs);
            i = next_line.max(fn_idx + 1);
        } else {
            i = fn_idx + 1;
        }
    }

    covered
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
    if val.is_empty() { None } else { Some(val) }
}

#[test]
fn unit_spec_invariant_registry_positive_and_negative() {
    let spec_docs = vec![(
        "docs/BACKLOG.md".to_string(),
        r#"
- Must enforce [INV:covered-doc-comment] and [INV:covered-body-comment].
- Also backed by ast-grep [INV:sg-tagged-inv] and [INV:sg-rule-id].
- Missing check [INV:missing-guard] and non-test [INV:only-in-helper] and ignored [INV:ignored-guard].
- Untested ast-grep [INV:untested-sg].
- Malformed tag [INV:Bad_ID].
- Inline syntax example `[INV:inline-example]` and `[INV:<id>]` are ignored.
```md
[INV:inside-code-fence]
```
"#
        .to_string(),
    )];

    let guard_sources = vec![(
        "packages/tools/cli/tests/arch_invariants/sample.rs".to_string(),
        r#"
// [INV:only-in-helper]
fn not_a_test_helper() {
    // [INV:only-in-helper]
}

/// Enforces [INV:covered-doc-comment].
#[test]
fn real_test_one() {
    // Also checks [INV:covered-body-comment]
}

/// Enforces [INV:ignored-guard].
#[test]
#[ignore = "disabled"]
fn ignored_test() {}
"#
        .to_string(),
    )];

    let sg_rules = vec![(
        "lints/ast-grep/rules/sample.yml".to_string(),
        r#"# [INV:sg-tagged-inv]
id: sg-rule-id
language: rust
---
# [INV:untested-sg]
id: untested-sg-rule
language: rust
"#
        .to_string(),
    )];

    let sg_tests = vec![(
        "lints/ast-grep/tests/sample-test.yml".to_string(),
        "id: sg-rule-id\nvalid: []\n".to_string(),
    )];

    let (spec_count, violations) =
        check_spec_invariant_coverage(&spec_docs, &guard_sources, &sg_rules, &sg_tests);
    assert_eq!(spec_count, 8, "should find 8 valid spec tags outside code spans/fences");

    let reasons: Vec<&str> = violations.iter().map(|v| v.reason.as_str()).collect();
    assert!(
        reasons.iter().any(|r| r.contains("[INV:missing-guard]")),
        "must flag uncovered spec invariant: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("[INV:only-in-helper]")),
        "must NOT count tags on non-#[test] helpers: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("[INV:ignored-guard]")),
        "must NOT count tags on #[ignore] tests: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("[INV:untested-sg]")),
        "must NOT count untested ast-grep rules: {reasons:?}"
    );
    assert!(
        reasons.iter().any(|r| r.contains("[INV:Bad_ID]")),
        "must flag malformed invariant tags: {reasons:?}"
    );
    assert_eq!(
        violations.len(),
        5,
        "covered tests, tested ast-grep rules, inline code spans, and fenced blocks must have 0 violations: {violations:?}"
    );
}
