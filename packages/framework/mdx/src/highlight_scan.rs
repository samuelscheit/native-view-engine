//! Low-level lexical scanning helpers for [`crate::highlight`].

pub(crate) fn advance_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

pub(crate) fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

pub(crate) fn scan_ident(bytes: &[u8], mut i: usize, allow_dash: bool) -> usize {
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'_'
            || (allow_dash && bytes[i] == b'-'))
    {
        i += 1;
    }
    i
}

pub(crate) fn is_pascal_type(word: &str) -> bool {
    let bytes = word.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_uppercase()
        && bytes[1..].iter().any(u8::is_ascii_lowercase)
}

pub(crate) fn scan_number(bytes: &[u8], mut i: usize) -> usize {
    if bytes.get(i) == Some(&b'0')
        && matches!(bytes.get(i + 1), Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
    {
        i += 2;
        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}

pub(crate) fn scan_quoted(line: &str, start: usize, quote: u8, escapes: bool) -> usize {
    let bytes = line.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if escapes && bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    bytes.len()
}

pub(crate) fn try_scan_char_literal(line: &str, start: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = start + 1;
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'\\' {
        i += 1;
        while i < bytes.len() && i < start + 12 {
            if bytes[i] == b'\'' {
                return Some(i + 1);
            }
            i += 1;
        }
        return None;
    }
    let ch = line[i..].chars().next()?;
    let next = i + ch.len_utf8();
    if bytes.get(next) == Some(&b'\'') {
        Some(next + 1)
    } else {
        None
    }
}

pub(crate) fn scan_triple_quote(line: &str, mut i: usize, quote: u8) -> (usize, bool) {
    let bytes = line.as_bytes();
    while i + 2 < bytes.len() {
        if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
            return (i + 3, true);
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    (bytes.len(), false)
}

pub(crate) fn scan_backtick(line: &str, mut i: usize) -> (usize, bool) {
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return (i + 1, true);
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    (bytes.len(), false)
}

pub(crate) fn scan_block_comment(
    line: &str,
    mut i: usize,
    mut depth: usize,
    nested: bool,
) -> (usize, usize) {
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if nested && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return (i, 0);
            }
            continue;
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    (bytes.len(), depth)
}

pub(crate) fn scan_rust_attribute(line: &str, start: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = start;
    let mut depth = 0usize;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

pub(crate) fn detect_rust_raw_string(rest: &str) -> Option<(usize, usize)> {
    let bytes = rest.as_bytes();
    let offset = if bytes.starts_with(b"br") {
        2
    } else if bytes.first() == Some(&b'r') {
        1
    } else {
        return None;
    };
    let mut hashes = 0usize;
    while bytes.get(offset + hashes) == Some(&b'#') {
        hashes += 1;
    }
    if bytes.get(offset + hashes) == Some(&b'"') {
        Some((offset + hashes + 1, hashes))
    } else {
        None
    }
}

pub(crate) fn scan_rust_raw_string(line: &str, mut i: usize, hashes: usize) -> (usize, bool) {
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"'
            && i + hashes < bytes.len()
            && bytes[i + 1..=i + hashes].iter().all(|&b| b == b'#')
        {
            return (i + 1 + hashes, true);
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
    (bytes.len(), false)
}

pub(crate) fn scan_dollar_var(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start + 1) == Some(&b'{') {
        let mut i = start + 2;
        while i < bytes.len() {
            if bytes[i] == b'}' {
                return i + 1;
            }
            i += 1;
        }
        return bytes.len();
    }
    if bytes.get(start + 1).is_some_and(|&b| is_ident_start(b)) {
        return scan_ident(bytes, start + 1, false);
    }
    start + 1
}
