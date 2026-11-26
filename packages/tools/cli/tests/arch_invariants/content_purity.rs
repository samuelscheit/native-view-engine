//! General content- and structure-based invariant scanners (name-independent).
//!
//! 1. **Mirrored `<->` encoder/decoder duplication (`packages/**`)**: flags any two functions in a
//!    file whose `match` arms mirror `>= 3` `(A, B) <-> (B, A)` pairs (enum <-> `&str`, enum <->
//!    int, enum <-> enum) regardless of function naming — must use a single source of truth
//!    (`#[derive(WordEnum)]`, `#[derive(NveCodec)]`, `const TABLE`, or a macro).
//! 2. **Manual integer-discriminant `match` (`packages/**`)**: flags `>= 3`-arm `0 => Variant` or
//!    `Variant => 0` blocks outside `codegen` / `macro_rules!`.
//! 3. **Core table enum <-> `&str` tables (`packages/core/{tables,components}`)**: flags even
//!    one-way `>= 3`-arm enum <-> string-literal tables in stateless core tables.
//! 4. **Repo-wide `.to_ascii_lowercase()` ban (`packages/**`)**: flags any unexplained
//!    `.to_ascii_lowercase()` call unless the same line or preceding 2 lines carry an explicit
//!    `// allow(to_ascii_lowercase): <reason>` explanation.
//! 5. **Split boolean presence sentinels, legacy `word_enum!`, and `Unset` enum variants**.

use std::collections::BTreeSet;

use crate::rust_items::ParsedField;
use crate::rust_lex::{contains_ident, scan_balanced, strip_rust_comments_keep_strings};
use crate::support::Violation;

const EXTERNAL_ENUM_PREFIXES: &[&str] = &[
    "vk::",
    "ash::",
    "winit::",
    "skia_safe::",
    "taffy::",
    "accesskit::",
    "web_sys::",
    "windows::",
    "objc2::",
    "gtk4::",
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

/// Scans ANY production Rust file under `packages/**` for:
/// - Unexplained `.to_ascii_lowercase()` calls (requires `// allow(to_ascii_lowercase): <reason>`).
/// - Duplicated bidirectional `<->` encoder/decoder `match` blocks (`>= 3` mirrored `(A, B)` arms).
/// - Manual `>= 3`-arm integer-discriminant `match` blocks (`0 => Variant` / `Variant => 0`).
pub fn scan_general_codec_duplication_and_lowercase(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    out: &mut Vec<Violation>,
) {
    let raw_lines: Vec<&str> = prod_raw.lines().collect();

    // 1. Repo-wide `.to_ascii_lowercase()` ban unless explicitly documented.
    for (idx, line) in prod_clean.lines().enumerate() {
        if line.contains(".to_ascii_lowercase()") && !has_lowercase_deviation_comment(&raw_lines, idx)
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                raw_lines.get(idx).copied().unwrap_or(""),
                "Unexplained `.to_ascii_lowercase()` is banned anywhere in `packages/`; use `eq_ignore_ascii_case`, `char::to_ascii_lowercase(&c)`, or justify with `// allow(to_ascii_lowercase): <reason>`",
            ));
        }
    }

    // 2. Collect all non-Error/Display functions in this file with their match arm pairs.
    let no_comments = strip_rust_comments_keep_strings(prod_raw);
    let mut fns = Vec::new();
    collect_fn_match_profiles(
        prod_clean,
        &no_comments,
        0,
        prod_clean.len(),
        1,
        &mut fns,
    );

    // 2a. Mirrored `<->` pair detection between any two functions in the file.
    let mut reported_mirror = BTreeSet::new();
    for a in 0..fns.len() {
        if fns[a].pairs.len() < 3 {
            continue;
        }
        for b in (a + 1)..fns.len() {
            if fns[b].pairs.len() < 3 {
                continue;
            }
            let overlap = count_mirrored_pairs(&fns[a].pairs, &fns[b].pairs);
            if overlap >= 3 {
                reported_mirror.insert(a);
                reported_mirror.insert(b);
                out.push(Violation::new(
                    rel,
                    fns[a].line,
                    raw_lines.get(fns[a].line.saturating_sub(1)).copied().unwrap_or(""),
                    format!(
                        "Duplicated bidirectional mapping between `fn {}` (line {}) and `fn {}` (line {}): {overlap} mirrored `match` arms — generate both directions from one single source of truth",
                        fns[a].name, fns[a].line, fns[b].name, fns[b].line
                    ),
                ));
            }
        }
    }

    // 2b. Single-direction manual integer-discriminant `match` (`>= 3` arms of `Int <-> Variant`),
    //     skipping `codegen` emitters and functions already reported as mirrored pairs.
    if !rel.starts_with("packages/core/codegen/") {
        for (idx, f) in fns.iter().enumerate() {
            if reported_mirror.contains(&idx) {
                continue;
            }
            let int_to_var: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Int(n), ArmAtom::Variant(_)) => Some(*n),
                    _ => None,
                })
                .collect();
            let var_to_int: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Variant(_), ArmAtom::Int(n)) => Some(*n),
                    _ => None,
                })
                .collect();
            let count = if is_compact_discriminant_set(&int_to_var) {
                int_to_var.len()
            } else if is_compact_discriminant_set(&var_to_int) {
                var_to_int.len()
            } else {
                0
            };
            if count >= 3 {
                out.push(Violation::new(
                    rel,
                    f.line,
                    raw_lines.get(f.line.saturating_sub(1)).copied().unwrap_or(""),
                    format!(
                        "Manual integer-discriminant `match` in `fn {}` ({count} arms); derive via `#[derive(WordEnum)]` / `#[derive(NveCodec)]` instead of hand-coding discriminants",
                        f.name,
                    ),
                ));
            }
        }
    }
}

/// Core table purity check: inside `packages/core/{tables,components}/`, even a one-way
/// `>= 3`-arm enum <-> string-literal keyword table is banned (strings belong in `nve-backend-web`).
pub fn scan_core_table_string_tables(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    out: &mut Vec<Violation>,
) {
    let in_core_tables =
        rel.starts_with("packages/core/tables/") || rel.starts_with("packages/core/components/");
    if !in_core_tables {
        return;
    }

    let raw_lines: Vec<&str> = prod_raw.lines().collect();
    let no_comments = strip_rust_comments_keep_strings(prod_raw);
    let mut fns = Vec::new();
    collect_fn_match_profiles(
        prod_clean,
        &no_comments,
        0,
        prod_clean.len(),
        1,
        &mut fns,
    );

    for f in &fns {
        let var_to_str = f
            .pairs
            .iter()
            .filter(|(l, r)| matches!((l, r), (ArmAtom::Variant(_), ArmAtom::Str(_))))
            .count();
        let str_to_var = f
            .pairs
            .iter()
            .filter(|(l, r)| matches!((l, r), (ArmAtom::Str(_), ArmAtom::Variant(_))))
            .count();
        if var_to_str >= 3 {
            out.push(Violation::new(
                rel,
                f.line,
                raw_lines.get(f.line.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Function `{}` maps {var_to_str} enum variants to string literals inside core; move Web/CSS/ARIA string tables to `nve-backend-web`",
                    f.name
                ),
            ));
        } else if str_to_var >= 3 {
            out.push(Violation::new(
                rel,
                f.line,
                raw_lines.get(f.line.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Function `{}` parses {str_to_var} string-literal keyword arms into enum variants inside core; move keyword parsing to `nve-backend-web`",
                    f.name
                ),
            ));
        }
    }
}

fn has_lowercase_deviation_comment(raw_lines: &[&str], idx: usize) -> bool {
    let start = idx.saturating_sub(2);
    for &line in &raw_lines[start..=idx] {
        if let Some(pos) = line.find("allow(to_ascii_lowercase):") {
            let reason = line[pos + "allow(to_ascii_lowercase):".len()..].trim();
            if !reason.is_empty() {
                return true;
            }
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ArmAtom {
    Str(String),
    Int(i128),
    Variant(String),
}

/// Returns `true` for compact enum-discriminant tables (e.g. `0, 1, 2` or `1, 2, 3`),
/// rejecting priority weights (`10, 100, 500, 1000`) and sparse constants (`200, 404, 500`).
fn is_compact_discriminant_set(vals: &[i128]) -> bool {
    if vals.len() < 3 {
        return false;
    }
    let unique: BTreeSet<i128> = vals.iter().copied().collect();
    if unique.len() < 3 {
        return false;
    }
    let min = *unique.first().unwrap_or(&0);
    let max = *unique.last().unwrap_or(&0);
    min >= 0 && max <= 255 && (max - min) <= (unique.len() as i128 * 2 + 4)
}

struct FnMatchProfile {
    name: String,
    line: usize,
    pairs: BTreeSet<(ArmAtom, ArmAtom)>,
}

fn count_mirrored_pairs(
    a: &BTreeSet<(ArmAtom, ArmAtom)>,
    b: &BTreeSet<(ArmAtom, ArmAtom)>,
) -> usize {
    a.iter()
        .filter(|(lhs, rhs)| lhs != rhs && b.contains(&(rhs.clone(), lhs.clone())))
        .count()
}

fn collect_fn_match_profiles(
    clean: &str,
    no_comments: &str,
    start: usize,
    end: usize,
    start_line: usize,
    out: &mut Vec<FnMatchProfile>,
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
                if word == "macro_rules" {
                    while i < end && bytes[i] != b'{' && bytes[i] != b';' {
                        if bytes[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                    if i < end && bytes[i] == b'{' {
                        let (b_end, nl) = scan_balanced(bytes, i, b'{', b'}');
                        i = b_end.min(end);
                        line += nl;
                    }
                } else if word == "impl" {
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
                            collect_fn_match_profiles(
                                clean,
                                no_comments,
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
                        collect_fn_match_profiles(
                            clean,
                            no_comments,
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
                            let raw_body =
                                &no_comments[body_start..b_end.saturating_sub(1).min(no_comments.len())];
                            let pairs = extract_match_arm_pairs(raw_body);
                            if !pairs.is_empty() {
                                out.push(FnMatchProfile {
                                    name: fn_name.to_string(),
                                    line: fn_line,
                                    pairs,
                                });
                            }
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

fn extract_match_arm_pairs(raw_body: &str) -> BTreeSet<(ArmAtom, ArmAtom)> {
    let mut pairs = BTreeSet::new();
    for line in raw_body.lines() {
        let Some((lhs_raw, rhs_raw)) = line.split_once("=>") else {
            continue;
        };
        let Some(rhs) = parse_arm_atom(rhs_raw) else {
            continue;
        };
        for lhs_alt in lhs_raw.split('|') {
            if let Some(lhs) = parse_arm_atom(lhs_alt) {
                pairs.insert((lhs, rhs.clone()));
            }
        }
    }
    pairs
}

fn parse_arm_atom(raw: &str) -> Option<ArmAtom> {
    let mut s = raw.trim().trim_end_matches(',').trim();
    if s.is_empty() || s.starts_with("if ") || s.contains('(') && !s.starts_with("Some(") && !s.starts_with("Ok(") {
        return None;
    }
    if let Some(inner) = s
        .strip_prefix("Some(")
        .or_else(|| s.strip_prefix("Ok("))
        .and_then(|x| x.strip_suffix(')'))
    {
        s = inner.trim();
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"'))
        && !inner.contains('"')
    {
        return Some(ArmAtom::Str(inner.to_string()));
    }
    if let Some(int_norm) = parse_int_atom(s) {
        return Some(ArmAtom::Int(int_norm));
    }
    if EXTERNAL_ENUM_PREFIXES.iter().any(|p| s.contains(p)) {
        return None;
    }
    if let Some((_, leaf)) = s.rsplit_once("::") {
        let leaf = leaf.trim();
        if leaf.starts_with(|c: char| c.is_ascii_uppercase())
            && leaf.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Some(ArmAtom::Variant(leaf.to_string()));
        }
    }
    None
}

fn parse_int_atom(s: &str) -> Option<i128> {
    let stripped = s.strip_prefix('-').unwrap_or(s);
    let body = stripped
        .trim_end_matches("usize")
        .trim_end_matches("isize")
        .trim_end_matches("u64")
        .trim_end_matches("i64")
        .trim_end_matches("u32")
        .trim_end_matches("i32")
        .trim_end_matches("u16")
        .trim_end_matches("i16")
        .trim_end_matches("u8")
        .trim_end_matches("i8");
    if body.is_empty() {
        return None;
    }
    let val = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        let digits: String = hex.chars().filter(|c| *c != '_').collect();
        i128::from_str_radix(&digits, 16).ok()?
    } else if body.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
        let digits: String = body.chars().filter(|c| *c != '_').collect();
        digits.parse::<i128>().ok()?
    } else {
        return None;
    };
    Some(if s.starts_with('-') { -val } else { val })
}

fn skip_ws(bytes: &[u8], i: &mut usize, line: &mut usize) {
    while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
        if bytes[*i] == b'\n' {
            *line += 1;
        }
        *i += 1;
    }
}
