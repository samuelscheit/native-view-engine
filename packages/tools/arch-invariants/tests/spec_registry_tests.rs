//! Positive and negative unit tests for `spec_registry::check_spec_invariant_coverage`.

use nve_arch_invariants::spec_registry::check_spec_invariant_coverage;

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
        "packages/tools/arch-invariants/tests/sample.rs".to_string(),
        r#"
// [INV:only-in-helper]
fn not_a_test_helper() {
    // [INV:only-in-helper]
}

/// Enforces [INV:covered-doc-comment].
#[test]
fn real_test_one() {
    let _braces = "}}}";
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
    assert_eq!(
        spec_count, 8,
        "should find 8 valid spec tags outside code spans/fences"
    );

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
