//! Lightweight std-only Rust `struct` (braced + tuple) and `enum` item parsers, descending into
//! inline `mod <name> { ... }` blocks and distinguishing bare `pub` from restricted `pub(...)`.

use crate::rust_lex::{scan_balanced, strip_rust_comments_and_strings};

#[derive(Debug, Clone)]
pub struct ParsedStruct {
    pub name: String,
    pub is_pub: bool,
    pub is_tuple: bool,
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

#[derive(Debug, Clone)]
pub struct ParsedEnum {
    pub name: String,
    pub is_pub: bool,
    pub line: usize,
}

/// Parses braced and tuple `struct` definitions across top-level and inline `mod foo { ... }`
/// blocks, distinguishing bare `pub` (`is_pub = true`) from restricted `pub(...)` (`is_pub = false`).
pub fn parse_rust_structs(src: &str) -> Vec<ParsedStruct> {
    let clean = strip_rust_comments_and_strings(src);
    let mut structs = Vec::new();
    parse_structs_in_slice(&clean, 1, &mut structs);
    structs
}

fn parse_structs_in_slice(clean: &str, start_line: usize, structs: &mut Vec<ParsedStruct>) {
    let bytes = clean.as_bytes();
    let len = bytes.len();
    let mut attrs: Vec<String> = Vec::new();
    let mut i = 0;
    let mut line = start_line;

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
                    let is_bare_pub = skip_ws_and_check_bare_pub(bytes, &mut i, &mut line);
                    match peek_word(bytes, i) {
                        Some("struct") => {
                            let struct_line = line;
                            i += "struct".len();
                            if let Some(parsed) = parse_struct_after_keyword(
                                clean,
                                is_bare_pub,
                                struct_line,
                                &attrs,
                                &mut i,
                                &mut line,
                            ) {
                                structs.push(parsed);
                            }
                            attrs.clear();
                        }
                        Some("mod") => {
                            i += "mod".len();
                            parse_inline_mod_structs(clean, &mut i, &mut line, structs);
                            attrs.clear();
                        }
                        _ => {
                            i = saved_i;
                            line = saved_line;
                        }
                    }
                } else if word == "struct" {
                    let struct_line = line;
                    if let Some(parsed) = parse_struct_after_keyword(
                        clean,
                        false,
                        struct_line,
                        &attrs,
                        &mut i,
                        &mut line,
                    ) {
                        structs.push(parsed);
                    }
                    attrs.clear();
                } else if word == "mod" {
                    parse_inline_mod_structs(clean, &mut i, &mut line, structs);
                    attrs.clear();
                } else if matches!(
                    word,
                    "fn" | "impl" | "enum" | "trait" | "use" | "const" | "static" | "type"
                ) {
                    attrs.clear();
                }
            }
            _ => i += 1,
        }
    }
}

fn parse_inline_mod_structs(
    clean: &str,
    i: &mut usize,
    line: &mut usize,
    structs: &mut Vec<ParsedStruct>,
) {
    let bytes = clean.as_bytes();
    skip_ws(bytes, i, line);
    while *i < bytes.len() && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_') {
        *i += 1;
    }
    skip_ws(bytes, i, line);
    if *i < bytes.len() && bytes[*i] == b'{' {
        let body_start = *i + 1;
        let body_line = *line;
        let (end, newlines) = scan_balanced(bytes, *i, b'{', b'}');
        let inner = &clean[body_start..end.saturating_sub(1)];
        parse_structs_in_slice(inner, body_line, structs);
        *i = end;
        *line += newlines;
    }
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
    let mut in_where = false;

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
            b'>' if angle_depth > 0 && !is_arrow_gt(bytes, *i) => {
                angle_depth -= 1;
                *i += 1;
            }
            b'{' if angle_depth == 0 => {
                let body_start = *i + 1;
                let body_start_line = *line;
                let (end, newlines) = scan_balanced(bytes, *i, b'{', b'}');
                let body = &clean[body_start..end.saturating_sub(1)];
                let fields = parse_braced_fields(body, body_start_line);
                *i = end;
                *line += newlines;
                return Some(ParsedStruct {
                    name,
                    is_pub,
                    is_tuple: false,
                    line: struct_line,
                    attrs: attrs.to_vec(),
                    fields,
                });
            }
            b'(' if angle_depth == 0 && !in_where => {
                let body_start = *i + 1;
                let body_start_line = *line;
                let (end, newlines) = scan_balanced(bytes, *i, b'(', b')');
                let body = &clean[body_start..end.saturating_sub(1)];
                let fields = parse_tuple_fields(body, body_start_line);
                *i = end;
                *line += newlines;
                return Some(ParsedStruct {
                    name,
                    is_pub,
                    is_tuple: true,
                    line: struct_line,
                    attrs: attrs.to_vec(),
                    fields,
                });
            }
            b';' if angle_depth == 0 => return None,
            c if angle_depth == 0 && (c.is_ascii_alphabetic() || c == b'_') => {
                if peek_word(bytes, *i) == Some("where") {
                    in_where = true;
                    *i += 5;
                } else {
                    while *i < bytes.len()
                        && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_')
                    {
                        *i += 1;
                    }
                }
            }
            _ => *i += 1,
        }
    }
    None
}

fn is_arrow_gt(bytes: &[u8], i: usize) -> bool {
    i > 0 && (bytes[i - 1] == b'-' || bytes[i - 1] == b'=')
}

fn parse_braced_fields(body: &str, start_line: usize) -> Vec<ParsedField> {
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
            let _ = skip_ws_and_check_bare_pub(bytes, &mut i, &mut line);
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
        let ty = scan_type_until_comma(body, &mut i, &mut line);
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

fn parse_tuple_fields(body: &str, start_line: usize) -> Vec<ParsedField> {
    let mut fields = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut line = start_line;
    let mut field_attrs = Vec::new();
    let mut index = 0usize;

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
            let _ = skip_ws_and_check_bare_pub(bytes, &mut i, &mut line);
        }
        let field_line = line;
        let ty = scan_type_until_comma(body, &mut i, &mut line);
        if !ty.is_empty() {
            fields.push(ParsedField {
                name: index.to_string(),
                ty,
                line: field_line,
                attrs: std::mem::take(&mut field_attrs),
            });
            index += 1;
        }
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
        }
    }
    fields
}

fn scan_type_until_comma(body: &str, i: &mut usize, line: &mut usize) -> String {
    let bytes = body.as_bytes();
    let ty_start = *i;
    let mut depth_angle = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_paren = 0usize;
    while *i < bytes.len() {
        match bytes[*i] {
            b'\n' => {
                *line += 1;
                *i += 1;
            }
            b'<' => {
                depth_angle += 1;
                *i += 1;
            }
            b'>' if depth_angle > 0 && !is_arrow_gt(bytes, *i) => {
                depth_angle -= 1;
                *i += 1;
            }
            b'[' => {
                depth_bracket += 1;
                *i += 1;
            }
            b']' if depth_bracket > 0 => {
                depth_bracket -= 1;
                *i += 1;
            }
            b'(' => {
                depth_paren += 1;
                *i += 1;
            }
            b')' if depth_paren > 0 => {
                depth_paren -= 1;
                *i += 1;
            }
            b',' if depth_angle == 0 && depth_bracket == 0 && depth_paren == 0 => break,
            _ => *i += 1,
        }
    }
    body[ty_start..*i]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses top-level and inline-module `enum <Name>` declarations with exact 1-based line numbers.
pub fn parse_rust_enums(src: &str) -> Vec<ParsedEnum> {
    let clean = strip_rust_comments_and_strings(src);
    let mut enums = Vec::new();
    parse_enums_in_slice(&clean, 1, &mut enums);
    enums
}

fn parse_enums_in_slice(clean: &str, start_line: usize, enums: &mut Vec<ParsedEnum>) {
    let bytes = clean.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut line = start_line;

    while i < len {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'#' if i + 1 < len && bytes[i + 1] == b'[' => {
                let (end, newlines) = scan_balanced(bytes, i + 1, b'[', b']');
                line += newlines;
                i = end;
            }
            b'{' => {
                let (end, newlines) = scan_balanced(bytes, i, b'{', b'}');
                line += newlines;
                i = end;
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
                    let is_bare_pub = skip_ws_and_check_bare_pub(bytes, &mut i, &mut line);
                    match peek_word(bytes, i) {
                        Some("enum") => {
                            let enum_line = line;
                            i += "enum".len();
                            skip_ws(bytes, &mut i, &mut line);
                            let name_start = i;
                            while i < len
                                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                            {
                                i += 1;
                            }
                            if i > name_start {
                                enums.push(ParsedEnum {
                                    name: clean[name_start..i].to_string(),
                                    is_pub: is_bare_pub,
                                    line: enum_line,
                                });
                            }
                        }
                        Some("mod") => {
                            i += "mod".len();
                            parse_inline_mod_enums(clean, &mut i, &mut line, enums);
                        }
                        _ => {
                            i = saved_i;
                            line = saved_line;
                        }
                    }
                } else if word == "enum" {
                    let enum_line = line;
                    skip_ws(bytes, &mut i, &mut line);
                    let name_start = i;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    if i > name_start {
                        enums.push(ParsedEnum {
                            name: clean[name_start..i].to_string(),
                            is_pub: false,
                            line: enum_line,
                        });
                    }
                } else if word == "mod" {
                    parse_inline_mod_enums(clean, &mut i, &mut line, enums);
                }
            }
            _ => i += 1,
        }
    }
}

fn parse_inline_mod_enums(
    clean: &str,
    i: &mut usize,
    line: &mut usize,
    enums: &mut Vec<ParsedEnum>,
) {
    let bytes = clean.as_bytes();
    skip_ws(bytes, i, line);
    while *i < bytes.len() && (bytes[*i].is_ascii_alphanumeric() || bytes[*i] == b'_') {
        *i += 1;
    }
    skip_ws(bytes, i, line);
    if *i < bytes.len() && bytes[*i] == b'{' {
        let body_start = *i + 1;
        let body_line = *line;
        let (end, newlines) = scan_balanced(bytes, *i, b'{', b'}');
        let inner = &clean[body_start..end.saturating_sub(1)];
        parse_enums_in_slice(inner, body_line, enums);
        *i = end;
        *line += newlines;
    }
}

fn skip_ws(bytes: &[u8], i: &mut usize, line: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        if bytes[*i] == b'\n' {
            *line += 1;
        }
        *i += 1;
    }
}

fn skip_ws_and_check_bare_pub(bytes: &[u8], i: &mut usize, line: &mut usize) -> bool {
    skip_ws(bytes, i, line);
    if *i < bytes.len() && bytes[*i] == b'(' {
        let (end, newlines) = scan_balanced(bytes, *i, b'(', b')');
        *i = end;
        *line += newlines;
        skip_ws(bytes, i, line);
        false
    } else {
        true
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
