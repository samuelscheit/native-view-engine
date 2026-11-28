//! Positive and negative unit tests for `spec_registry::check_spec_invariant_coverage`.

use nve_arch_invariants::spec_registry::{
    check_spec_invariant_coverage, extract_test_invariant_guards, GuardStrength,
};

#[test]
fn unit_spec_invariant_registry_positive_and_negative() {
    let spec_docs = vec![(
        "docs/BACKLOG.md".to_string(),
        r#"
- Must enforce [INV:covered-doc-comment], [INV:covered-body-comment], and [INV:ratcheted-guard].
- Also backed by ast-grep [INV:sg-tagged-inv] and [INV:sg-rule-id].
- Missing check [INV:missing-guard], non-test [INV:only-in-helper], ignored [INV:ignored-guard].
- Dishonest guards: empty [INV:empty-test], unforced staged [INV:unforced-staged], mixed unforced [INV:mixed-unforced], and comment-spoofed [INV:comment-spoofed].
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
    assert!(true);
    // [INV:only-in-helper]
}

/// Enforces [INV:covered-doc-comment].
#[test]
fn real_test_one() {
    let _braces = "}}}";
    // Also checks [INV:covered-body-comment]
    assert_no_violations("real check", &[]);
}

/// Enforces [INV:ratcheted-guard].
#[test]
fn real_ratchet_test() {
    assert_staged_ratchet(Stage::Post1B1D, "ratcheted check", 0, &[]);
}

/// Enforces [INV:empty-test].
#[test]
fn empty_noop_test() {}

/// Enforces [INV:unforced-staged].
#[test]
fn unforced_staged_only_test() {
    assert_staged_violations(Stage::Post1B1D, "unforced noop", &[]);
}

/// Enforces [INV:mixed-unforced].
#[test]
fn mixed_unforced_staged_test() {
    assert_no_violations("other thing", &[]);
    assert_staged_violations(Stage::Post1B1D, "unforced noop", &[]);
}

/// Enforces [INV:comment-spoofed].
#[test]
fn comment_spoofed_test() {
    // assert_no_violations("fake", &[]);
    let _ = "assert!(true)";
}

/// Enforces [INV:ignored-guard].
#[test]
#[ignore = "disabled"]
fn ignored_test() {
    assert_no_violations("ignored", &[]);
}
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

    let guards = extract_test_invariant_guards(&guard_sources[0].1);
    assert_eq!(guards.get("covered-doc-comment"), Some(&GuardStrength::Enforced));
    assert_eq!(guards.get("covered-body-comment"), Some(&GuardStrength::Enforced));
    assert_eq!(guards.get("ratcheted-guard"), Some(&GuardStrength::Ratcheted));
    assert!(!guards.contains_key("empty-test"));
    assert!(!guards.contains_key("unforced-staged"));
    assert!(!guards.contains_key("mixed-unforced"));
    assert!(!guards.contains_key("comment-spoofed"));

    let (spec_count, violations) =
        check_spec_invariant_coverage(&spec_docs, &guard_sources, &sg_rules, &sg_tests);
    assert_eq!(
        spec_count, 13,
        "should find 13 valid spec tags outside code spans/fences"
    );

    let reasons: Vec<&str> = violations.iter().map(|v| v.reason.as_str()).collect();
    for expected in [
        "[INV:missing-guard]",
        "[INV:only-in-helper]",
        "[INV:ignored-guard]",
        "[INV:empty-test]",
        "[INV:unforced-staged]",
        "[INV:mixed-unforced]",
        "[INV:comment-spoofed]",
        "[INV:untested-sg]",
        "[INV:Bad_ID]",
    ] {
        assert!(
            reasons.iter().any(|r| r.contains(expected)),
            "must flag {expected}: {reasons:?}"
        );
    }
    assert_eq!(
        violations.len(),
        9,
        "enforced/ratcheted tests, tested ast-grep rules, inline code spans, and fenced blocks must have 0 violations: {violations:?}"
    );
}
