//! Lightweight std-only Rust lexer and braced `struct` field parser.
//!
//! Preserves 1-based line numbers when stripping comments and string/char literals so every
//! invariant diagnostic points to the exact `file:line`.

/// Strips Rust comments (`//`, nested `/* ... */`) and string/char literals while preserving
/// exact line counts so line numbers stay aligned with the original source.
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
        // Raw string literal: r#"..."# / br#"..."#
        if (bytes[i] == b'r' || (bytes[i] == b'b' && i + 1 < len && bytes[i + 1] == b'r'))
            && is_raw_string_start(bytes, i)
        {
            if bytes[i] == b'b' {
                out.push(b' ');
                i += 1;
            }
            out.push(b' ');
            i += 1; // 'r'
            let mut hashes = 0usize;
            while i < len && bytes[i] == b'#' {
                hashes += 1;
                out.push(b' ');
                i += 1;
            }
            if i < len && bytes[i] == b'"' {
                out.push(b' ');
                i += 1;
                while i < len {
                    if bytes[i] == b'"' {
                        let mut matched = 0usize;
                        while matched < hashes
                            && i + 1 + matched < len
                            && bytes[i + 1 + matched] == b'#'
                        {
                            matched += 1;
                        }
                        if matched == hashes {
                            out.extend(std::iter::repeat_n(b' ', hashes + 1));
                            i += 1 + hashes;
                            break;
                        }
                    }
                    out.push(if bytes[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
                continue;
            }
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
        // Char literal (avoid lifetimes like `'a`)
        if bytes[i] == b'\''
            && let Some(end_idx) = match_char_literal(bytes, i)
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
    String::from_utf8(out).expect("ASCII/UTF-8 preserved")
}

fn is_raw_string_start(bytes: &[u8], i: usize) -> bool {
    if i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        return false;
    }
    let mut j = i;
    if bytes[j] == b'b' {
        j += 1;
    }
    if j >= bytes.len() || bytes[j] != b'r' {
        return false;
    }
    j += 1;
    while j < bytes.len() && bytes[j] == b'#' {
        j += 1;
    }
    j < bytes.len() && bytes[j] == b'"'
}

fn match_char_literal(bytes: &[u8], start: usize) -> Option<usize> {
    let len = bytes.len();
    if start + 2 >= len {
        return None;
    }
    if bytes[start + 1] == b'\\' {
        let mut j = start + 2;
        while j < len && j < start + 12 {
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
    if bytes[start + 2] == b'\'' {
        return Some(start + 3);
    }
    None
}

/// Returns the non-test prefix of a Rust source file (everything before `#[cfg(test)]`).
pub fn production_code_prefix(src: &str) -> &str {
    let stripped = strip_rust_comments_and_strings(src);
    let mut byte_offset = 0usize;
    for (orig_line, clean_line) in src.lines().zip(stripped.lines()) {
        if clean_line.replace(' ', "").contains("#[cfg(test)]") {
            return &src[..byte_offset];
        }
        byte_offset += orig_line.len() + 1;
        if byte_offset >= src.len() {
            break;
        }
    }
    src
}

#[derive(Debug, Clone)]
pub struct ParsedStruct {
    pub name: String,
    pub is_pub: bool,
    pub line: usize,
    pub attrs: Vec<String>,
    pub fields: Vec<ParsedField>,
}

#[derive(Debug, Clone)]
pub struct ParsedField {
    pub name: String,
    pub ty: String,
    pub line: usize,
    pub attrs: Vec<String>,
}

/// Lightweight std-only Rust parser extracting braced `struct` definitions, their attributes,
/// and their named fields with exact 1-based line numbers.
pub fn parse_rust_structs(src: &str) -> Vec<ParsedStruct> {
    let clean = strip_rust_comments_and_strings(src);
    let bytes = clean.as_bytes();
    let len = bytes.len();
    let mut structs = Vec::new();
    let mut attrs: Vec<String> = Vec::new();
    let mut i = 0;
    let mut line = 1usize;

    while i < len {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'#' if i + 1 < len && bytes[i + 1] == b'[' => {
                let start = i;
                let (end, newlines) = scan_balanced(bytes, i + 1, b'[', b']');
                attrs.push(clean[start..end].to_string());
                line += newlines;
                i = end;
            }
            b'{' => {
                let (end, newlines) = scan_balanced(bytes, i, b'{', b'}');
                attrs.clear();
                line += newlines;
                i = end;
            }
            b';' => {
                attrs.clear();
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &clean[start..i];
                if word == "pub" {
                    let saved_i = i;
                    let saved_line = line;
                    skip_ws_and_vis(bytes, &mut i, &mut line);
                    if peek_word(bytes, i) == Some("struct") {
                        let struct_line = line;
                        i += "struct".len();
                        if let Some(parsed) = parse_struct_after_keyword(
                            &clean,
                            true,
                            struct_line,
                            &attrs,
                            &mut i,
                            &mut line,
                        ) {
                            structs.push(parsed);
                        }
                        attrs.clear();
                    } else {
                        i = saved_i;
                        line = saved_line;
                    }
                } else if word == "struct" {
                    let struct_line = line;
                    if let Some(parsed) = parse_struct_after_keyword(
                        &clean,
                        false,
                        struct_line,
                        &attrs,
                        &mut i,
                        &mut line,
                    ) {
                        structs.push(parsed);
                    }
                    attrs.clear();
                } else if matches!(
                    word,
                    "fn" | "impl" | "enum" | "trait" | "mod" | "use" | "const" | "static" | "type"
                ) {
                    attrs.clear();
                }
            }
            _ => i += 1,
        }
    }
    structs
}

fn parse_struct_after_keyword(
    clean: &str,
    is_pub: bool,
    struct_line: usize,
    attrs: &[String],
    i: &mut usize,
    line: &mut usize,
) -> Option<ParsedStruct> {
    let bytes = clean.as_bytes();
    skip_ws(bytes, i, line);
    let name_start = *i;
    while *i < bytes.len() && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_') {
        *i += 1;
    }
    if *i == name_start {
        return None;
    }
    let name = clean[name_start..*i].to_string();
    let mut angle_depth = 0usize;
    while *i < bytes.len() {
        match bytes[*i] {
            b'\n' => {
                *line += 1;
                *i += 1;
            }
            b'<' => {
                angle_depth += 1;
                *i += 1;
            }
            b'>' if angle_depth > 0 => {
                angle_depth -= 1;
                *i += 1;
            }
            b'{' if angle_depth == 0 => {
                let body_start = *i + 1;
                let body_start_line = *line;
                let (end, newlines) = scan_balanced(bytes, *i, b'{', b'}');
                let body = &clean[body_start..end.saturating_sub(1)];
                let fields = parse_struct_fields(body, body_start_line);
                *i = end;
                *line += newlines;
                return Some(ParsedStruct {
                    name,
                    is_pub,
                    line: struct_line,
                    attrs: attrs.to_vec(),
                    fields,
                });
            }
            b';' | b'(' if angle_depth == 0 => return None,
            _ => *i += 1,
        }
    }
    None
}

fn parse_struct_fields(body: &str, start_line: usize) -> Vec<ParsedField> {
    let mut fields = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut line = start_line;
    let mut field_attrs = Vec::new();

    while i < bytes.len() {
        skip_ws(bytes, &mut i, &mut line);
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let start = i;
            let (end, newlines) = scan_balanced(bytes, i + 1, b'[', b']');
            field_attrs.push(body[start..end].to_string());
            line += newlines;
            i = end;
            continue;
        }
        if peek_word(bytes, i) == Some("pub") {
            i += 3;
            skip_ws_and_vis(bytes, &mut i, &mut line);
        }
        let field_line = line;
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == name_start {
            if bytes[i] == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }
        let name = body[name_start..i].to_string();
        skip_ws(bytes, &mut i, &mut line);
        if i >= bytes.len() || bytes[i] != b':' {
            field_attrs.clear();
            continue;
        }
        i += 1;
        skip_ws(bytes, &mut i, &mut line);
        let ty_start = i;
        let mut depth_angle = 0usize;
        let mut depth_bracket = 0usize;
        let mut depth_paren = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    line += 1;
                    i += 1;
                }
                b'<' => {
                    depth_angle += 1;
                    i += 1;
                }
                b'>' if depth_angle > 0 => {
                    depth_angle -= 1;
                    i += 1;
                }
                b'[' => {
                    depth_bracket += 1;
                    i += 1;
                }
                b']' if depth_bracket > 0 => {
                    depth_bracket -= 1;
                    i += 1;
                }
                b'(' => {
                    depth_paren += 1;
                    i += 1;
                }
                b')' if depth_paren > 0 => {
                    depth_paren -= 1;
                    i += 1;
                }
                b',' if depth_angle == 0 && depth_bracket == 0 && depth_paren == 0 => break,
                _ => i += 1,
            }
        }
        let ty = body[ty_start..i]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        fields.push(ParsedField {
            name,
            ty,
            line: field_line,
            attrs: std::mem::take(&mut field_attrs),
        });
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
        }
    }
    fields
}

fn scan_balanced(bytes: &[u8], start: usize, open: u8, close: u8) -> (usize, usize) {
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

fn skip_ws(bytes: &[u8], i: &mut usize, line: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        if bytes[*i] == b'\n' {
            *line += 1;
        }
        *i += 1;
    }
}

fn skip_ws_and_vis(bytes: &[u8], i: &mut usize, line: &mut usize) {
    skip_ws(bytes, i, line);
    if *i < bytes.len() && bytes[*i] == b'(' {
        let (end, newlines) = scan_balanced(bytes, *i, b'(', b')');
        *i = end;
        *line += newlines;
        skip_ws(bytes, i, line);
    }
}

fn peek_word(bytes: &[u8], start: usize) -> Option<&str> {
    let mut j = start;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    if j == start {
        None
    } else {
        std::str::from_utf8(&bytes[start..j]).ok()
    }
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
