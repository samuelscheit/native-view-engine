//! 53-bit JavaScript numeric-literal safety of generated Dart and TypeScript ABI / table /
//! hash-vector files.

use nve_codegen::{GeneratedFile, generate_all_abi_files};
use nve_tables::tables::CORE_TABLE_DESCS;

fn all_files() -> Vec<GeneratedFile> {
    generate_all_abi_files(CORE_TABLE_DESCS).expect("generate every host ABI file")
}

/// Integer-valued numeric literals in JS / Dart source that are not exact JavaScript numbers:
/// `>= 2^53` (also spelled `1e20` / `9.1e15`), a value that does not even parse, or a BigInt `...n`
/// literal. Non-integral literals (`0.5`, `1e-3`) and string / comment contents are skipped.
fn inexact_js_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < bytes.len() {
        let rest = &source[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |e| e + 2);
        } else if matches!(bytes[i], b'\'' | b'"' | b'`') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                // An escape skips the whole next character, which may be multi-byte.
                let skip = usize::from(bytes[i] == b'\\');
                i += skip + source[i + skip..].chars().next().map_or(1, char::len_utf8);
            }
            i += 1;
        } else if literal_start(bytes, i) {
            let end = numeric_literal_len(rest);
            let token = &rest[..end];
            if !js_exact_integer(token) {
                out.push(token.to_owned());
            }
            i += end;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// A numeric literal starts at `i`: a digit, or a `.` before a digit (`.5`), that does not continue
/// an identifier, a number, or a member access (`a.0`, `x.5`, `1..2`).
fn literal_start(bytes: &[u8], i: usize) -> bool {
    let prev_ok = i == 0
        || !(bytes[i - 1].is_ascii_alphanumeric()
            || matches!(bytes[i - 1], b'_' | b'.' | b')' | b']'));
    match bytes[i] {
        b'0'..=b'9' => prev_ok,
        b'.' => prev_ok && bytes.get(i + 1).is_some_and(u8::is_ascii_digit),
        _ => false,
    }
}

/// Length of the numeric literal `s` starts with: digits / letters / `_`, one `.` fraction, and a
/// signed decimal exponent (`1.5e+20`).
fn numeric_literal_len(s: &str) -> usize {
    let b = s.as_bytes();
    let word = |from: usize| {
        from + s[from..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(s.len() - from)
    };
    // `.5e20` starts at its dot.
    let mut end = word(usize::from(s.starts_with('.')));
    let hex = s.starts_with("0x") || s.starts_with("0X");
    // `1.5`, and `1.e20` (an exponent right after the dot).
    let fraction = b.get(end + 1).is_some_and(u8::is_ascii_digit)
        || (matches!(b.get(end + 1), Some(b'e' | b'E'))
            && b.get(end + 2)
                .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-')));
    if !hex && !s.starts_with('.') && b.get(end) == Some(&b'.') && fraction {
        end = word(end + 1);
    }
    if !hex
        && matches!(b.get(end - 1), Some(b'e' | b'E'))
        && matches!(b.get(end), Some(b'+' | b'-'))
    {
        end = word(end + 1);
    }
    end
}

/// Whether the literal is exact in a JS double: a hex or decimal integer `< 2^53`, or not an
/// integer at all (a fraction). BigInts (`...n`) and unparsable spellings are not.
fn js_exact_integer(token: &str) -> bool {
    const LIMIT: u128 = 1 << 53;
    if token.ends_with('n') {
        return false;
    }
    let digits = token.replace('_', "");
    if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        return u128::from_str_radix(hex, 16).is_ok_and(|v| v < LIMIT);
    }
    let (mantissa, exponent) = digits.split_once(['e', 'E']).unwrap_or((&digits, "0"));
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let (Ok(m), Ok(e)) = (
        format!("{int}{frac}").parse::<u128>(),
        exponent.parse::<i32>(),
    ) else {
        return false;
    };
    // value = m * 10^(e - frac digits)
    let Ok(frac_len) = i32::try_from(frac.len()) else {
        return false;
    };
    let scale = e.saturating_sub(frac_len);
    if scale >= 0 {
        return 10u128
            .checked_pow(scale.unsigned_abs())
            .and_then(|p| m.checked_mul(p))
            .is_some_and(|v| v < LIMIT);
    }
    match 10u128.checked_pow(scale.unsigned_abs()) {
        // Integral only when the division is exact; a true fraction is not an integer literal.
        Some(p) => m % p != 0 || m / p < LIMIT,
        None => true, // 10^-39 and below: m (< 2^128) cannot be a multiple, so a fraction
    }
}

#[test]
fn the_53_bit_scanner_catches_what_it_must() {
    assert_eq!(
        inexact_js_literals("const a = 9007199254740991; // 9007199254740992"),
        Vec::<String>::new()
    );
    assert_eq!(
        inexact_js_literals("const s = '0xFFFFFFFFFFFFFFFF'; /* 1n */"),
        Vec::<String>::new()
    );
    assert_eq!(
        inexact_js_literals("x = 9007199254740992;"),
        ["9007199254740992"]
    );
    assert_eq!(
        inexact_js_literals("x = 0x20000000000000;"),
        ["0x20000000000000"]
    );
    assert_eq!(inexact_js_literals("x = 0xFFn;"), ["0xFFn"]);
    assert_eq!(
        inexact_js_literals("x = 0xFFFFFFFFFFFFFFFE;"),
        ["0xFFFFFFFFFFFFFFFE"]
    );
    assert_eq!(
        inexact_js_literals("x = 0xE0000000000000;"),
        ["0xE0000000000000"]
    );
    assert_eq!(
        inexact_js_literals("x = 0x2000000000000E + 0xEF46DB3751D8E999;"),
        ["0x2000000000000E", "0xEF46DB3751D8E999"]
    );
    assert_eq!(
        inexact_js_literals("x = 0xDEADBEEF + 1e3 + 0.5 + 1e-3 + 1.5e2 + 9.007199254740991e15;"),
        Vec::<String>::new()
    );
    assert_eq!(
        inexact_js_literals("x = 1e20 + 1E+16 + 9.1e15 + 1_0e15 + 123456789012345678.0 + 1e300;"),
        [
            "1e20",
            "1E+16",
            "9.1e15",
            "1_0e15",
            "123456789012345678.0",
            "1e300"
        ]
    );
    assert_eq!(
        inexact_js_literals("x = 0.12345678901234567890 + 1.5e-40;"),
        Vec::<String>::new()
    );
    // An exponent right after the dot, and a literal that starts with its dot.
    assert_eq!(
        inexact_js_literals("x = 1.e20 + .5e20 + .5 + 1.e3 + a.0 + x.5e20;"),
        ["1.e20", ".5e20"]
    );
    assert_eq!(
        inexact_js_literals("x = 1e2147483647 + 1e-2147483648 + 1.0e-2147483648;"),
        ["1e2147483647"]
    );
    // Escapes skip whole (multi-byte) characters; an escape at the end of input is not a panic.
    assert_eq!(
        inexact_js_literals("s = '\\é9007199254740993' + \"\\\"\" + 9007199254740993;"),
        ["9007199254740993"]
    );
    assert_eq!(inexact_js_literals("s = '\\"), Vec::<String>::new());
    assert_eq!(
        inexact_js_literals("x = 999999999999999999999999999999999999999999;").len(),
        1
    );
    assert_eq!(
        inexact_js_literals("x = a1234567890123456789 + 1.5;"),
        Vec::<String>::new()
    );
}

/// Every integer literal in Dart / TypeScript output stays below 2^53 (JavaScript numbers are
/// exact only up to there), and no 64-bit read or `BigInt` truncation is emitted.
#[test]
fn dart_and_typescript_output_is_53_bit_safe() {
    let mut scanned = 0;
    for f in all_files()
        .iter()
        .filter(|f| f.relative_path.ends_with(".dart") || f.relative_path.ends_with(".ts"))
    {
        for banned in [
            "getUint64",
            "getInt64",
            "setUint64",
            "setInt64",
            "BigInt",
            "Number(",
        ] {
            assert!(
                !f.content.contains(banned),
                "{} emits `{banned}`",
                f.relative_path
            );
        }
        let inexact = inexact_js_literals(&f.content);
        assert!(
            inexact.is_empty(),
            "{}: literals not exact in JavaScript: {inexact:?}",
            f.relative_path
        );
        scanned += 1;
    }
    assert_eq!(scanned, 8, "5 Dart + 3 TypeScript files");
}
