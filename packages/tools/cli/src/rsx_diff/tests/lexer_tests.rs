//! Compact SoA lexer: token boundaries, literal classes and error cases.

use super::super::lexer::{Lex, LexError, Tok, tokenize};

fn tokens(src: &str) -> Vec<(Tok, String)> {
    let table = tokenize(src).unwrap_or_else(|e| panic!("lex {src:?}: {}", e.as_str()));
    let lex = Lex::new(src, &table);
    (0..lex.len())
        .map(|i| (lex.kind(i), lex.text(i).to_owned()))
        .collect()
}

fn texts(src: &str) -> Vec<String> {
    tokens(src).into_iter().map(|(_, t)| t).collect()
}

#[test]
fn number_literals_keep_radix_exponent_and_suffix_rules() {
    assert_eq!(texts("0xfe-1"), ["0xfe", "-", "1"]);
    assert_eq!(texts("0xFF_00_00_FF"), ["0xFF_00_00_FF"]);
    assert_eq!(texts("1e-5 2E+3"), ["1e-5", "2E+3"]);
    assert_eq!(texts("8.0_f32"), ["8.0_f32"]);
    assert_eq!(texts("1usize-1"), ["1usize", "-", "1"]);
    assert_eq!(texts("1."), ["1."]);
    assert_eq!(texts("1.0.max(2.0)"), ["1.0", ".", "max", "(", "2.0", ")"]);
    assert_eq!(texts("0..3"), ["0", ".", ".", "3"]);
    assert_eq!(texts("t.0.1"), ["t", ".", "0.1"]);
    assert_eq!(texts("1.2.3"), ["1.2", ".", "3"]);
    assert_eq!(texts("1e5f32"), ["1e5f32"]);
    assert!(
        tokens("42 0b1 0o7 3.5")
            .iter()
            .all(|(k, _)| *k == Tok::Number)
    );
}

#[test]
fn strings_chars_lifetimes_and_raw_identifiers() {
    assert_eq!(
        tokens(r##"r#"a "q" b"# "x\"y" b"bytes" c"c" br#"r"# 'a' '\'' b'z' 'static r#type"##),
        vec![
            (Tok::RawStr, r##"r#"a "q" b"#"##.into()),
            (Tok::Str, r#""x\"y""#.into()),
            (Tok::OtherLit, "b\"bytes\"".into()),
            (Tok::OtherLit, "c\"c\"".into()),
            (Tok::OtherLit, "br#\"r\"#".into()),
            (Tok::OtherLit, "'a'".into()),
            (Tok::OtherLit, "'\\''".into()),
            (Tok::OtherLit, "b'z'".into()),
            (Tok::OtherLit, "'static".into()),
            (Tok::Ident, "r#type".into()),
        ]
    );
    // A lifetime with a brace-looking body never desynchronizes delimiter matching.
    assert_eq!(
        texts("fn f<'a>(x: &'a str) {}"),
        [
            "fn", "f", "<", "'a", ">", "(", "x", ":", "&", "'a", "str", ")", "{", "}"
        ]
    );
}

#[test]
fn comments_whitespace_and_unicode_are_skipped() {
    assert_eq!(
        texts("a /* x /* nested */ y */ b // tail\n c\u{00A0}d é"),
        ["a", "b", "c", "d", "é"]
    );
}

#[test]
fn delimiters_match_and_errors_are_reported() {
    let src = "f(a[1], { b })";
    let table = tokenize(src).unwrap_or_else(|e| panic!("{}", e.as_str()));
    let lex = Lex::new(src, &table);
    assert_eq!(lex.close(1), lex.len() - 1);
    assert!(lex.is_punct(lex.close(3), b']'));
    assert!(lex.is_punct(lex.close(7), b'}'));

    for (src, err) in [
        ("{ ( }", LexError::UnbalancedDelimiter),
        ("{", LexError::UnbalancedDelimiter),
        (")", LexError::UnbalancedDelimiter),
        ("\"open", LexError::UnterminatedString),
        ("r#\"open\"", LexError::UnterminatedString),
        ("/* open", LexError::UnterminatedBlockComment),
        ("b'", LexError::UnterminatedChar),
    ] {
        assert_eq!(tokenize(src).err(), Some(err), "{src}");
    }
}
