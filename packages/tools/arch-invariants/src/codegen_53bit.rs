//! JS-53-bit integer safety of the Dart-web and TypeScript bindings `nve-codegen` emits: scanned
//! over the emitter sources (Rust) line by line, with trailing `//` comments stripped so a comment
//! can neither exempt (`BigInt`) nor flag a line.

use crate::Violation;
use crate::hygiene::hex_literal_digit_counts;

/// `0x{..}` widths that print at most 32 bits for a `u32` operand. A width is a minimum, so this is
/// safe only because every value emitters spell this way is a `u32` (`interface_id`; the ABI
/// validator rejects any `const` above `u32::MAX`).
const NARROW_HEX: &[&str] = &["02x", "04x", "08x"];
/// The same widths in Rust's alternate form (`{v:#010x}` prints `0x` itself).
const NARROW_ALT_HEX: &[&str] = &["#04x", "#06x", "#010x"];

pub fn scan_dart_53bit_safety(rel: &str, src: &str, out: &mut Vec<Violation>) {
    for (idx, line) in src.lines().enumerate() {
        let code = strip_line_comment(line);
        let is_dart_decl = code.contains("const int ")
            || code.contains("final int ")
            || (code.contains("const ") && code.contains('='));
        if is_dart_decl
            && !code.contains("BigInt")
            && (has_wide_hex_literal(code) || has_wide_hex_format(code))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                line,
                "Unsafe 64-bit integer constant in Dart-web binding (JS Number precision is 53 bits); use `(lo: u32, hi: u32)` or `BigInt`",
            ));
        }
    }
}

pub fn scan_ts_53bit_safety(rel: &str, src: &str, out: &mut Vec<Violation>) {
    for (idx, line) in src.lines().enumerate() {
        let code = strip_line_comment(line);
        let unsafe_ts_64 = (code.contains("Number(")
            && (code.contains("getBigUint64") || code.contains("getBigInt64")))
            || code.contains("4294967296")
            || (code.contains("<< 32") && !code.contains("BigInt") && !code.contains("32n"));
        if unsafe_ts_64 {
            out.push(Violation::new(
                rel,
                idx + 1,
                line,
                "Unsafe 64-bit integer arithmetic in TypeScript/JS binding (JS Number precision is 53 bits); use `(lo: u32, hi: u32)` or `BigInt`",
            ));
        }
    }
}

/// `line` up to its first `//` (an emitted target comment inside a string literal is cut too).
fn strip_line_comment(line: &str) -> &str {
    line.split_once("//").map_or(line, |(code, _)| code)
}

/// A hex literal of >= 14 digits (> 53 bits) cannot be represented exactly by a JS Number.
fn has_wide_hex_literal(code: &str) -> bool {
    hex_literal_digit_counts(code).any(|digits| digits >= 14)
}

/// Whether a format placeholder may print a hex number wider than 32 bits: `0x{..}` / `0X{..}`
/// without a narrow width (`0x{h}`, `0x{h:X}`, `0x{h:016X}`), an alternate-form hex placeholder
/// without one (`{h:#x}`, `{h:#018X}`), or a narrow one followed by more digits
/// (`0x{hi:08X}{lo:08X}`, `0x{hi:08X}00000000`).
fn has_wide_hex_format(code: &str) -> bool {
    let bytes = code.as_bytes();
    let one_of = |spec: &str, list: &[&str]| list.iter().any(|s| s.eq_ignore_ascii_case(spec));
    let mut at = 0;
    while let Some(offset) = code[at..].find('{') {
        let open = at + offset;
        if bytes.get(open + 1) == Some(&b'{') {
            at = open + 2; // `{{` is a literal brace
            continue;
        }
        let Some(len) = code[open..].find('}') else {
            return false;
        };
        let close = open + len;
        at = close + 1;
        let spec = code[open + 1..close].rsplit_once(':').map_or("", |(_, spec)| spec);
        let prefixed = open >= 2 && bytes[open - 2] == b'0' && bytes[open - 1].eq_ignore_ascii_case(&b'x');
        let narrow = if prefixed {
            one_of(spec, NARROW_HEX)
        } else if spec.starts_with('#') && spec.bytes().last().is_some_and(|b| b.eq_ignore_ascii_case(&b'x')) {
            one_of(spec, NARROW_ALT_HEX)
        } else {
            continue; // not a hex placeholder
        };
        let continued = bytes.get(at).is_some_and(|b| b.is_ascii_hexdigit() || *b == b'{');
        if !narrow || continued {
            return true;
        }
    }
    false
}
