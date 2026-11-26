//! Content-based platform purity & table-enum sentinel scanners (name-independent).
//!
//! Catches Web/CSS/ARIA string tables, keyword parsers, CSS/HTML format payloads, core
//! `.to_ascii_lowercase()` allocations, legacy `word_enum!` macros, and hand-rolled `Unset`
//! enum variants regardless of function or method naming.

use crate::rust_items::ParsedField;
use crate::rust_lex::{contains_ident, scan_balanced};
use crate::support::Violation;

/// Unmistakable Web CSS/HTML/ARIA format literals that belong strictly in `nve-backend-web`.
const WEB_FORMAT_TOKENS: &[&str] = &[
    "linear-gradient(",
    "radial-gradient(",
    "cubic-bezier(",
    "@media ",
    "@media(",
    "prefers-color-scheme",
    "prefers-reduced-motion",
    "translate(",
    "translate3d(",
    "rotate(",
    "scale(",
    "minmax(",
    "repeat(",
    "rgba(",
    "px;",
    "%;",
    "<!DOCTYPE",
    "aria-",
];

/// Returns `true` when a `bool` field in a `#[derive(NveTable)]` struct is a split presence
/// sentinel (`*_enabled`, `*_set`, `*_present`, `*_specified`, or `has_<sibling>`).
pub fn is_split_bool_sentinel(field: &ParsedField, all_fields: &[ParsedField]) -> bool {
    if field.ty.trim() != "bool" {
        return false;
    }
    let name = field.name.as_str();
    if name.ends_with("_enabled")
        || name.ends_with("_set")
        || name.ends_with("_present")
        || name.ends_with("_specified")
    {
        return true;
    }
    if let Some(stem) = name.strip_prefix("has_") {
        return all_fields.iter().any(|other| other.name == stem);
    }
    false
}

/// Scans for legacy manual-bit-width `word_enum!` invocations and hand-rolled `Unset`
/// optionality variants inside core table & module spec enums.
pub fn scan_word_enum_and_unset_variants(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    out: &mut Vec<Violation>,
) {
    for (idx, (orig, line)) in prod_raw.lines().zip(prod_clean.lines()).enumerate() {
        if line.contains("word_enum!") && !line.contains("macro_rules!") {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Legacy manual-bit-width `word_enum!` macro is banned; use `#[derive(WordEnum)]`",
            ));
        }
    }

    let check_unset = rel.starts_with("packages/core/tables/")
        || rel.starts_with("packages/core/components/")
        || (rel.starts_with("packages/modules/") && rel.matches('/').count() >= 4);
    if !check_unset {
        return;
    }

    let raw_lines: Vec<&str> = prod_raw.lines().collect();
    let bytes = prod_clean.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if &prod_clean[start..i] != "enum" {
                    continue;
                }
                skip_ws(bytes, &mut i, &mut line);
                let name_start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if i == name_start {
                    continue;
                }
                let enum_name = &prod_clean[name_start..i];
                // Advance past generics / `: <bits>` (for `word_enum!`) until `{` or `;`.
                while i < bytes.len() && bytes[i] != b'{' && bytes[i] != b';' {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'{' {
                    let body_start = i + 1;
                    let body_line = line;
                    let (end, newlines) = scan_balanced(bytes, i, b'{', b'}');
                    let body = &prod_clean[body_start..end.saturating_sub(1)];
                    if let Some(unset_line) = find_unset_variant_line(body, body_line) {
                        out.push(Violation::new(
                            rel,
                            unset_line,
                            raw_lines
                                .get(unset_line.saturating_sub(1))
                                .copied()
                                .unwrap_or(""),
                            format!(
                                "Hand-rolled `Unset` sentinel variant in enum `{enum_name}` is banned; model optionality with `Option<{enum_name}>`"
                            ),
                        ));
                    }
                    i = end;
                    line += newlines;
                }
            }
            _ => i += 1,
        }
    }
}

fn find_unset_variant_line(enum_body: &str, start_line: usize) -> Option<usize> {
    let bytes = enum_body.as_bytes();
    let mut i = 0usize;
    let mut line = start_line;
    let mut expect_variant = true;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'#' => {
                i += 1;
                skip_ws(bytes, &mut i, &mut line);
                if i < bytes.len() && bytes[i] == b'[' {
                    let (end, nl) = scan_balanced(bytes, i, b'[', b']');
                    i = end;
                    line += nl;
                }
            }
            b'{' => {
                let (end, nl) = scan_balanced(bytes, i, b'{', b'}');
                i = end;
                line += nl;
            }
            b'(' => {
                let (end, nl) = scan_balanced(bytes, i, b'(', b')');
                i = end;
                line += nl;
            }
            b',' => {
                expect_variant = true;
                i += 1;
            }
            c if expect_variant && (c.is_ascii_alphabetic() || c == b'_') => {
                let start = i;
                let vline = line;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if &enum_body[start..i] == "Unset" {
                    return Some(vline);
                }
                expect_variant = false;
            }
            _ => i += 1,
        }
    }
    None
}

/// Content- and structure-based platform purity checks inside `packages/core/*`:
/// 1. Bans `.to_ascii_lowercase()` heap allocations in core library crates.
/// 2. Detects enum <-> keyword string tables (regardless of fn name) in `packages/core/{tables,components}/`.
/// 3. Detects Web CSS/HTML/ARIA formatting tokens inside function bodies in `packages/core/{tables,components}/`.
pub fn scan_content_based_platform_purity(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    out: &mut Vec<Violation>,
) {
    // 1. Heap-allocating `.to_ascii_lowercase()` anywhere in core runtime/table/ABI crates.
    let in_core_no_alloc = rel.starts_with("packages/core/abi/")
        || rel.starts_with("packages/core/store/")
        || rel.starts_with("packages/core/engine/")
        || rel.starts_with("packages/core/tables/")
        || rel.starts_with("packages/core/components/")
        || rel.starts_with("packages/core/codec/");
    if in_core_no_alloc {
        for (idx, (orig, line)) in prod_raw.lines().zip(prod_clean.lines()).enumerate() {
            if line.contains(".to_ascii_lowercase()") {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    orig,
                    "`.to_ascii_lowercase()` allocates a heap `String` in core; use `eq_ignore_ascii_case` or move string parsing to `nve-backend-web`",
                ));
            }
        }
    }

    // 2 & 3. Structural content checks in stateless table/component crates.
    let in_core_tables =
        rel.starts_with("packages/core/tables/") || rel.starts_with("packages/core/components/");
    if !in_core_tables {
        return;
    }

    let no_comments = strip_comments_keep_strings(prod_raw);
    let raw_lines: Vec<&str> = prod_raw.lines().collect();
    scan_fns_for_web_content(
        rel,
        prod_clean,
        &no_comments,
        &raw_lines,
        0,
        prod_clean.len(),
        1,
        out,
    );
}

#[allow(clippy::too_many_arguments)]
fn scan_fns_for_web_content(
    rel: &str,
    clean: &str,
    no_comments: &str,
    raw_lines: &[&str],
    start: usize,
    end: usize,
    start_line: usize,
    out: &mut Vec<Violation>,
) {
    let bytes = clean.as_bytes();
    let mut i = start;
    let mut line = start_line;
    while i < end {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'{' => {
                let (b_end, nl) = scan_balanced(bytes, i, b'{', b'}');
                i = b_end.min(end);
                line += nl;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let wstart = i;
                while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &clean[wstart..i];
                if word == "impl" {
                    let hdr_start = i;
                    while i < end && bytes[i] != b'{' && bytes[i] != b';' {
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    let hdr = &clean[hdr_start..i];
                    if i < end && bytes[i] == b'{' {
                        let body_start = i + 1;
                        let body_line = line;
                        let (b_end, nl) = scan_balanced(bytes, i, b'{', b'}');
                        let skip_impl = contains_ident(hdr, "Display")
                            || contains_ident(hdr, "Debug")
                            || hdr
                                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                                .any(|tok| tok.ends_with("Error"));
                        if !skip_impl {
                            scan_fns_for_web_content(
                                rel,
                                clean,
                                no_comments,
                                raw_lines,
                                body_start,
                                b_end.saturating_sub(1).min(end),
                                body_line,
                                out,
                            );
                        }
                        i = b_end.min(end);
                        line += nl;
                    }
                } else if word == "mod" {
                    while i < end && bytes[i] != b'{' && bytes[i] != b';' {
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    if i < end && bytes[i] == b'{' {
                        let body_start = i + 1;
                        let body_line = line;
                        let (b_end, nl) = scan_balanced(bytes, i, b'{', b'}');
                        scan_fns_for_web_content(
                            rel,
                            clean,
                            no_comments,
                            raw_lines,
                            body_start,
                            b_end.saturating_sub(1).min(end),
                            body_line,
                            out,
                        );
                        i = b_end.min(end);
                        line += nl;
                    }
                } else if word == "fn" {
                    let fn_line = line;
                    skip_ws(bytes, &mut i, &mut line);
                    let nstart = i;
                    while i < end && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    let fn_name = &clean[nstart..i];
                    while i < end && bytes[i] != b'{' && bytes[i] != b';' {
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    if i < end && bytes[i] == b'{' {
                        let body_start = i + 1;
                        let (b_end, nl) = scan_balanced(bytes, i, b'{', b'}');
                        if fn_name != "fmt" && !fn_name.is_empty() {
                            let raw_body = &no_comments[body_start..b_end.saturating_sub(1).min(no_comments.len())];
                            check_fn_body_web_content(
                                rel,
                                fn_name,
                                fn_line,
                                raw_body,
                                raw_lines.get(fn_line.saturating_sub(1)).copied().unwrap_or(""),
                                out,
                            );
                        }
                        i = b_end.min(end);
                        line += nl;
                    }
                }
            }
            _ => i += 1,
        }
    }
}

fn check_fn_body_web_content(
    rel: &str,
    fn_name: &str,
    fn_line: usize,
    raw_body: &str,
    snippet: &str,
    out: &mut Vec<Violation>,
) {
    let mut enum_to_str_arms = 0usize;
    let mut str_to_enum_arms = 0usize;
    let mut found_web_token: Option<&'static str> = None;

    for line in raw_body.lines() {
        let trimmed = line.trim();
        if let Some((lhs, rhs)) = trimmed.split_once("=>") {
            let lhs_t = lhs.trim();
            let rhs_t = rhs.trim();
            if lhs_t.contains("::") && rhs_t.starts_with('"') {
                enum_to_str_arms += 1;
            }
            if lhs_t.starts_with('"') {
                str_to_enum_arms += 1;
            }
        }
        if found_web_token.is_none()
            && !trimmed.contains("Key64::of(")
            && !trimmed.contains("const_id(")
            && trimmed.contains('"')
        {
            for &tok in WEB_FORMAT_TOKENS {
                if trimmed.contains(tok) {
                    found_web_token = Some(tok);
                    break;
                }
            }
        }
    }

    if enum_to_str_arms >= 3 {
        out.push(Violation::new(
            rel,
            fn_line,
            snippet,
            format!(
                "Function `{fn_name}` maps {enum_to_str_arms} enum variants to string literals inside core; move Web/CSS/ARIA string tables to `nve-backend-web`"
            ),
        ));
    } else if str_to_enum_arms >= 3 {
        out.push(Violation::new(
            rel,
            fn_line,
            snippet,
            format!(
                "Function `{fn_name}` parses {str_to_enum_arms} string-literal keyword arms inside core; move Web/CSS/ARIA string parsing to `nve-backend-web`"
            ),
        ));
    } else if let Some(tok) = found_web_token {
        out.push(Violation::new(
            rel,
            fn_line,
            snippet,
            format!(
                "Function `{fn_name}` emits/parses Web CSS/HTML/ARIA syntax (`{tok}`) inside core; move formatting to `nve-backend-web`"
            ),
        ));
    }
}

/// Strips `//` line comments and nested `/* ... */` block comments while preserving string/char
/// literals and exact byte length + newlines.
fn strip_comments_keep_strings(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        // Line comment
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < n && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        // Nested block comment
        if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let mut depth = 1usize;
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < n && depth > 0 {
                if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else if i + 1 < n && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else {
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            continue;
        }
        // Preserve raw strings (`r#"..."#`, `br#"..."#`, `cr#"..."#`) verbatim
        if let Some((hashes, qpos)) = match_raw_string_prefix(bytes, i) {
            i = qpos + 1;
            while i < n {
                if bytes[i] == b'"' && has_n_hashes(bytes, i + 1, hashes) {
                    i += 1 + hashes;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Preserve normal strings verbatim
        if bytes[i] == b'"' {
            i += 1;
            while i < n {
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    String::from_utf8(out).expect("ASCII comment blanking preserves UTF-8")
}

fn match_raw_string_prefix(bytes: &[u8], pos: usize) -> Option<(usize, usize)> {
    let n = bytes.len();
    let r_pos = if bytes[pos] == b'r' {
        if pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
            return None;
        }
        pos
    } else if (bytes[pos] == b'b' || bytes[pos] == b'c') && pos + 1 < n && bytes[pos + 1] == b'r' {
        if pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_') {
            return None;
        }
        pos + 1
    } else {
        return None;
    };
    let mut j = r_pos + 1;
    let mut hashes = 0usize;
    while j < n && bytes[j] == b'#' {
        hashes += 1;
        j += 1;
    }
    (j < n && bytes[j] == b'"').then_some((hashes, j))
}

fn has_n_hashes(bytes: &[u8], pos: usize, count: usize) -> bool {
    pos + count <= bytes.len() && bytes[pos..pos + count].iter().all(|&b| b == b'#')
}

fn skip_ws(bytes: &[u8], i: &mut usize, line: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        if bytes[*i] == b'\n' {
            *line += 1;
        }
        *i += 1;
    }
}
