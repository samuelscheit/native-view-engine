//! Zero-boilerplate macro hygiene, duplicate internal enum detection, redundant NVE-to-NVE
//! enum/discriminant converter detection, sentinel/Opt wrapper bans, and Tier-1/2 byte-codec checks.

use std::collections::{BTreeMap, BTreeSet};

use crate::arch_inv::types_abi_and_regressions::{
    has_derive_token, has_repr_token, iter_keyword_statements,
};
use crate::support::{
    assert_staged_violations, contains_ident, is_self_invariant_file, is_test_or_bench_rs,
    parse_rust_enums, parse_rust_structs, repo_index, strip_rust_comments_and_strings, Violation,
};
use crate::Stage;

const EXTERNAL_SDK_PREFIXES: &[&str] = &[
    "taffy::",
    "skia_safe::",
    "web_sys::",
    "accesskit::",
    "winit::",
    "parley::",
    "cosmic_text::",
];

const EXTERNAL_BRIDGE_FN_PREFIXES: &[&str] = &[
    "to_taffy_",
    "from_taffy_",
    "to_skia_",
    "from_skia_",
    "to_web_",
    "to_accesskit_",
    "to_winit_",
    "to_parley_",
    "to_cosmic_",
];

const MAGIC_SENTINEL_DEFAULTS: &[&str] = &[
    "default(u8::MAX)",
    "default(u16::MAX)",
    "default(u32::MAX)",
    "default(i16::MIN)",
    "default(i32::MIN)",
    "default(f32::NAN)",
    "default(f32::INFINITY)",
    "default(f32::NEG_INFINITY)",
];

const HANDWRITTEN_BYTE_READERS: &[&str] = &["ByteReader", "BinReader", "WireReader", "Cursor"];

#[test]
fn zero_boilerplate_macros_and_converters() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut core_files: Vec<(String, String)> = Vec::new();
    let mut backend_files: Vec<(String, String)> = Vec::new();
    let mut scanned = 0usize;

    for rel in idx.rs_files_under("packages") {
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let Some(cached) = idx.read_rust(&rel) else {
            continue;
        };
        scanned += 1;
        scan_macro_and_converter_str(&rel, &cached.prod_raw, &mut violations);
        if rel.starts_with("packages/core/") {
            core_files.push((rel, cached.prod_raw));
        } else if rel.starts_with("packages/backends/") {
            backend_files.push((rel, cached.prod_raw));
        }
    }
    assert!(
        scanned > 0 && !core_files.is_empty() && !backend_files.is_empty(),
        "zero_boilerplate_macros_and_converters must scan core & backend files"
    );

    scan_duplicate_enums_and_converters(&core_files, &backend_files, &mut violations);

    assert_staged_violations(
        Stage::Post1B1D,
        "Zero-boilerplate macros, no duplicate internal enums, no redundant NVE-to-NVE converters, no Tier-1 byte codecs",
        &violations,
    );
}

/// Scans `packages/core/*` for duplicate `pub enum <Name>` definitions across 2+ files, and
/// scans `packages/{core,backends}/*` for redundant NVE-to-NVE `impl From` and manual
/// enum/discriminant converter functions (allowing genuine external SDK bridges).
pub fn scan_duplicate_enums_and_converters(
    core_files: &[(String, String)],
    backend_files: &[(String, String)],
    out: &mut Vec<Violation>,
) {
    let mut all_enum_defs: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    let mut core_enum_names: BTreeSet<String> = BTreeSet::new();
    let mut internal_enum_names: BTreeSet<String> = BTreeSet::new();

    for (rel, prod_raw) in core_files {
        for en in parse_rust_enums(prod_raw) {
            if en.is_pub {
                all_enum_defs
                    .entry(en.name.clone())
                    .or_default()
                    .push((rel.clone(), en.line));
                core_enum_names.insert(en.name.clone());
                if !en.name.ends_with("Error") {
                    internal_enum_names.insert(en.name);
                }
            }
        }
    }
    for (rel, prod_raw) in backend_files {
        for en in parse_rust_enums(prod_raw) {
            if en.is_pub {
                all_enum_defs
                    .entry(en.name.clone())
                    .or_default()
                    .push((rel.clone(), en.line));
                if !en.name.ends_with("Error") {
                    internal_enum_names.insert(en.name);
                }
            }
        }
    }

    // 1. Flag duplicate `pub enum <Name>` where at least one definition lives in `packages/core/`
    //    and another lives in any other `packages/{core,backends}/*` file (e.g. `style_patch::TextAlign`
    //    vs `skia::text_cache::TextAlign`, or `interop/surface::ColorScheme` vs `interop/traits::ColorScheme`).
    for (name, locs) in &all_enum_defs {
        if !core_enum_names.contains(name) {
            continue;
        }
        let distinct_files: BTreeSet<&str> = locs.iter().map(|(f, _)| f.as_str()).collect();
        if distinct_files.len() >= 2 {
            let formatted: Vec<String> = locs.iter().map(|(f, l)| format!("{f}:{l}")).collect();
            let summary = formatted.join(", ");
            for (f, l) in locs {
                out.push(Violation::new(
                    f,
                    *l,
                    format!("pub enum {name} (duplicates: {summary})"),
                    format!(
                        "Duplicate internal `pub enum {name}` defined across {} core/backend files; collapse into one canonical type",
                        distinct_files.len()
                    ),
                ));
            }
        }
    }

    // 2. Scan `packages/{core,backends}/*` for redundant internal `impl From` and manual converter fns.
    for (rel, prod_raw) in core_files.iter().chain(backend_files.iter()) {
        scan_internal_enum_converters_in_str(rel, prod_raw, &internal_enum_names, out);
    }
}

pub fn scan_internal_enum_converters_in_str(
    rel: &str,
    src: &str,
    internal_enums: &BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    let clean = strip_rust_comments_and_strings(src);
    let lines: Vec<&str> = src.lines().collect();

    // 2a. `impl ... From<A> for B` where both A and B are known internal NVE enums (and not foreign SDKs).
    for (line_no, stmt) in iter_keyword_statements(&clean, "impl", b'{') {
        let Some(from_pos) = stmt.find("From<") else {
            continue;
        };
        let after_from = &stmt[from_pos + "From<".len()..];
        let Some(gt_pos) = after_from.find('>') else {
            continue;
        };
        let src_ty = after_from[..gt_pos].trim();
        let after_gt = &after_from[gt_pos + 1..];
        let Some(for_pos) = after_gt.find("for ") else {
            continue;
        };
        let dst_ty = after_gt[for_pos + 4..]
            .split(|c: char| c == '{' || c == '<' || c.is_whitespace())
            .find(|s| !s.is_empty())
            .unwrap_or("");
        if has_external_sdk_prefix(src_ty) || has_external_sdk_prefix(dst_ty) {
            continue;
        }
        let src_leaf = last_path_ident(src_ty);
        let dst_leaf = last_path_ident(dst_ty);
        if src_leaf != dst_leaf
            && internal_enums.contains(src_leaf)
            && internal_enums.contains(dst_leaf)
        {
            out.push(Violation::new(
                rel,
                line_no,
                lines.get(line_no.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Redundant internal NVE-to-NVE enum converter `impl From<{src_ty}> for {dst_ty}`; collapse into one canonical enum"
                ),
            ));
        }
    }

    // 2b. Manual enum/discriminant converter functions: `fn (decode_|map_|convert_|to_|from_...)(...) -> ...`
    for (line_no, sig) in iter_keyword_statements(&clean, "fn", b'{') {
        let trimmed = sig.trim().strip_prefix("fn").unwrap_or("").trim_start();
        let fn_name: String = trimmed
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if fn_name.is_empty() {
            continue;
        }
        let is_converter_name = fn_name.starts_with("decode_")
            || fn_name.starts_with("map_")
            || fn_name.starts_with("convert_")
            || fn_name.starts_with("to_")
            || fn_name.starts_with("from_")
            || matches!(fn_name.as_str(), "from_u8" | "from_u32" | "to_u8");
        if !is_converter_name
            || EXTERNAL_BRIDGE_FN_PREFIXES
                .iter()
                .any(|p| fn_name.starts_with(p))
        {
            continue;
        }
        let Some(lparen) = trimmed.find('(') else {
            continue;
        };
        let Some(rparen) = find_matching_paren(trimmed, lparen) else {
            continue;
        };
        let params = &trimmed[lparen + 1..rparen];
        let rest = &trimmed[rparen + 1..];
        let Some(arrow_pos) = rest.find("->") else {
            continue;
        };
        let ret_ty = rest[arrow_pos + 2..].trim_end_matches('{').trim();

        // Allow genuine external SDK bridges (`taffy::*`, `skia_safe::*`, `web_sys::*`, etc.).
        if has_external_sdk_prefix(params) || has_external_sdk_prefix(ret_ty) {
            continue;
        }
        // Skip methods on `self` (e.g. `impl MyEnum { fn to_u8(self) }`) unless converting between two enums.
        let takes_self = params
            .split(',')
            .next()
            .is_some_and(|p| contains_ident(p, "self"));

        let param_enums = find_internal_enums_in_types(params, internal_enums);
        let ok_ret_ty = unwrap_result_ok_type(ret_ty);
        let ret_enums = find_internal_enums_in_types(ok_ret_ty, internal_enums);
        let single_discriminant = is_single_bare_discriminant_param(params);
        let is_self_discriminant_fn = fn_name.starts_with("from_u")
            || fn_name.starts_with("from_i")
            || fn_name.starts_with("from_word")
            || fn_name.starts_with("from_code")
            || fn_name.starts_with("decode_")
            || fn_name.starts_with("map_")
            || fn_name.starts_with("convert_");

        let ret_is_collection =
            ok_ret_ty.contains("Vec<") || ok_ret_ty.contains('&') || ok_ret_ty.contains('[');
        let is_redundant_discriminant = !takes_self
            && !ret_is_collection
            && ((single_discriminant && contains_ident(ok_ret_ty, "Self") && is_self_discriminant_fn)
                || (!ret_enums.is_empty() && params_have_bare_discriminant(params)));
        let is_redundant_enum_to_enum = !ret_is_collection
            && !param_enums.is_empty()
            && !ret_enums.is_empty()
            && param_enums != ret_enums;

        if is_redundant_discriminant || is_redundant_enum_to_enum {
            out.push(Violation::new(
                rel,
                line_no,
                lines.get(line_no.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Manual internal enum/discriminant converter `fn {fn_name}(...) -> {ret_ty}`; use the canonical `#[derive(WordEnum)]` type directly"
                ),
            ));
        }
    }
}

/// Returns `true` iff the function takes exactly one parameter of bare integer type (`u8`/`u16`/`u32`/`i32`).
fn is_single_bare_discriminant_param(params: &str) -> bool {
    let mut parts = params.split(',').map(str::trim).filter(|p| !p.is_empty());
    let Some(first) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let Some((_name, ty)) = first.split_once(':') else {
        return false;
    };
    let norm: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    matches!(norm.as_str(), "u8" | "u16" | "u32" | "i32")
}

/// Returns `true` iff any parameter type is literally a bare integer discriminant (`u8`, `u16`,
/// `u32`, `i32`), NOT a byte slice (`&[u8]`) or `Vec<u8>`.
fn params_have_bare_discriminant(params: &str) -> bool {
    for param in params.split(',') {
        let Some((_name, ty)) = param.split_once(':') else {
            continue;
        };
        let norm: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        if matches!(norm.as_str(), "u8" | "u16" | "u32" | "i32") {
            return true;
        }
    }
    false
}

/// If `ret_ty` is `Result<OkTy, ErrTy>`, returns `OkTy` so error enums (`DecodeError`) don't
/// count as returning an internal domain enum.
fn unwrap_result_ok_type(ret_ty: &str) -> &str {
    let t = ret_ty.trim();
    let Some(inner) = t
        .strip_prefix("Result<")
        .or_else(|| t.strip_prefix("core::result::Result<"))
        .or_else(|| t.strip_prefix("std::result::Result<"))
        .and_then(|s| s.strip_suffix('>'))
    else {
        return t;
    };
    let mut depth = 0usize;
    for (idx, b) in inner.bytes().enumerate() {
        match b {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return inner[..idx].trim(),
            _ => {}
        }
    }
    inner.trim()
}

fn has_external_sdk_prefix(s: &str) -> bool {
    EXTERNAL_SDK_PREFIXES.iter().any(|p| s.contains(p))
}

fn last_path_ident(ty: &str) -> &str {
    ty.trim()
        .trim_start_matches('&')
        .trim()
        .rsplit("::")
        .next()
        .unwrap_or("")
        .split('<')
        .next()
        .unwrap_or("")
        .trim()
}

fn find_internal_enums_in_types(ty_text: &str, internal_enums: &BTreeSet<String>) -> Vec<String> {
    let mut found = Vec::new();
    let bytes = ty_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &ty_text[start..i];
            if internal_enums.contains(ident) && !found.iter().any(|x| x == ident) {
                found.push(ident.to_string());
            }
        } else {
            i += 1;
        }
    }
    found
}

fn find_matching_paren(s: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, b) in s[open_idx..].bytes().enumerate() {
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(open_idx + offset);
            }
        }
    }
    None
}

pub fn scan_macro_and_converter_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    let in_store = rel.starts_with("packages/core/store/");
    let in_dioxus = rel.starts_with("packages/core/dioxus/");
    let in_codec = rel.starts_with("packages/core/codec/");
    let lines: Vec<&str> = src.lines().collect();

    // 1. Manual `impl ... WordValue for ...` outside nve-store.
    if !in_store {
        for (line_no, stmt) in iter_keyword_statements(&clean, "impl", b'{') {
            if contains_ident(stmt, "WordValue") && contains_ident(stmt, "for") {
                out.push(Violation::new(
                    rel,
                    line_no,
                    lines.get(line_no.saturating_sub(1)).copied().unwrap_or(""),
                    "Manual `impl WordValue` outside `nve-store` is banned; derive via `#[derive(WordEnum)]`, `word_flags!`, or `#[derive(NveTable)]`",
                ));
            }
        }
    }

    // 2. Struct-level checks: `#[derive(NveTable)]` (`#[bits(N)]`, magic sentinels, `*_enabled`),
    //    manual `pub struct Opt<UpperCamel>`, and hand-written byte readers outside `packages/core/codec/`.
    let structs = parse_rust_structs(src);
    let has_repr_c = structs.iter().any(|st| has_repr_token(&st.attrs, "C"));
    let check_opt_wrappers = rel.starts_with("packages/core/tables/")
        || rel.starts_with("packages/core/components/")
        || rel.starts_with("packages/core/store/")
        || rel.starts_with("packages/modules/");
    let has_reader_methods = clean.contains("fn read_u32")
        || clean.contains("fn read_u16")
        || clean.contains("from_le_bytes");

    for st in &structs {
        if check_opt_wrappers
            && st.is_pub
            && let Some(rest) = st.name.strip_prefix("Opt")
            && rest.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        {
            out.push(Violation::new(
                rel,
                st.line,
                lines.get(st.line.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Manual optional wrapper `pub struct {}` is banned; use `Option<T>` / `Opt<T>` directly",
                    st.name
                ),
            ));
        }
        if !in_codec
            && !st.is_pub
            && has_reader_methods
            && HANDWRITTEN_BYTE_READERS.contains(&st.name.as_str())
        {
            out.push(Violation::new(
                rel,
                st.line,
                lines.get(st.line.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Hand-written private byte-reader `struct {}` outside `packages/core/codec/` is banned; use `#[derive(NveCodec)]`",
                    st.name
                ),
            ));
        }

        if !has_derive_token(&st.attrs, "NveTable") {
            continue;
        }
        for field in &st.fields {
            let snippet = lines.get(field.line.saturating_sub(1)).copied().unwrap_or("");
            if let Some(bits_n) = extract_bits_attr(&field.attrs)
                && !is_valid_narrowing_bits(&field.ty, bits_n)
            {
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    format!(
                        "Redundant/invalid `#[bits({bits_n})]` on `{}: {}` in #[derive(NveTable)]; only valid when strictly narrowing an integer primitive (`u8`/`u16`/`u32`/`i8`/`i16`/`i32`)",
                        field.name, field.ty
                    ),
                ));
            }
            for &sentinel in MAGIC_SENTINEL_DEFAULTS {
                if field
                    .attrs
                    .iter()
                    .any(|a| a.replace(' ', "").contains(sentinel))
                {
                    out.push(Violation::new(
                        rel,
                        field.line,
                        snippet,
                        format!(
                            "Magic sentinel `#[{sentinel}]` in NveTable is banned; use `Option<T>`"
                        ),
                    ));
                }
            }
            if field.ty == "bool" && field.name.ends_with("_enabled") {
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    format!(
                        "Split boolean sentinel `{}` in NveTable is banned; fold into `Option<T>`",
                        field.name
                    ),
                ));
            }
        }
    }

    // 3. Manual `SET_*` constants and Tier-1 `#[repr(C)]` `encode_bytes`/`decode_bytes`.
    for (idx, (orig, line)) in src.lines().zip(clean.lines()).enumerate() {
        if (rel.starts_with("packages/core/tables/") || rel.starts_with("packages/core/components/"))
            && (line.contains("const SET_") || line.contains("pub const SET_"))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Manual `SET_*` mask constants are banned; use `#[nve_table(inherited)]`",
            ));
        }
        if has_repr_c
            && rel.starts_with("packages/core/")
            && (line.contains("fn encode_bytes(") || line.contains("fn decode_bytes("))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Hand-written `encode_bytes`/`decode_bytes` on Tier 1 `#[repr(C)]` POD is banned; pass typed memory or derive `#[derive(NveCodec)]`",
            ));
        }
    }

    // 4. Multi-line numeric RSX `f64`/`i64` -> String round-trip scan in `nve-dioxus`.
    if in_dioxus {
        let clean_lines: Vec<&str> = clean.lines().collect();
        for (idx, line) in clean_lines.iter().enumerate() {
            if line.contains("AttributeValue::Float(") || line.contains("AttributeValue::Int(") {
                let arm = collect_match_arm(&clean_lines, idx);
                if arm.contains(".to_string()") || arm.contains("format!(") {
                    out.push(Violation::new(
                        rel,
                        idx + 1,
                        lines.get(idx).copied().unwrap_or(""),
                        "Numeric RSX `f64`/`i64` -> `String` -> parse round-trip in `nve-dioxus` is banned; write typed numeric value directly",
                    ));
                }
            }
        }
    }
}

fn extract_bits_attr(attrs: &[String]) -> Option<usize> {
    for attr in attrs {
        let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(pos) = compact.find("bits(") {
            let rest = &compact[pos + "bits(".len()..];
            if let Some(end) = rest.find(')')
                && let Ok(n) = rest[..end].parse::<usize>()
            {
                return Some(n);
            }
        }
    }
    None
}

/// Only allows `#[bits(K)]` on integer primitives when strictly narrowing (`0 < K < natural_bits`).
fn is_valid_narrowing_bits(ty: &str, bits_n: usize) -> bool {
    let natural = match ty.trim() {
        "u8" | "i8" => 8,
        "u16" | "i16" => 16,
        "u32" | "i32" => 32,
        _ => return false,
    };
    bits_n > 0 && bits_n < natural
}

fn collect_match_arm(lines: &[&str], start: usize) -> String {
    let mut buf = String::new();
    let mut brace_depth = 0usize;
    let mut seen_arrow = false;
    for &line in lines.iter().skip(start).take(12) {
        buf.push_str(line);
        buf.push(' ');
        if line.contains("=>") {
            seen_arrow = true;
        }
        for b in line.bytes() {
            if b == b'{' {
                brace_depth += 1;
            } else if b == b'}' {
                brace_depth = brace_depth.saturating_sub(1);
                if seen_arrow && brace_depth == 0 {
                    return buf;
                }
            }
        }
        if seen_arrow && brace_depth == 0 && line.trim_end().ends_with(',') {
            return buf;
        }
    }
    buf
}
