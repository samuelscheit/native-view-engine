//! Lightweight std-only Rust lexer, raw-string span extractor, and `#[cfg(test)]` item stripper.
//!
//! Every transformation preserves exact byte alignment for newlines (`\n`) so 1-based line
//! numbers stay accurate even on CRLF files.

pub use crate::rust_items::{parse_rust_enums, parse_rust_structs, ParsedStruct};

/// Strips Rust comments (`//`, nested `/* ... */`) and string/c-string/raw-string/char literals
/// while preserving exact `\n` counts and byte positions.
pub fn strip_rust_comments_and_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;

    while i < len {
        // Line comment
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
            continue;
        }
        // Nested block comment
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            out.extend_from_slice(b"  ");
            i += 2;
            while i < len && depth > 0 {
                if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    out.extend_from_slice(b"  ");
                    i += 2;
                } else {
                    out.push(if bytes[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
            }
            continue;
        }
        // Raw string literal: r"...", r#"..."#, br#"..."#, cr#"..."#
        if let Some((prefix_len, hashes)) = match_raw_string_prefix(bytes, i) {
            let start_quote = i + prefix_len;
            out.extend(std::iter::repeat_n(b' ', prefix_len + 1));
            i = start_quote + 1;
            while i < len {
                if bytes[i] == b'"' && has_n_hashes(bytes, i + 1, hashes) {
                    out.extend(std::iter::repeat_n(b' ', hashes + 1));
                    i += 1 + hashes;
                    break;
                }
                out.push(if bytes[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            continue;
        }
        // Byte/C regular string prefix (`b"..."`, `c"..."`)
        if (bytes[i] == b'b' || bytes[i] == b'c')
            && i + 1 < len
            && bytes[i + 1] == b'"'
            && !is_ident_before(bytes, i)
        {
            out.push(b' ');
            i += 1;
            continue;
        }
        // Regular string literal
        if bytes[i] == b'"' {
            out.push(b' ');
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    out.push(b' ');
                    out.push(if bytes[i + 1] == b'\n' { b'\n' } else { b' ' });
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    out.push(b' ');
                    i += 1;
                    break;
                }
                out.push(if bytes[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            continue;
        }
        // Byte char literal `b'x'`
        if bytes[i] == b'b'
            && i + 1 < len
            && bytes[i + 1] == b'\''
            && !is_ident_before(bytes, i)
            && let Some(end_idx) = match_char_literal(src, i + 1)
        {
            for &b in &bytes[i..end_idx] {
                out.push(if b == b'\n' { b'\n' } else { b' ' });
            }
            i = end_idx;
            continue;
        }
        // Char literal (handles `'\''`, `'\u{1234}'`, multi-byte UTF-8 `'—'`, ignores lifetimes `'a`)
        if bytes[i] == b'\''
            && let Some(end_idx) = match_char_literal(src, i)
        {
            for &b in &bytes[i..end_idx] {
                out.push(if b == b'\n' { b'\n' } else { b' ' });
            }
            i = end_idx;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).expect("UTF-8 preserved")
}

/// Returns `(start_line, end_line, line_span)` for every raw string literal (`r"..."`, `r#"..."#`,
/// `br#"..."#`, `cr#"..."#`) outside comments and regular strings.
pub fn extract_raw_string_spans(src: &str) -> Vec<(usize, usize, usize)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line = 1usize;

    while i < len {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            i += 2;
            while i < len && depth > 0 {
                if bytes[i] == b'\n' {
                    line += 1;
                    i += 1;
                } else if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if let Some((prefix_len, hashes)) = match_raw_string_prefix(bytes, i) {
            let start_line = line;
            i += prefix_len + 1;
            while i < len {
                if bytes[i] == b'\n' {
                    line += 1;
                    i += 1;
                } else if bytes[i] == b'"' && has_n_hashes(bytes, i + 1, hashes) {
                    i += 1 + hashes;
                    break;
                } else {
                    i += 1;
                }
            }
            spans.push((start_line, line, line - start_line + 1));
            continue;
        }
        if bytes[i] == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    if bytes[i + 1] == b'\n' {
                        line += 1;
                    }
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'\''
            && let Some(end_idx) = match_char_literal(src, i)
        {
            i = end_idx;
            continue;
        }
        i += 1;
    }
    spans
}

fn is_ident_before(bytes: &[u8], i: usize) -> bool {
    i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')
}

fn match_raw_string_prefix(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    if is_ident_before(bytes, i) {
        return None;
    }
    let mut j = i;
    if bytes[j] == b'b' || bytes[j] == b'c' {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'r' {
        return None;
    }
    j += 1;
    let hash_start = j;
    while j < bytes.len() && bytes[j] == b'#' {
        j += 1;
    }
    if j < bytes.len() && bytes[j] == b'"' {
        Some((j - i, j - hash_start))
    } else {
        None
    }
}

fn has_n_hashes(bytes: &[u8], start: usize, n: usize) -> bool {
    start + n <= bytes.len() && bytes[start..start + n].iter().all(|&b| b == b'#')
}

fn match_char_literal(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    if start + 2 >= len || bytes[start] != b'\'' {
        return None;
    }
    if bytes[start + 1] == b'\\' {
        if start + 3 < len && bytes[start + 2] == b'\'' && bytes[start + 3] == b'\'' {
            return Some(start + 4);
        }
        let mut j = start + 2;
        while j < len && j < start + 14 {
            if bytes[j] == b'\'' {
                return Some(j + 1);
            }
            if bytes[j] == b'\n' {
                return None;
            }
            j += 1;
        }
        return None;
    }
    let ch = src[start + 1..].chars().next()?;
    if ch == '\'' || ch == '\n' || ch == '\r' {
        return None;
    }
    let after_ch = start + 1 + ch.len_utf8();
    if after_ch < len && bytes[after_ch] == b'\'' {
        Some(after_ch + 1)
    } else {
        None
    }
}

/// Blanks every item guarded by an outer `#[cfg(...)]` attribute that requires `test`
/// (e.g. `#[cfg(test)]`, `#[cfg(all(test, ...))]`, `#[cfg(any(test, ...))]`, multiline
/// `#[cfg(\n  test\n)]`), while leaving `#[cfg(not(test))]` and ALL subsequent production items
/// completely intact. Preserves exact `\n` alignment (CRLF-safe).
pub fn strip_cfg_test_items(src: &str) -> String {
    let clean = strip_rust_comments_and_strings(src);
    let clean_bytes = clean.as_bytes();
    let mut out = src.as_bytes().to_vec();
    let len = clean_bytes.len();
    let mut i = 0;
    let mut pending_attrs: Vec<(usize, usize)> = Vec::new();

    while i < len {
        let b = clean_bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#' && i + 1 < len && clean_bytes[i + 1] == b'[' {
            let attr_start = i;
            let (attr_end, _) = scan_balanced(clean_bytes, i + 1, b'[', b']');
            pending_attrs.push((attr_start, attr_end));
            i = attr_end;

            if is_cfg_test_attr(&clean[attr_start..attr_end]) {
                let mut item_pos = i;
                loop {
                    while item_pos < len && clean_bytes[item_pos].is_ascii_whitespace() {
                        item_pos += 1;
                    }
                    if item_pos + 1 < len
                        && clean_bytes[item_pos] == b'#'
                        && clean_bytes[item_pos + 1] == b'['
                    {
                        let (next_end, _) = scan_balanced(clean_bytes, item_pos + 1, b'[', b']');
                        item_pos = next_end;
                    } else {
                        break;
                    }
                }
                let item_end = scan_rust_item_end(clean_bytes, item_pos);
                let blank_start = pending_attrs.first().map(|(s, _)| *s).unwrap_or(attr_start);
                blank_range(&mut out, blank_start, item_end);
                pending_attrs.clear();
                i = item_end;
            }
            continue;
        }
        pending_attrs.clear();
        i += 1;
    }

    String::from_utf8(out).expect("UTF-8 preserved")
}

fn is_cfg_test_attr(attr: &str) -> bool {
    let inner = attr
        .trim()
        .strip_prefix("#[")
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim);
    let Some(inner) = inner else {
        return false;
    };
    let Some(rest) = inner.strip_prefix("cfg") else {
        return false;
    };
    let rest = rest.trim_start();
    if !rest.starts_with('(') || !rest.ends_with(')') {
        return false;
    }
    cfg_expr_contains_positive_test(&rest[1..rest.len() - 1])
}

fn cfg_expr_contains_positive_test(expr: &str) -> bool {
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &expr[start..i];
            if ident == "not" {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'(' {
                    let (end, _) = scan_balanced(bytes, j, b'(', b')');
                    i = end;
                    continue;
                }
            } else if ident == "test" {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= bytes.len() || bytes[j] != b'=' {
                    return true;
                }
            }
        } else {
            i += 1;
        }
    }
    false
}

fn scan_rust_item_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                paren += 1;
                i += 1;
            }
            b')' => {
                paren = paren.saturating_sub(1);
                i += 1;
            }
            b'[' => {
                bracket += 1;
                i += 1;
            }
            b']' => {
                bracket = bracket.saturating_sub(1);
                i += 1;
            }
            b'{' if paren == 0 && bracket == 0 => {
                let (end, _) = scan_balanced(bytes, i, b'{', b'}');
                return end;
            }
            b';' if paren == 0 && bracket == 0 => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn blank_range(buf: &mut [u8], start: usize, end: usize) {
    let clamped_end = end.min(buf.len());
    for b in &mut buf[start..clamped_end] {
        if *b != b'\n' && *b != b'\r' {
            *b = b' ';
        }
    }
}

pub fn scan_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> (usize, usize) {
    let mut depth = 0usize;
    let mut i = start;
    let mut newlines = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            newlines += 1;
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return (i + 1, newlines);
            }
        }
        i += 1;
    }
    (i, newlines)
}

pub fn contains_ident(line: &str, needle: &str) -> bool {
    let bytes = line.as_bytes();
    let nlen = needle.len();
    if nlen == 0 || bytes.len() < nlen {
        return false;
    }
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(needle) {
        let abs = pos + idx;
        let before_ok =
            abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let after = abs + nlen;
        let after_ok = after == bytes.len()
            || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            return true;
        }
        pos = abs + 1;
    }
    false
}
