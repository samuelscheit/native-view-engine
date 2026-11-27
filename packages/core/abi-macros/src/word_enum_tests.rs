//! Exact diagnostic and code-emission unit tests for `#[derive(WordEnum)]`.

use super::derive_tokens;
use crate::tok::lex;

#[test]
fn derive_word_enum_computes_exact_bit_widths_and_respects_default_fallback() {
    let code = derive_tokens(&lex(
        "#[repr(u8)] pub enum Facing { Unknown = 0, #[default] Back = 1, Front = 2, External = 3 }",
    ))
    .expect("valid WordEnum");
    assert!(code.contains("pub const COUNT: usize = 4;"));
    assert!(code.contains("pub const BITS: u8 = 2;"));
    assert!(code.contains("const OPT_BITS: u8 = 3;"));
    assert!(code.contains("pub const fn try_from_word("));
    assert!(code.contains("pub const fn opt_to_word("));
    assert!(code.contains("OptEncoding::PlusOne"));
    assert!(code.contains("Self::try_from_word(__nve_bits - 1)"));
    assert!(code.contains("_ => Self::Back,"));

    // 3 variants -> BITS = 2, OPT_BITS = 2 (3 + 1 = 4 states fit in 2 bits!)
    let tri = derive_tokens(&lex("#[repr(u8)] pub enum Live { Off, Polite, Assertive }"))
        .expect("3-variant WordEnum");
    assert!(tri.contains("pub const COUNT: usize = 3;"));
    assert!(tri.contains("pub const BITS: u8 = 2;"));
    assert!(tri.contains("const OPT_BITS: u8 = 2;"));
    assert!(tri.contains("_ => Self::Off,"));
}

#[test]
fn derive_word_enum_handles_1_variant_256_variants_and_rejects_257_and_qualified_default() {
    // 1-variant enum: BITS = 1, OPT_BITS = 1 (0 = None, 1 = Some(Only)).
    let one = derive_tokens(&lex("#[repr(u8)] pub enum Solo { Only = 0 }"))
        .expect("1-variant WordEnum");
    assert!(one.contains("pub const COUNT: usize = 1;"));
    assert!(one.contains("pub const BITS: u8 = 1;"));
    assert!(one.contains("const OPT_BITS: u8 = 1;"));

    // Qualified `#[other::default]` must NOT be treated as `#[default]`.
    let qual = derive_tokens(&lex(
        "#[repr(u8)] pub enum Qual { First = 0, #[other::default] Second = 1 }",
    ))
    .expect("qualified default ignored");
    assert!(qual.contains("_ => Self::First,"));

    // 256 variants: BITS = 8, OPT_BITS = 9 (256 + 1 = 257 states).
    let variants_256: String = (0..256).map(|i| format!("V{i} = {i},")).collect();
    let max_ok = derive_tokens(&lex(&format!("#[repr(u8)] pub enum E256 {{ {variants_256} }}")))
        .expect("256-variant WordEnum");
    assert!(max_ok.contains("pub const COUNT: usize = 256;"));
    assert!(max_ok.contains("pub const BITS: u8 = 8;"));
    assert!(max_ok.contains("const OPT_BITS: u8 = 9;"));

    // 257 variants: rejected cleanly.
    let variants_257: String = (0..257).map(|i| format!("V{i},")).collect();
    let err = derive_tokens(&lex(&format!("#[repr(u8)] pub enum E257 {{ {variants_257} }}")))
        .expect_err("257 variants must fail");
    assert!(
        err.contains("max 256 for #[repr(u8)]"),
        "unexpected 257-variant error: {err}"
    );
}

#[test]
fn derive_word_enum_rejects_every_invalid_shape_with_exact_diagnostics() {
    let cases: &[(&str, &str)] = &[
        (
            "#[repr(u8)] pub struct NotEnum { a: u8 }",
            "only supports enums",
        ),
        (
            "pub enum MissingRepr { A, B }",
            "requires `#[repr(u8)]`",
        ),
        (
            "#[repr(u16)] pub enum WideRepr { A = 0, B = 1 }",
            "requires `#[repr(u8)]`, found `#[repr(u16)]`",
        ),
        (
            "#[repr(u8)] pub enum Generic<T> { A }",
            "does not support generic parameters",
        ),
        (
            "#[repr(u8)] pub enum WhereClause where u8: Copy { A }",
            "does not support `where` clauses",
        ),
        (
            "#[repr(u8)] pub enum Empty {}",
            "must have at least 1 variant",
        ),
        (
            "#[repr(u8)] pub enum HasUnset { Unset = 0, On = 1 }",
            "bans `Unset` sentinel variants",
        ),
        (
            "#[repr(u8)] pub enum TupleVar { A = 0, B(u8) }",
            "must be unit-only",
        ),
        (
            "#[repr(u8)] pub enum StructVar { A { x: u8 } }",
            "must be unit-only",
        ),
        (
            "#[repr(u8)] pub enum Gap { A = 0, B = 2 }",
            "expected contiguous 1 (0..N)",
        ),
        (
            "#[repr(u8)] pub enum OneBased { A = 1, B = 2 }",
            "expected contiguous 0 (0..N)",
        ),
        (
            "#[repr(u8)] pub enum Neg { A = -1 }",
            "expected contiguous 0 (0..N)",
        ),
        (
            "#[repr(u8)] pub enum MultiDef { #[default] A, #[default] B }",
            "multiple `#[default]` variants",
        ),
    ];
    for &(src, expected) in cases {
        let err = derive_tokens(&lex(src)).expect_err(&format!("expected failure for `{src}`"));
        assert!(
            err.contains(expected),
            "input `{src}` gave `{err}`, expected substring `{expected}`"
        );
    }
}
