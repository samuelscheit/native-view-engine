//! JS-53-bit integer safety of the Dart-web and TypeScript bindings `nve-codegen` emits: scanned
//! over the emitter sources (Rust) line by line, with Rust `//` and `/* */` comments outside
//! string and char literals removed, so a comment can neither exempt (`BigInt`, `32n`) nor flag a
//! line while emitted text (`"https://…"`, `"// …\nconst int …"`) stays scanned.

use std::borrow::Cow;
use std::ops::Range;

use crate::Violation;
use crate::hygiene::hex_literal_digit_counts;

/// `0x{..}` widths that print at most 32 bits for a `u32` operand. A width is a minimum, so this is
/// safe only because every value emitters spell this way is a `u32` (`interface_id`; the ABI
/// validator rejects any `const` above `u32::MAX`).
const NARROW_HEX: &[&str] = &["02x", "04x", "08x"];
/// The same widths in Rust's alternate form (`{v:#010x}` prints `0x` itself).
const NARROW_ALT_HEX: &[&str] = &["#04x", "#06x", "#010x"];

pub fn scan_dart_53bit_safety(rel: &str, src: &str, out: &mut Vec<Violation>) {
    for (idx, line, code) in code_lines(src) {
        let code = code.as_ref();
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
    for (idx, line, code) in code_lines(src) {
        let code = code.as_ref();
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

/// Where the lexer is at the start of a line: Rust strings and block comments span lines.
#[derive(Clone, Copy)]
enum LexState {
    Code,
    Str,
    /// `r#"…"#` with this many `#`.
    RawStr(usize),
    /// `/* … */`, nested this deep.
    Block(usize),
}

/// Every line of `src` with its comments removed (each block comment becomes one space). Borrowed
/// unless a line has a comment before more code, so clean lines never allocate.
fn code_lines(src: &str) -> impl Iterator<Item = (usize, &str, Cow<'_, str>)> {
    let mut state = LexState::Code;
    src.lines().enumerate().map(move |(idx, line)| {
        let comments = comment_ranges(line, &mut state);
        let code = match comments.as_slice() {
            [] => Cow::Borrowed(line),
            [only] if only.end == line.len() => Cow::Borrowed(&line[..only.start]),
            ranges => {
                let mut code = String::with_capacity(line.len());
                let mut at = 0;
                for range in ranges {
                    code.push_str(&line[at..range.start]);
                    code.push(' ');
                    at = range.end;
                }
                code.push_str(&line[at..]);
                Cow::Owned(code)
            }
        };
        (idx, line, code)
    })
}

/// The byte ranges of `line` inside Rust comments, advancing `state` over the line.
fn comment_ranges(line: &str, state: &mut LexState) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut comments = Vec::new();
    let mut comment_start = matches!(state, LexState::Block(_)).then_some(0);
    let mut i = 0;
    while i < bytes.len() {
        let (b, next) = (bytes[i], bytes.get(i + 1).copied());
        match *state {
            LexState::Code => match (b, next) {
                (b'/', Some(b'/')) => {
                    comments.push(i..bytes.len());
                    return comments;
                }
                (b'/', Some(b'*')) => {
                    comment_start = Some(i);
                    *state = LexState::Block(1);
                    i += 1;
                }
                (b'"', _) => *state = LexState::Str,
                (b'r', Some(b'"' | b'#')) if starts_literal(bytes, i) => {
                    let hashes = bytes[i + 1..].iter().take_while(|&&h| h == b'#').count();
                    if bytes.get(i + 1 + hashes) == Some(&b'"') {
                        *state = LexState::RawStr(hashes);
                        i += hashes + 1;
                    }
                }
                // A char literal (`'"'`, `'\''`); any other `'` starts a lifetime or label.
                (b'\'', Some(b'\\')) => {
                    let close = bytes.get(i + 3..).and_then(|rest| rest.iter().position(|&c| c == b'\''));
                    i += close.map_or(1, |p| p + 3);
                }
                (b'\'', Some(_)) => {
                    let width = line[i + 1..].chars().next().map_or(1, char::len_utf8);
                    if bytes.get(i + 1 + width) == Some(&b'\'') {
                        i += width + 1;
                    }
                }
                _ => {}
            },
            LexState::Str => match b {
                b'\\' => i += 1,
                b'"' => *state = LexState::Code,
                _ => {}
            },
            LexState::RawStr(hashes) => {
                let closes = b == b'"' && bytes[i + 1..].iter().take_while(|&&h| h == b'#').count() >= hashes;
                if closes {
                    *state = LexState::Code;
                    i += hashes;
                }
            }
            LexState::Block(depth) => match (b, next) {
                (b'/', Some(b'*')) => {
                    *state = LexState::Block(depth + 1);
                    i += 1;
                }
                (b'*', Some(b'/')) => {
                    i += 1;
                    if depth == 1 {
                        *state = LexState::Code;
                        comments.push(comment_start.take().unwrap_or(0)..i + 1);
                    } else {
                        *state = LexState::Block(depth - 1);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    if let Some(start) = comment_start {
        comments.push(start..bytes.len());
    }
    comments
}

/// Whether the `r` at `i` starts a raw string literal: `r"`, `r#"`, byte `br"`, C `cr"`, not `bar"`.
fn starts_literal(bytes: &[u8], i: usize) -> bool {
    let is_ident = |at: Option<usize>| at.is_some_and(|at| bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_');
    let before = i.checked_sub(1);
    !is_ident(before) || (before.is_some_and(|b| matches!(bytes[b], b'b' | b'c')) && !is_ident(i.checked_sub(2)))
}

/// A hex literal of >= 14 digits (> 53 bits) cannot be represented exactly by a JS Number.
fn has_wide_hex_literal(code: &str) -> bool {
    hex_literal_digit_counts(code).any(|digits| digits >= 14)
}

/// Whether a format placeholder may print a hex number wider than 32 bits:
/// - `0x{..}` / `0X{..}` without a narrow width (`0x{h}`, `0x{h:X}`, `0x{h:016X}`);
/// - an alternate-form hex placeholder without one, whatever its fill / alignment
///   (`{h:#x}`, `{h:#018X}`, `{h:>#018X}`);
/// - a narrow one with more digits, placeholders, or `_` digit separators next to it
///   (`0x{hi:08X}{lo:08X}`, `0x{hi:08X}00000000`, `0x10000000{lo:08X}`, `0x{hi:08X}_{lo:08X}`).
fn has_wide_hex_format(code: &str) -> bool {
    let bytes = code.as_bytes();
    let one_of = |spec: &str, list: &[&str]| list.iter().any(|s| s.eq_ignore_ascii_case(spec));
    let is_digit_part = |b: &u8| b.is_ascii_hexdigit() || *b == b'_';
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
        // A `{` before the `}` (a Rust block around an emitted string) is not this placeholder's:
        // restart at it, so the placeholder inside is still checked.
        if let Some(inner) = code[open + 1..close].find('{') {
            at = open + 1 + inner;
            continue;
        }
        at = close + 1;
        let spec = code[open + 1..close].rsplit_once(':').map_or("", |(_, spec)| spec);
        // `0x` right before the placeholder, or before digits that lead into it.
        let lead = bytes[..open].iter().rev().take_while(|b| is_digit_part(b)).count();
        let digits_start = open - lead;
        let prefixed = digits_start >= 2
            && bytes[digits_start - 2] == b'0'
            && bytes[digits_start - 1].eq_ignore_ascii_case(&b'x');
        let alt_hex = spec.bytes().last().is_some_and(|b| b.eq_ignore_ascii_case(&b'x'));
        let narrow = match spec.find('#') {
            Some(hash) if alt_hex => one_of(&spec[hash..], NARROW_ALT_HEX),
            _ if prefixed => lead == 0 && one_of(spec, NARROW_HEX),
            _ => continue, // not a hex placeholder
        };
        let continued = bytes.get(at).is_some_and(|b| is_digit_part(b) || *b == b'{');
        if !narrow || continued {
            return true;
        }
    }
    false
}
