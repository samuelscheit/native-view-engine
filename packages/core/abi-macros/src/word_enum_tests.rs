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
