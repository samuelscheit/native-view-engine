//! Positive and negative unit tests for `spec_registry::check_spec_invariant_coverage`.

use nve_arch_invariants::spec_registry::{
    check_spec_invariant_coverage, extract_test_invariant_guards, GuardStrength,
};

#[test]
fn unit_spec_invariant_registry_positive_and_negative() {
    let spec_docs = vec![(
        "docs/BACKLOG.md".to_string(),
        r#"
- Must enforce [INV:covered-doc-comment], [INV:covered-body-comment], [INV:cfg-test-mod-guard], [INV:ratcheted-guard], [INV:mixed-strict-ratchet], [INV:split-strict-ratchet], and [INV:cross-file-split-ratchet].
- Also backed by ast-grep [INV:sg-tagged-inv], [INV:sg-rule-id], and [INV:sg-ratchet-collision].
- Missing check [INV:missing-guard], non-test [INV:only-in-helper], ignored [INV:ignored-guard], should-panic [INV:should-panic-guard].
- Dishonest guards: empty [INV:empty-test], unforced staged [INV:unforced-staged], mixed unforced [INV:mixed-unforced], and comment-spoofed [INV:comment-spoofed].
- AST spoofing: block comment [INV:block-comment-spoof], raw string [INV:raw-string-spoof], cfg-any fn [INV:cfg-any-fn], cfg-any mod [INV:cfg-any-mod], cfg-feature fn [INV:cfg-feature-fn].
- Recursive cfg_attr spoofing: file [INV:file-cfg-attr-dead], mod [INV:mod-cfg-attr-dead], stmt [INV:stmt-cfg-attr-dead], nested ignore [INV:nested-cfg-attr-ignore], qualified ignore [INV:qualified-cfg-attr-ignore], qualified should-panic [INV:qualified-cfg-attr-panic], edition ignore [INV:edition-prelude-ignore], edition should-panic [INV:edition-prelude-panic].
- Dead-branch & tautology spoofing: if-false [INV:if-false-spoof], uncalled closure [INV:closure-spoof], nested fn [INV:nested-fn-spoof], cfg-any stmt [INV:cfg-any-stmt], assert-true [INV:assert-true-tautology], assert-eq-lit [INV:assert-eq-tautology], suffixed-lit [INV:suffixed-lit-tautology], empty-slice [INV:empty-slice-tautology].
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
        r####"
// [INV:only-in-helper]
fn not_a_test_helper(v: &[Violation]) {
    assert_no_violations("helper only", v);
    // [INV:only-in-helper]
}

/// Enforces [INV:covered-doc-comment].
#[test]
fn real_test_one() {
    let _braces = "}}}";
    let violations = scan_workspace();
    // Also checks [INV:covered-body-comment]
    assert_no_violations("real check", &violations);
}

#[cfg(test)]
mod nested_active_mod {
    /// Enforces [INV:cfg-test-mod-guard].
    #[test]
    fn inside_cfg_test_mod() {
        let count = count_issues();
        assert_eq!(count, 0);
    }
}

/// Enforces [INV:ratcheted-guard].
#[test]
fn real_ratchet_test() {
    let violations = scan_workspace();
    assert_staged_ratchet(Stage::Post1B1D, "ratcheted check", 0, &violations);
}

/// Enforces [INV:mixed-strict-ratchet].
#[test]
fn mixed_strict_and_ratchet_test() {
    let active = scan_active();
    let allowlisted = scan_allowlisted();
    assert_no_violations("zero new", &active);
    assert_staged_ratchet(Stage::Post1B1D, "legacy ceiling", 5, &allowlisted);
}

/// Enforces [INV:split-strict-ratchet] (strict half).
#[test]
fn split_strict_half_test() {
    let active = scan_active();
    assert_no_violations("zero new", &active);
}

/// Enforces [INV:split-strict-ratchet] (ratcheted half — must dominate via weakest-link `.min`).
#[test]
fn split_ratchet_half_test() {
    let allowlisted = scan_allowlisted();
    assert_staged_ratchet(Stage::Post1B1D, "legacy ceiling", 2, &allowlisted);
}

/// Enforces [INV:cross-file-split-ratchet] (strict in sample.rs; ratcheted in second file).
#[test]
fn cross_file_strict_half_test() {
    let active = scan_active();
    assert_no_violations("zero new", &active);
}

/// Enforces [INV:empty-test].
#[test]
fn empty_noop_test() {}

/// Enforces [INV:unforced-staged].
#[test]
fn unforced_staged_only_test() {
    let violations = scan_workspace();
    assert_staged_violations(Stage::Post1B1D, "unforced noop", &violations);
}

/// Enforces [INV:mixed-unforced].
#[test]
fn mixed_unforced_staged_test() {
    let violations = scan_workspace();
    assert_no_violations("other thing", &violations);
    assert_staged_violations(Stage::Post1B1D, "unforced noop", &violations);
}

/// Enforces [INV:comment-spoofed].
#[test]
fn comment_spoofed_test() {
    // assert_no_violations("fake", &violations);
    let _ = "assert!(ok)";
}

/// Enforces [INV:ignored-guard].
#[test]
#[ignore = "disabled"]
fn ignored_test() {
    let violations = scan_workspace();
    assert_no_violations("ignored", &violations);
}

/// Enforces [INV:should-panic-guard].
#[test]
#[should_panic]
fn should_panic_test() {
    let violations = scan_workspace();
    assert_no_violations("should panic", &violations);
}

/*
/// Enforces [INV:block-comment-spoof].
#[test]
fn spoofed_inside_block_comment() {
    let violations = scan_workspace();
    assert_no_violations("fake block comment", &violations);
}
*/

const FAKE_TEST_IN_RAW_STRING: &str = r#"
/// Enforces [INV:raw-string-spoof].
#[test]
fn spoofed_inside_raw_string() {
    let violations = scan_workspace();
    assert_no_violations("fake raw string", &violations);
}
"#;

/// Enforces [INV:cfg-any-fn].
#[cfg(any())]
#[test]
fn disabled_cfg_any_fn() {
    let violations = scan_workspace();
    assert_no_violations("disabled fn", &violations);
}

#[cfg(any())]
mod disabled_cfg_any_mod {
    /// Enforces [INV:cfg-any-mod].
    #[test]
    fn inside_disabled_mod() {
        let violations = scan_workspace();
        assert_no_violations("disabled mod", &violations);
    }
}

/// Enforces [INV:cfg-feature-fn].
#[cfg(feature = "never-enabled")]
#[test]
fn feature_gated_fn() {
    let violations = scan_workspace();
    assert_no_violations("feature gated", &violations);
}

/// Enforces [INV:if-false-spoof].
#[test]
fn dead_if_false_branch_test() {
    if false {
        assert!(false);
    }
}

/// Enforces [INV:closure-spoof].
#[test]
fn uncalled_closure_test() {
    let _ = || assert!(false);
}

/// Enforces [INV:nested-fn-spoof].
#[test]
fn uncalled_nested_fn_test() {
    fn never_called() {
        assert!(false);
    }
}

/// Enforces [INV:cfg-any-stmt].
#[test]
fn cfg_any_stmt_test() {
    #[cfg(any())]
    {
        let violations = scan_workspace();
        assert_no_violations("dead cfg block", &violations);
    }
    #[cfg(any())]
    assert!(false);
}

/// Enforces [INV:assert-true-tautology].
#[test]
fn assert_true_tautology_test() {
    assert!(true);
    assert!(!false);
    assert!(1 == 1);
}

/// Enforces [INV:assert-eq-tautology].
#[test]
fn assert_eq_tautology_test() {
    assert_eq!(1, 1);
    assert_ne!(1, 2);
}

/// Enforces [INV:suffixed-lit-tautology].
#[test]
fn suffixed_literal_tautology_test() {
    assert_eq!(1usize, 1);
    assert!(1usize == 1);
    assert_eq!(01usize, 1);
    assert_eq!(1.0f32, 1.0);
    assert_eq!(1f32, 1.0);
    assert_eq!(1.0f32, 1.00);
    assert_eq!(1e0, 1.0);
}

#[cfg_attr(all(), cfg(any()))]
mod dead_cfg_attr_mod {
    /// Enforces [INV:mod-cfg-attr-dead].
    #[test]
    fn inside_dead_cfg_attr_mod() {
        let violations = scan_workspace();
        assert_no_violations("dead cfg_attr mod", &violations);
    }
}

/// Enforces [INV:stmt-cfg-attr-dead].
#[test]
fn dead_cfg_attr_stmt_test() {
    #[cfg_attr(all(), cfg(any()))]
    {
        let violations = scan_workspace();
        assert_no_violations("dead cfg_attr block", &violations);
    }
    #[cfg_attr(all(), cfg(any()))]
    assert!(false);
}

/// Enforces [INV:nested-cfg-attr-ignore].
#[test]
#[cfg_attr(all(), cfg_attr(all(), ignore))]
fn nested_cfg_attr_ignore_test() {
    let violations = scan_workspace();
    assert_no_violations("nested cfg_attr ignore", &violations);
}

/// Enforces [INV:qualified-cfg-attr-ignore].
#[test]
#[cfg_attr(all(), core::prelude::v1::ignore)]
fn qualified_cfg_attr_ignore_test() {
    let violations = scan_workspace();
    assert_no_violations("qualified cfg_attr ignore", &violations);
}

/// Enforces [INV:qualified-cfg-attr-panic].
#[test]
#[cfg_attr(all(), std::prelude::v1::should_panic)]
fn qualified_cfg_attr_should_panic_test() {
    let violations = scan_workspace();
    assert_no_violations("qualified cfg_attr should_panic", &violations);
}

/// Enforces [INV:edition-prelude-ignore].
#[test]
#[cfg_attr(all(), core::prelude::rust_2024::ignore)]
fn edition_prelude_ignore_test() {
    let violations = scan_workspace();
    assert_no_violations("edition prelude ignore", &violations);
}

/// Enforces [INV:edition-prelude-panic].
#[test]
#[std::prelude::rust_2021::should_panic]
fn edition_prelude_panic_test() {
    let violations = scan_workspace();
    assert_no_violations("edition prelude should_panic", &violations);
}

/// Enforces [INV:empty-slice-tautology].
#[test]
fn empty_slice_tautology_test() {
    assert_no_violations("x", &[]);
    assert_staged_ratchet(Stage::Post1B1D, "y", 0, &[]);
}
"####
        .to_string(),
    ),
    (
        "packages/tools/arch-invariants/tests/dead_file.rs".to_string(),
        r####"
#![cfg_attr(all(), cfg(any()))]

/// Enforces [INV:file-cfg-attr-dead].
#[test]
fn inside_dead_cfg_attr_file() {
    let violations = scan_workspace();
    assert_no_violations("dead file", &violations);
}
"####
        .to_string(),
    ),
    (
        "packages/tools/arch-invariants/tests/split_ratchet.rs".to_string(),
        r####"
/// Enforces [INV:cross-file-split-ratchet] (ratcheted half in another file).
#[test]
fn cross_file_ratchet_half_test() {
    let allowlisted = scan_allowlisted();
    assert_staged_ratchet(Stage::Post1B1D, "cross-file ceiling", 1, &allowlisted);
}

/// Enforces [INV:sg-ratchet-collision] via ratchet (must dominate strict ast-grep via `.min`).
#[test]
fn sg_ratchet_collision_test() {
    let allowlisted = scan_allowlisted();
    assert_staged_ratchet(Stage::Post1B1D, "sg collision ceiling", 1, &allowlisted);
}
"####
        .to_string(),
    )];

    let sg_rules = vec![(
        "lints/ast-grep/rules/sample.yml".to_string(),
        r#"# [INV:sg-tagged-inv] [INV:sg-ratchet-collision]
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
    assert_eq!(guards.get("cfg-test-mod-guard"), Some(&GuardStrength::Enforced));
    assert_eq!(guards.get("ratcheted-guard"), Some(&GuardStrength::Ratcheted));
    assert_eq!(
        guards.get("mixed-strict-ratchet"),
        Some(&GuardStrength::Ratcheted),
        "mixed strict + ratchet test must downgrade to GuardStrength::Ratcheted"
    );
    assert_eq!(
        guards.get("split-strict-ratchet"),
        Some(&GuardStrength::Ratcheted),
        "split strict + ratchet #[test] functions in the same file must combine via weakest-link .min"
    );
    assert!(
        extract_test_invariant_guards(&guard_sources[1].1).is_empty(),
        "file-level `#![cfg_attr(all(), cfg(any()))]` must disqualify every test in the file"
    );

    let rejected = [
        "only-in-helper",
        "ignored-guard",
        "should-panic-guard",
        "empty-test",
        "unforced-staged",
        "mixed-unforced",
        "comment-spoofed",
        "block-comment-spoof",
        "raw-string-spoof",
        "cfg-any-fn",
        "cfg-any-mod",
        "cfg-feature-fn",
        "file-cfg-attr-dead",
        "mod-cfg-attr-dead",
        "stmt-cfg-attr-dead",
        "nested-cfg-attr-ignore",
        "qualified-cfg-attr-ignore",
        "qualified-cfg-attr-panic",
        "edition-prelude-ignore",
        "edition-prelude-panic",
        "if-false-spoof",
        "closure-spoof",
        "nested-fn-spoof",
        "cfg-any-stmt",
        "assert-true-tautology",
        "assert-eq-tautology",
        "suffixed-lit-tautology",
        "empty-slice-tautology",
    ];
    for id in rejected {
        assert!(
            !guards.contains_key(id),
            "dishonest/spoofed guard `[INV:{id}]` must NOT be accepted: {guards:?}"
        );
    }

    let report = check_spec_invariant_coverage(&spec_docs, &guard_sources, &sg_rules, &sg_tests);
    assert_eq!(
        report.spec_count, 40,
        "should find 40 valid spec tags outside code spans/fences"
    );
    assert_eq!(
        report.covered.get("covered-doc-comment"),
        Some(&GuardStrength::Enforced)
    );
    assert_eq!(
        report.covered.get("covered-body-comment"),
        Some(&GuardStrength::Enforced)
    );
    assert_eq!(
        report.covered.get("cfg-test-mod-guard"),
        Some(&GuardStrength::Enforced)
    );
    assert_eq!(
        report.covered.get("sg-tagged-inv"),
        Some(&GuardStrength::Enforced)
    );
    assert_eq!(
        report.covered.get("sg-rule-id"),
        Some(&GuardStrength::Enforced)
    );
    assert_eq!(
        report.covered.get("ratcheted-guard"),
        Some(&GuardStrength::Ratcheted)
    );
    assert_eq!(
        report.covered.get("mixed-strict-ratchet"),
        Some(&GuardStrength::Ratcheted)
    );
    assert_eq!(
        report.covered.get("split-strict-ratchet"),
        Some(&GuardStrength::Ratcheted)
    );
    assert_eq!(
        report.covered.get("cross-file-split-ratchet"),
        Some(&GuardStrength::Ratcheted),
        "split strict + ratchet #[test] functions across files must combine via weakest-link .min"
    );
    assert_eq!(
        report.covered.get("sg-ratchet-collision"),
        Some(&GuardStrength::Ratcheted),
        "strict ast-grep + ratcheted Rust test must combine via weakest-link .min"
    );
    assert_eq!(report.enforced_count(), 5);
    assert_eq!(report.ratcheted_count(), 5);

    let reasons: Vec<&str> = report.violations.iter().map(|v| v.reason.as_str()).collect();
    for expected in [
        "[INV:missing-guard]",
        "[INV:only-in-helper]",
        "[INV:ignored-guard]",
        "[INV:should-panic-guard]",
        "[INV:empty-test]",
        "[INV:unforced-staged]",
        "[INV:mixed-unforced]",
        "[INV:comment-spoofed]",
        "[INV:block-comment-spoof]",
        "[INV:raw-string-spoof]",
        "[INV:cfg-any-fn]",
        "[INV:cfg-any-mod]",
        "[INV:cfg-feature-fn]",
        "[INV:file-cfg-attr-dead]",
        "[INV:mod-cfg-attr-dead]",
        "[INV:stmt-cfg-attr-dead]",
        "[INV:nested-cfg-attr-ignore]",
        "[INV:qualified-cfg-attr-ignore]",
        "[INV:qualified-cfg-attr-panic]",
        "[INV:edition-prelude-ignore]",
        "[INV:edition-prelude-panic]",
        "[INV:if-false-spoof]",
        "[INV:closure-spoof]",
        "[INV:nested-fn-spoof]",
        "[INV:cfg-any-stmt]",
        "[INV:assert-true-tautology]",
        "[INV:assert-eq-tautology]",
        "[INV:suffixed-lit-tautology]",
        "[INV:empty-slice-tautology]",
        "[INV:untested-sg]",
        "[INV:Bad_ID]",
    ] {
        assert!(
            reasons.iter().any(|r| r.contains(expected)),
            "must flag {expected}: {reasons:?}"
        );
    }
    assert_eq!(
        report.violations.len(),
        31,
        "only uncovered/spoofed/malformed tags should flag: {:?}",
        report.violations
    );
}
