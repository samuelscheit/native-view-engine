//! String literal helpers: Rust unescaping, `rsx!` format-brace handling, source positions.

use super::lexer::LexError;

/// Unescapes a Rust normal (`"..."`) or raw (`r#"..."#`) string literal into its value, rejecting
/// every escape Rust rejects (`\q`, `\x80`, `\u{D800}`, ...).
pub(super) fn unescape_string_literal(raw: &str, is_raw: bool) -> Result<String, LexError> {
    if is_raw {
        return string_lit_inner(raw, true)
            .map(str::to_owned)
            .ok_or(LexError::UnterminatedString);
    }
    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(LexError::UnterminatedString)?;
    if !inner.contains('\\') {
        return Ok(inner.to_owned());
    }

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next().ok_or(LexError::InvalidEscape)? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\n' | '\r' => {
                // Line continuation: skip leading whitespace on the next line.
                while chars
                    .next_if(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
                    .is_some()
                {}
            }
            'x' => {
                let hi = chars.next().and_then(|c| c.to_digit(16));
                let lo = chars.next().and_then(|c| c.to_digit(16));
                let (Some(hi), Some(lo)) = (hi, lo) else {
                    return Err(LexError::InvalidEscape);
                };
                // `\x` is ASCII-only in `&str` literals.
                let byte = hi * 16 + lo;
                if byte > 0x7F {
                    return Err(LexError::InvalidEscape);
                }
                out.push(char::from(byte as u8));
            }
            'u' => {
                if chars.next() != Some('{') {
                    return Err(LexError::InvalidEscape);
                }
                let (mut code, mut digits) = (0u32, 0u32);
                loop {
                    match chars.next() {
                        Some('}') if digits > 0 => break,
                        Some('_') if digits > 0 => {}
                        Some(c) if digits < 6 => {
                            code = code * 16 + c.to_digit(16).ok_or(LexError::InvalidEscape)?;
                            digits += 1;
                        }
                        _ => return Err(LexError::InvalidEscape),
                    }
                }
                out.push(char::from_u32(code).ok_or(LexError::InvalidEscape)?);
            }
            _ => return Err(LexError::InvalidEscape),
        }
    }
    Ok(out)
}

/// Borrows the contents of a string literal that needs no unescaping (raw, or escape-free normal);
/// `None` when unescaping would be required.
pub(super) fn string_lit_inner(raw: &str, is_raw: bool) -> Option<&str> {
    if is_raw {
        let open = raw.find('"')?;
        let close = raw.rfind('"')?;
        return (close > open).then(|| &raw[open + 1..close]);
    }
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains('\\')).then_some(inner)
}

/// Returns `true` if `s` contains any unescaped `{` or `}` format interpolation brace.
/// Escaped `{{` and `}}` represent literal braces and do not count as interpolation.
pub(super) fn has_interpolation_braces(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b @ (b'{' | b'}') => {
                if bytes.get(i + 1) != Some(&b) {
                    return true;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    false
}

/// Applies `rsx!` format-string brace escapes (`{{` -> `{`, `}}` -> `}`) to a literal that
/// [`has_interpolation_braces`] already proved interpolation-free: the value the app renders.
pub(super) fn unescape_format_braces(mut s: String) -> String {
    if s.contains("{{") || s.contains("}}") {
        s = s.replace("{{", "{").replace("}}", "}");
    }
    s
}

/// 1-based `(line, column)` of byte `offset` in `src`, matching `line!()` / `column!()` (column
/// counts chars).
pub(super) fn line_col(src: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(src.len());
    let before = &src.as_bytes()[..offset];
    let line = 1 + before.iter().filter(|&&b| b == b'\n').count();
    let line_start = before
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    let column = 1 + src.get(line_start..offset).map_or(0, |s| s.chars().count());
    (line as u32, column as u32)
}
