//! Staged Track 1B-1 / 1B-1d invariants: typed domain fields, sealed arenas, `#[repr(C)]` ABI
//! layout locks, legacy regression bans, and zero-boilerplate macro & converter hygiene.

use std::collections::BTreeMap;

use crate::rust_lex::ParsedStruct;
use crate::support::{
    assert_staged_violations, contains_ident, is_self_invariant_file, is_test_or_bench_rs,
    parse_rust_structs, repo_index, strip_cfg_test_items, strip_rust_comments_and_strings,
    Violation,
};
use crate::Stage;

const BARE_PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64",
];

const DOMAIN_FIELD_SUFFIXES: &[&str] = &[
    "_id", "_key", "_slot", "_index", "_off", "_len", "_mask", "_flags", "_us", "_ms",
];

const DOMAIN_FIELD_EXACT: &[&str] = &["parent", "surface", "node", "rel"];

const BANNED_LEGACY_SYMBOLS_NODESTORE: &[&str] = &[
    "ComponentKind",
    "NVE_HOST_CLASS_",
    "Opcode::SetRawProp",
    "RawPropValue",
    "RawStyleProps",
    "NativeViewSpec",
    "RenderSink",
    "BatchSink",
    "ExtensionUpdate",
    "dyn ErasedExtension",
];

const BANNED_LEGACY_SYMBOLS_1B1D: &[&str] = &[
    "NativeEventKind",
    "NveEventPayloadTag",
    "EventPayload",
    "EventMask",
    "NveDirectEvent",
    "TypedTableEvent",
    "StylePatch",
];

/// Enforces [INV:typed-domain-handles].
#[test]
fn bare_primitive_domain_fields_in_core_structs() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for dir in ["abi", "store", "engine", "tables", "components", "codec"] {
        for rel in idx.rs_files_under(&format!("packages/core/{dir}/src")) {
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let Some(cached) = idx.read_rust(&rel) else {
                continue;
            };
            scanned += 1;
            scan_bare_primitive_domain_fields_in_str(&rel, &cached.prod_raw, &mut violations);
        }
    }
    // Also scan root spec crates `packages/modules/<cap>/src` (excluding leaf backend sub-crates).
    for pkg in idx.graph.workspace_packages() {
        if pkg.rel_dir.starts_with("packages/modules/") && pkg.rel_dir.matches('/').count() == 2 {
            for rel in idx.rs_files_under(&format!("{}/src", pkg.rel_dir)) {
                if is_test_or_bench_rs(&rel) {
                    continue;
                }
                if let Some(cached) = idx.read_rust(&rel) {
                    scanned += 1;
                    scan_bare_primitive_domain_fields_in_str(
                        &rel,
                        &cached.prod_raw,
                        &mut violations,
                    );
                }
            }
        }
    }
    assert!(scanned > 0, "bare_primitive_domain_fields must scan > 0 files");
    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            !violations.is_empty(),
            "Expected bare primitive domain field scanner to detect pre-NodeStore fields in packages/core/abi"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero bare primitive domain fields in core/{abi,store,engine,tables,components,codec} & modules/<cap> pub structs",
        &violations,
    );
}

pub fn scan_bare_primitive_domain_fields_in_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let lines: Vec<&str> = src.lines().collect();
    for st in parse_rust_structs(src) {
        if !st.is_pub || has_repr_token(&st.attrs, "transparent") {
            continue;
        }
        for field in &st.fields {
            if st.is_tuple {
                // Tuple struct field name is positional ("0"), check struct name for domain suffix.
                let is_domain_struct = DOMAIN_FIELD_SUFFIXES
                    .iter()
                    .any(|s| st.name.to_ascii_lowercase().ends_with(s));
                if is_domain_struct && is_bare_or_wrapped_primitive(&field.ty) {
                    let snippet = lines.get(field.line.saturating_sub(1)).copied().unwrap_or("");
                    out.push(Violation::new(
                        rel,
                        field.line,
                        snippet,
                        format!(
                            "pub tuple struct `{}` wraps bare primitive `{}` without #[repr(transparent)]",
                            st.name, field.ty
                        ),
                    ));
                }
                continue;
            }
            let is_domain_name = DOMAIN_FIELD_EXACT.contains(&field.name.as_str())
                || DOMAIN_FIELD_SUFFIXES.iter().any(|s| field.name.ends_with(s));
            if is_domain_name && is_bare_or_wrapped_primitive(&field.ty) {
                let snippet = lines
                    .get(field.line.saturating_sub(1))
                    .copied()
                    .unwrap_or("");
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    format!(
                        "pub struct `{}` field `{}: {}` uses bare primitive instead of a distinct #[repr(transparent)] handle/unit newtype",
                        st.name, field.name, field.ty
                    ),
                ));
            }
        }
    }
}

fn is_bare_or_wrapped_primitive(ty: &str) -> bool {
    let compact: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    if BARE_PRIMITIVES.contains(&compact.as_str()) {
        return true;
    }
    // Option<prim>, core::option::Option<prim>, std::option::Option<prim>, Vec<prim>
    for prefix in [
        "Option<",
        "core::option::Option<",
        "std::option::Option<",
        "Vec<",
        "std::vec::Vec<",
        "alloc::vec::Vec<",
    ] {
        if let Some(inner) = compact
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix('>'))
            && BARE_PRIMITIVES.contains(&inner)
        {
            return true;
        }
    }
    // &[prim] or &mut [prim]
    if let Some(slice_inner) = compact
        .strip_prefix("&[")
        .or_else(|| compact.strip_prefix("&mut["))
        .and_then(|s| s.strip_suffix(']'))
        && BARE_PRIMITIVES.contains(&slice_inner)
    {
        return true;
    }
    // [prim; N]
    if let Some(arr_inner) = compact.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
        && let Some((elem, _len)) = arr_inner.split_once(';')
        && BARE_PRIMITIVES.contains(&elem)
    {
        return true;
    }
    false
}

/// Checks whether any `#[repr(...)]` attribute contains `token` as an exact identifier
/// (handles `#[repr(C)]`, `#[repr(align(64), C)]`, `#[repr(transparent)]`, etc.).
pub fn has_repr_token(attrs: &[String], token: &str) -> bool {
    for attr in attrs {
        let Some(inner) = attr
            .trim()
            .strip_prefix("#[")
            .and_then(|s| s.strip_suffix(']'))
            .map(str::trim)
        else {
            continue;
        };
        let Some(rest) = inner.strip_prefix("repr") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(args) = rest.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            continue;
        };
        if args
            .split(',')
            .any(|part| part.trim() == token)
        {
            return true;
        }
    }
    false
}

#[test]
fn untyped_span_and_transparent_arena_alias_guard() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for dir in ["abi", "store", "engine", "tables"] {
        for rel in idx.rs_files_under(&format!("packages/core/{dir}/src")) {
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let Some(cached) = idx.read_rust(&rel) else {
                continue;
            };
            scanned += 1;
            scan_untyped_span_in_str(&rel, &cached.prod_raw, &mut violations);
        }
    }
    assert!(scanned > 0, "untyped_span_and_transparent_arena_alias_guard must scan > 0 files");
    assert_staged_violations(
        Stage::PostNodeStore,
        "No untyped `Span` or transparent `StrArena = SpanPool<u8>` aliases",
        &violations,
    );
}

pub fn scan_untyped_span_in_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    let lines: Vec<&str> = src.lines().collect();

    // 1. Check multi-line `type StrArena = ...SpanPool<u8>;` declarations.
    for (line_no, stmt) in iter_keyword_statements(&clean, "type", b';') {
        let compact: String = stmt.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("typeStrArena=") && compact.contains("SpanPool<u8>") {
            let snippet = lines.get(line_no.saturating_sub(1)).copied().unwrap_or("");
            out.push(Violation::new(
                rel,
                line_no,
                snippet,
                "StrArena must be a sealed UTF-8 newtype, never a transparent `SpanPool<u8>` alias",
            ));
        }
    }

    // 2. Check ALL structs (pub or private, braced or tuple) for untyped `struct Span` (non-generic).
    for (line_no, stmt) in iter_keyword_statements(&clean, "struct", b'{') {
        let trimmed = stmt.trim();
        let after_kw = trimmed.strip_prefix("struct").unwrap_or("").trim_start();
        let ident: String = after_kw
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let rest = after_kw[ident.len()..].trim_start();
        if ident == "Span" && !rest.starts_with('<') {
            let snippet = lines.get(line_no.saturating_sub(1)).copied().unwrap_or("");
            out.push(Violation::new(
                rel,
                line_no,
                snippet,
                "Untyped `struct Span` is banned; use typed `Span<T>` / `Utf8Span` handles",
            ));
        }
    }
}

/// Yields `(1-based start line, statement slice)` for every occurrence of identifier `kw`
/// up to the next `terminator` (`;` or `{` / `(` for structs).
pub fn iter_keyword_statements<'a>(clean: &'a str, kw: &str, terminator: u8) -> Vec<(usize, &'a str)> {
    let bytes = clean.as_bytes();
    let mut results = Vec::new();
    let mut pos = 0usize;
    let kw_len = kw.len();

    while let Some(rel_idx) = clean[pos..].find(kw) {
        let abs = pos + rel_idx;
        let before_ok =
            abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let after = abs + kw_len;
        let after_ok = after < bytes.len()
            && !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            let line_no = clean[..abs].bytes().filter(|&b| b == b'\n').count() + 1;
            let mut end = after;
            while end < bytes.len() {
                let b = bytes[end];
                if b == terminator
                    || (terminator == b'{' && (b == b';' || (kw == "struct" && b == b'(')))
                {
                    end += 1;
                    break;
                }
                end += 1;
            }
            results.push((line_no, &clean[abs..end]));
            pos = end;
        } else {
            pos = after;
        }
    }
    results
}

/// Enforces [INV:abi-repr-c-locks].
#[test]
fn repr_c_abi_layout_asserts_and_cache_line_locks() {
    let idx = repo_index();
    let mut abi_files: Vec<(String, String)> = Vec::new();
    for rel in idx.rs_files_under("packages/core/abi/src") {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        if let Some(cached) = idx.read_rust(&rel) {
            abi_files.push((rel, cached.prod_raw));
        }
    }
    assert!(
        !abi_files.is_empty(),
        "expected production Rust files under packages/core/abi/src"
    );

    let mut store_files: Vec<(String, String)> = Vec::new();
    for rel in idx.rs_files_under("packages/core/store/src") {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        if let Some(cached) = idx.read_rust(&rel) {
            store_files.push((rel, cached.prod_raw));
        }
    }

    let violations = scan_repr_c_abi_locks(&abi_files, &store_files);
    if !Stage::Post1B1D.is_enforced() {
        // Canary: on pre-migration `main`, `NveEvent` and `NodeRow` do NOT exist yet, so the
        // scanner MUST report missing locks for both (never false-pass on `NveEventBatch`!),
        // while `NveSurfaceApi`..`NveDevApi` MUST have zero false positives.
        assert!(
            violations.iter().any(|v| v.reason.contains("`NveEvent`")),
            "Expected missing cache-line lock violation for `NveEvent` on pre-1B-1d main"
        );
        assert!(
            !violations.iter().any(|v| v.snippet.contains("NveSurfaceApi")
                || v.snippet.contains("NveLayoutApi")
                || v.snippet.contains("NveComponentApi")
                || v.snippet.contains("NveDevApi")),
            "NveSurfaceApi..NveDevApi layout asserts in layout_asserts.rs must not be falsely flagged: {violations:?}"
        );
    }
    assert_staged_violations(
        Stage::Post1B1D,
        "#[repr(C)] ABI structs have compile-time layout assertions & cache-line locks (NodeRow=32, NveRenderNode=64, NveEvent=64)",
        &violations,
    );
}

pub fn scan_repr_c_abi_locks(
    abi_files: &[(String, String)],
    store_files: &[(String, String)],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut all_clean = String::new();
    let mut abi_structs: Vec<(String, ParsedStruct)> = Vec::new();
    let mut store_structs: Vec<(String, ParsedStruct)> = Vec::new();

    for (rel, prod_raw) in abi_files {
        let clean = strip_rust_comments_and_strings(&strip_cfg_test_items(prod_raw));
        all_clean.push_str(&clean);
        all_clean.push('\n');
        for st in parse_rust_structs(&clean) {
            abi_structs.push((rel.clone(), st));
        }
    }
    for (rel, prod_raw) in store_files {
        let clean = strip_rust_comments_and_strings(&strip_cfg_test_items(prod_raw));
        all_clean.push_str(&clean);
        all_clean.push('\n');
        for st in parse_rust_structs(&clean) {
            store_structs.push((rel.clone(), st));
        }
    }

    let compact_clean: String = all_clean.chars().filter(|c| !c.is_whitespace()).collect();
    let usize_consts = extract_usize_consts(&all_clean);

    // 1. Every `pub` `#[repr(C)]` struct in `nve-abi` must derive `NveAbi` OR have compile-time
    //    `size_of`, `align_of`, and `offset_of!` assertions for every named field in `all_clean`.
    for (rel, st) in &abi_structs {
        if !st.is_pub || !has_repr_token(&st.attrs, "C") {
            continue;
        }
        let has_derive_nve_abi = has_derive_token(&st.attrs, "NveAbi");
        if has_derive_nve_abi {
            continue;
        }
        let has_size = compact_clean.contains(&format!("size_of::<{}>()", st.name));
        let has_align = compact_clean.contains(&format!("align_of::<{}>()", st.name));
        let missing_offsets: Vec<&str> = if st.is_tuple {
            Vec::new()
        } else {
            st.fields
                .iter()
                .filter(|f| !compact_clean.contains(&format!("offset_of!({},{})", st.name, f.name)))
                .map(|f| f.name.as_str())
                .collect()
        };
        if !(has_size && has_align && missing_offsets.is_empty()) {
            violations.push(Violation::new(
                rel,
                st.line,
                format!("pub struct {}", st.name),
                format!(
                    "#[repr(C)] ABI struct must derive `NveAbi` or declare compile-time size_of/align_of/offset_of! assertions (size={has_size}, align={has_align}, missing_offsets={missing_offsets:?})"
                ),
            ));
        }
    }

    // 2. Cache-line size locks: require exact `ParsedStruct` match AND exact size lock.
    for (ty, expected_bytes) in [("NodeRow", 32usize), ("NveRenderNode", 64), ("NveEvent", 64)] {
        let exact_struct = abi_structs
            .iter()
            .chain(store_structs.iter())
            .find(|(_, st)| st.name == ty);
        let locked = exact_struct.is_some_and(|(_, st)| {
            has_nve_abi_size_attr(&st.attrs, expected_bytes)
                || has_const_size_of_eq(&compact_clean, &usize_consts, ty, expected_bytes)
        });
        if !locked {
            violations.push(Violation::new(
                "packages/core/abi/src",
                0,
                format!("expected exact struct `{ty}` with compile-time lock == {expected_bytes} bytes"),
                format!("Missing exact struct or compile-time cache-line size lock for `{ty}` ({expected_bytes} bytes)"),
            ));
        }
    }
    violations
}

pub fn has_derive_token(attrs: &[String], token: &str) -> bool {
    for attr in attrs {
        let Some(inner) = attr
            .trim()
            .strip_prefix("#[")
            .and_then(|s| s.strip_suffix(']'))
            .map(str::trim)
        else {
            continue;
        };
        let Some(rest) = inner.strip_prefix("derive") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(args) = rest.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            continue;
        };
        if args
            .split(',')
            .any(|p| p.trim().rsplit("::").next() == Some(token))
        {
            return true;
        }
    }
    false
}

fn has_nve_abi_size_attr(attrs: &[String], expected: usize) -> bool {
    for attr in attrs {
        let Some(inner) = attr
            .trim()
            .strip_prefix("#[")
            .and_then(|s| s.strip_suffix(']'))
            .map(str::trim)
        else {
            continue;
        };
        let Some(rest) = inner.strip_prefix("nve_abi") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(args) = rest.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            continue;
        };
        for item in args.split(',') {
            if let Some((k, v)) = item.split_once('=') {
                let k = k.trim();
                let v = v.trim().replace('_', "");
                if (k == "size" || k == "size_64") && v.parse::<usize>().ok() == Some(expected) {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_usize_consts(clean: &str) -> BTreeMap<String, usize> {
    let mut map = BTreeMap::new();
    for (_, stmt) in iter_keyword_statements(clean, "const", b';') {
        let body = stmt.trim_start_matches("const").trim_end_matches(';').trim();
        let Some((lhs, rhs)) = body.split_once('=') else {
            continue;
        };
        let Some((name, ty)) = lhs.split_once(':') else {
            continue;
        };
        if ty.trim() != "usize" {
            continue;
        }
        let val_str = rhs.trim().replace('_', "");
        if let Ok(val) = val_str.parse::<usize>() {
            map.insert(name.trim().to_string(), val);
        }
    }
    map
}

fn has_const_size_of_eq(
    compact_clean: &str,
    usize_consts: &BTreeMap<String, usize>,
    ty: &str,
    expected: usize,
) -> bool {
    let needle = format!("size_of::<{ty}>()==");
    let mut pos = 0;
    while let Some(idx) = compact_clean[pos..].find(&needle) {
        let start = pos + idx + needle.len();
        let rhs: String = compact_clean[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if rhs.replace('_', "").parse::<usize>().ok() == Some(expected) {
            return true;
        }
        if usize_consts.get(&rhs).copied() == Some(expected) {
            return true;
        }
        pos = start;
    }
    false
}

/// Enforces [INV:no-legacy-nodestore-symbols] and [INV:no-legacy-event-style-symbols].
#[test]
fn legacy_regression_bans_post_nodestore_and_1b1d() {
    let idx = repo_index();
    let (v_nodestore, scanned_ns) = scan_banned_symbols(BANNED_LEGACY_SYMBOLS_NODESTORE, true);
    let (v_1b1d, scanned_1d) = scan_banned_symbols(BANNED_LEGACY_SYMBOLS_1B1D, false);
    assert!(scanned_ns > 0 && scanned_1d > 0, "legacy scanners must scan > 0 files");
    let _ = idx;

    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            !v_nodestore.is_empty(),
            "Expected legacy NodeStore regression scanner to match pre-migration symbols on main"
        );
    }
    if !Stage::Post1B1D.is_enforced() {
        assert!(
            !v_1b1d.is_empty(),
            "Expected legacy 1B-1d event regression scanner to match pre-migration symbols on main"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero legacy NodeStore/ComponentKind/RenderSink/Any::downcast_ref regressions",
        &v_nodestore,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "Zero legacy NativeEventKind/NveEventPayloadTag/EventPayload/EventMask regressions",
        &v_1b1d,
    );
}

fn scan_banned_symbols(symbols: &[&str], check_downcast: bool) -> (Vec<Violation>, usize) {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for rel in idx.rs_files_under("packages") {
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let Some(cached) = idx.read_rust(&rel) else {
            continue;
        };
        scanned += 1;
        scan_banned_symbols_in_str(&rel, &cached.prod_raw, &cached.prod_clean, symbols, check_downcast, &mut violations);
    }
    (violations, scanned)
}

pub fn scan_banned_symbols_in_str(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    symbols: &[&str],
    check_downcast: bool,
    out: &mut Vec<Violation>,
) {
    for (idx, (orig, line)) in prod_raw.lines().zip(prod_clean.lines()).enumerate() {
        for &sym in symbols {
            let matched = if sym.contains(' ') || sym.contains("::") || sym.ends_with('_') {
                line.contains(sym)
            } else {
                contains_ident(line, sym)
            };
            if matched {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    orig,
                    format!("Legacy symbol `{sym}` is banned after migration"),
                ));
            }
        }
        if check_downcast
            && (line.contains("Any::downcast_ref")
                || line.contains(".downcast_ref::<")
                || line.contains(".downcast_ref("))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Per-node `Any::downcast_ref` is banned; use typed tables/handles",
            ));
        }
    }
}
