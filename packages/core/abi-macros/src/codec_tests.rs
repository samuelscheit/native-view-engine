//! Exact diagnostic unit tests for `#[derive(NveCodec)]` validation and code emission.

use super::{derive_tokens, Delim, Tok};

fn lex(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut pos = 0;
    lex_until(bytes, &mut pos, None)
}

fn lex_until(bytes: &[u8], pos: &mut usize, end: Option<u8>) -> Vec<Tok> {
    let mut out = Vec::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        if b.is_ascii_whitespace() {
            *pos += 1;
            continue;
        }
        if end == Some(b) {
            *pos += 1;
            break;
        }
        match b {
            b'(' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Paren, lex_until(bytes, pos, Some(b')'))));
            }
            b'[' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Bracket, lex_until(bytes, pos, Some(b']'))));
            }
            b'{' => {
                *pos += 1;
                out.push(Tok::Group(Delim::Brace, lex_until(bytes, pos, Some(b'}'))));
            }
            b'"' => {
                let start = *pos;
                *pos += 1;
                while *pos < bytes.len() {
                    if bytes[*pos] == b'\\' {
                        *pos += 2;
                    } else if bytes[*pos] == b'"' {
                        *pos += 1;
                        break;
                    } else {
                        *pos += 1;
                    }
                }
                out.push(Tok::Literal(std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned()));
            }
            c if c.is_ascii_digit() => {
                let start = *pos;
                while *pos < bytes.len()
                    && (bytes[*pos].is_ascii_alphanumeric() || bytes[*pos] == b'_')
                {
                    *pos += 1;
                }
                out.push(Tok::Literal(std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned()));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = *pos;
                while *pos < bytes.len()
                    && (bytes[*pos].is_ascii_alphanumeric() || bytes[*pos] == b'_')
                {
                    *pos += 1;
                }
                out.push(Tok::Ident(std::str::from_utf8(&bytes[start..*pos]).unwrap().to_owned()));
            }
            c => {
                *pos += 1;
                let joint = *pos < bytes.len()
                    && !bytes[*pos].is_ascii_whitespace()
                    && !matches!(bytes[*pos], b'(' | b')' | b'[' | b']' | b'{' | b'}');
                out.push(Tok::Punct(c as char, joint));
            }
        }
    }
    out
}

fn assert_err_contains(src: &str, expected: &str) {
    let err = derive_tokens(&lex(src)).expect_err(&format!("expected error for:\n{src}"));
    assert!(
        err.contains(expected),
        "error `{err}` did not contain `{expected}` for:\n{src}"
    );
}

#[test]
fn derive_nve_codec_rejects_every_invalid_shape_with_exact_diagnostics() {
    assert_err_contains(
        "struct BadGeneric<T>(T);",
        "does not support generics on `BadGeneric`",
    );
    assert_err_contains(
        "struct BadWhere where u8: Copy { x: u8 }",
        "does not support `where` clauses on `BadWhere`",
    );
    assert_err_contains(
        "struct BadTupleWhere(u8) where u8: Copy;",
        "does not support `where` clauses on `BadTupleWhere`",
    );
    assert_err_contains(
        "enum BadEnumWhere where u8: Copy { A }",
        "does not support `where` clauses on `BadEnumWhere`",
    );
    assert_err_contains(
        "struct MissingSemicolon",
        "expected struct body or `;` on `MissingSemicolon`",
    );
    assert_err_contains(
        "struct FieldAttr { #[nve_codec(skip)] a: u8 }",
        "does not support field-level `#[nve_codec(...)]` attributes",
    );
    assert_err_contains(
        "struct TupleFieldAttr(#[nve_codec(skip)] u8);",
        "does not support field-level `#[nve_codec(...)]` attributes",
    );
    assert_err_contains(
        "enum VariantFieldAttr { #[nve_codec(tag = 1)] A { #[nve_codec(skip)] x: u8 } }",
        "does not support field-level `#[nve_codec(...)]` attributes",
    );
    assert_err_contains(
        "#[repr(u8)] enum Conflicting { #[nve_codec(tag = 1)] A(u8) = 2 }",
        "cannot specify both `#[nve_codec(tag = ...)]` and `= discriminant`",
    );
    assert_err_contains(
        "#[repr(u8)] enum UnitTagAttr { #[nve_codec(tag = 1)] A }",
        "unit-only enum `UnitTagAttr` forbids `#[nve_codec(tag = ...)]`",
    );
    assert_err_contains(
        "enum OutOfRange { #[nve_codec(tag = 256)] TooBig(u8) }",
        "tag 256 out of range for `u8`",
    );
    assert_err_contains(
        "enum DupTag { #[nve_codec(tag = 1)] A(u8), #[nve_codec(tag = 1)] B(u16) }",
        "duplicate tag 1 on `A` and `B`",
    );
    assert_err_contains(
        "#[repr(i8)] enum BadRepr { A = 1 }",
        "does not support `#[repr(i8)]`",
    );
    assert_err_contains(
        "#[repr(u16)] enum BadDataRepr { A(u32) }",
        "data-carrying enum `BadDataRepr` uses u8 variant tags",
    );
    assert_err_contains(
        "#[nve_codec(unknown_key = \"x\")] struct BadKey(u8);",
        "unknown #[nve_codec] item key `unknown_key`",
    );
    assert_err_contains(
        "enum EmptyEnum {}",
        "must have at least one variant",
    );
    assert_err_contains(
        "#[repr(i32)] enum ImplicitOverflow { Max = 2147483647i32, Next }",
        "tag 2147483648 out of range for `i32`",
    );
}

#[test]
fn derive_nve_codec_emits_hygienic_locals_and_i64_raw_discriminant() {
    let code = derive_tokens(&lex("#[repr(i32)] enum Signed { Neg = -1i32, Pos = 1 }"))
        .expect("signed enum should derive");
    assert!(code.contains("let __nve_raw = __nve_r.read_i32()?;"));
    assert!(code.contains("raw: ::core::primitive::i64::from(__nve_raw)"));

    let data_code = derive_tokens(&lex("enum Data { #[nve_codec(tag = 1)] V { raw: u8, other: u8 } }"))
        .expect("data enum with raw/other fields should derive");
    assert!(data_code.contains("__nve_other =>"));
    assert!(data_code.contains("__f_raw"));
    assert!(data_code.contains("__f_other"));
}
