//! Unit tests of the host-runtime drift guard's declaration parsing (`host_runtimes::decl`):
//! every way a mirror can drift must produce a problem, and comments can never satisfy a check.

use nve_arch_invariants::host_runtimes::decl::{
    Syntax, block, check_block, contains_hex_token, declarations, declared_values,
};

fn set(entries: &[(&str, i64)]) -> Vec<(String, i64)> {
    entries.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
}

#[test]
fn exact_sets_pass_in_every_syntax() {
    let status = set(&[("Ok", 0), ("Err", 1), ("InvalidArg", 2)]);
    let cases = [
        (
            Syntax::Assign,
            "enum class S : int32_t { Ok = 0, Err = 1, InvalidArg = 2 };",
        ),
        (
            Syntax::Assign,
            "public enum S: Int32 {\n  case ok = 0, err = 1, invalidArg = 2\n",
        ),
        (
            Syntax::Call,
            "public enum S { OK(0), ERR(1), INVALID_ARG(2); final int code; S(int code) {",
        ),
        (
            Syntax::Colon,
            "export const S = { Ok: 0, Err: 1, InvalidArg: 2 } as const;",
        ),
        (Syntax::Ordered, "enum S { ok, err, invalidArg }"),
        (
            Syntax::Ordered,
            "public enum S {\n    OK, ERR, INVALID_ARG;\n    public static final int X = 3;",
        ),
    ];
    for (syntax, body) in cases {
        assert_eq!(
            check_block(&status, syntax, "", &[], body),
            Vec::<String>::new(),
            "{body}"
        );
    }
    let prefixed = "typedef enum K { NVE_FIELD_U32 = 0, NVE_FIELD_BOOL = 1 } K;";
    let kinds = set(&[("U32", 0), ("Bool", 1)]);
    assert!(check_block(&kinds, Syntax::Assign, "nve_field_", &[], prefixed).is_empty());
    let renamed = "enum K { u32, boolean }";
    assert!(check_block(&kinds, Syntax::Ordered, "", &[("Bool", "boolean")], renamed).is_empty());
}

#[test]
fn digit_boundaries_reject_longer_numbers() {
    let words = set(&[("MaxWords", 32)]);
    let problems = check_block(&words, Syntax::Assign, "", &[], "{ MaxWords = 320 }");
    assert_eq!(problems.len(), 1, "{problems:?}");
    let err = set(&[("Ok", 0), ("Err", 1)]);
    assert!(!check_block(&err, Syntax::Assign, "", &[], "{ Ok = 0, Err = 10 }").is_empty());
    assert!(!check_block(&err, Syntax::Call, "", &[], "{ OK(0), ERR(10) }").is_empty());
    assert!(!check_block(&err, Syntax::Colon, "", &[], "{ Ok: 0, Err: 1x }").is_empty());
    assert_eq!(
        declarations("{ A = 32x, B = 4 }", Syntax::Assign),
        set(&[("B", 4)])
    );
}

#[test]
fn stale_extra_and_duplicate_entries_are_reported() {
    let status = set(&[("Ok", 0), ("Err", 1)]);
    let extra = check_block(
        &status,
        Syntax::Call,
        "",
        &[],
        "{ OK(0), ERR(1), RETIRED(2); }",
    );
    assert!(
        extra.iter().any(|p| p.contains("stale extra `retired`")),
        "{extra:?}"
    );
    let trailing = check_block(
        &status,
        Syntax::Ordered,
        "",
        &[],
        "enum S { ok, err, retired }",
    );
    assert!(
        trailing.iter().any(|p| p.contains("stale extra")),
        "{trailing:?}"
    );
    let reordered = check_block(&status, Syntax::Ordered, "", &[], "enum S { err, ok }");
    assert_eq!(reordered.len(), 2, "{reordered:?}");
    let missing = check_block(&status, Syntax::Colon, "", &[], "{ Ok: 0 }");
    assert!(
        missing.iter().any(|p| p.contains("missing `err`")),
        "{missing:?}"
    );
    let twice = check_block(
        &status,
        Syntax::Assign,
        "",
        &[],
        "{ Ok = 0, Err = 1, ok = 0 }",
    );
    assert!(
        twice.iter().any(|p| p.contains("declared twice")),
        "{twice:?}"
    );
}

#[test]
fn comments_and_comparisons_never_count_as_declarations() {
    let status = set(&[("Ok", 0), ("Err", 1)]);
    let commented = "{ Ok = 0, /* Err = 1 */ // Err = 1\n }";
    assert!(!check_block(&status, Syntax::Assign, "", &[], commented).is_empty());
    assert_eq!(
        declarations("{ if (x == 3) { Err = 1 } }", Syntax::Assign),
        set(&[("Err", 1)])
    );

    assert_eq!(
        declared_values("// MAX_DEPTH = 64\nconst other = 1;", "MAX_DEPTH"),
        Vec::<i64>::new()
    );
    assert_eq!(
        declared_values("/** MAX_DEPTH = 64 */ x", "MAX_DEPTH"),
        Vec::<i64>::new()
    );
    assert_eq!(
        declared_values("static final int MAX_DEPTH = 640;", "MAX_DEPTH"),
        vec![640]
    );
    assert_eq!(
        declared_values("int kMAX_DEPTH = 64; int MAX_DEPTH_X = 64;", "MAX_DEPTH"),
        Vec::<i64>::new()
    );
    assert_eq!(
        declared_values("if (MAX_DEPTH == 64) {}", "MAX_DEPTH"),
        Vec::<i64>::new()
    );
    assert_eq!(
        declared_values("#define NVE_MAX_TABLE_WORDS 32u", "NVE_MAX_TABLE_WORDS"),
        vec![32]
    );
    assert_eq!(
        declared_values("#define NVE_MAX_TABLE_WORDS 320u", "NVE_MAX_TABLE_WORDS"),
        vec![320]
    );
    assert_eq!(
        declared_values("let MAX = 64\nlet MAX = 65", "MAX"),
        vec![64, 65]
    );
}

#[test]
fn digests_match_as_whole_tokens_outside_comments() {
    let digest = "0x149aa44972cdae00";
    assert!(contains_hex_token(
        "(\"seq1024\", seq(1024), 0, 0x149A_A449_72CD_AE00),",
        digest
    ));
    assert!(
        !contains_hex_token("x = 0x149AA44972CDAE001;", digest),
        "prefix of a longer number"
    );
    assert!(
        !contains_hex_token("// expected 0x149AA44972CDAE00\nx = 1;", digest),
        "comment only"
    );
    let halves = "key64(0x72cdae00,0x149aa449)";
    assert!(contains_hex_token(
        "const Key64(0x72CDAE00, 0x149AA449)",
        halves
    ));
    assert!(!contains_hex_token(
        "/* const Key64(0x72CDAE00, 0x149AA449) */",
        halves
    ));
}

#[test]
fn blocks_are_brace_balanced_and_ordered_lists_reject_prepends() {
    let java = "public enum S {\n  OK(0), ERR(1);\n  static S of(int c) { for (S s : values()) { if (s.code == c) return s; } return null; }\n}\nclass Next { X(9) }";
    let body = block(java, "public enum S").unwrap_or_default();
    assert!(body.ends_with("}\n}"), "{body}");
    assert!(!body.contains("Next"));
    let status = set(&[("Ok", 0), ("Err", 1)]);
    assert!(check_block(&status, Syntax::Call, "", &[], body).is_empty());
    // A stale value after a nested method is still inside the balanced block and is reported.
    let late = "enum S { OK(0), ERR(1); void f() { if (x) { y(); } } } // RETIRED(2) is a comment";
    assert!(
        check_block(
            &status,
            Syntax::Call,
            "",
            &[],
            block(late, "enum S").unwrap_or_default()
        )
        .is_empty()
    );
    let late_real = "enum S { OK(0), ERR(1); void f() { if (x) { y(); } } RETIRED(2); }";
    assert!(
        !check_block(
            &status,
            Syntax::Call,
            "",
            &[],
            block(late_real, "enum S").unwrap_or_default()
        )
        .is_empty()
    );
    let prepended = check_block(
        &status,
        Syntax::Ordered,
        "",
        &[],
        "enum S { added, ok, err }",
    );
    assert!(
        prepended.iter().any(|p| p.contains("stale extra `added`")),
        "{prepended:?}"
    );
    assert_eq!(block("enum S { a, b", "enum S"), None, "unbalanced block");
    assert_eq!(block("no marker here", "enum S"), None);
}
