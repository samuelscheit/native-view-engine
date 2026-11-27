//! Staged Track 1B-1 / 1B-1d / 1B-1e invariants: zero FNV-1a, zero hardcoded schema hashes,
//! hot-path allocation & UTF-8 guards, codegen hygiene & 53-bit JS/Dart-web safety, platform
//! purity, banned 4-letter format acronyms, and core/backend benchmark presence.

use crate::content_purity::scan_core_table_string_tables;
use crate::support::{
    assert_no_violations, assert_staged_violations, contains_ident, extract_raw_string_spans,
    is_binary_ext, is_self_invariant_file, is_test_or_bench_rs, parse_rust_structs, repo_index,
    strip_rust_comments_and_strings, Violation, DOC_EXCLUDES,
};
use crate::Stage;

const BANNED_4_LETTER_ACRONYMS: &[&str] = &[
    "NVH1", "NVW1", "NVT1", "NVD1", "NVC1", "NVWS1", "NVB1", "NVNP", "NVNC", "NVERELOC", "NWS1",
    "NVE_MET1", "NVE1", "NVP1", "NK1",
];

/// Real existing hot-path production files always scanned today (plus `packages/core/store/src`).
/// Every listed file MUST exist so this scanner can never silently no-op.
const EXISTING_HOT_PATH_FILES: &[&str] = &[
    "packages/core/engine/src/layout.rs",
    "packages/core/wire-core/src/frame.rs",
    "packages/core/store/src/store.rs",
    "packages/core/store/src/tx.rs",
    "packages/core/store/src/span_pool.rs",
    "packages/backends/headless/src/taffy_store/mod.rs",
];

/// Enforces [INV:zero-fnv1a].
#[test]
fn zero_fnv1a_repo_wide() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || rel == "AGENTS.md"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        scanned += 1;
        for (line_idx, line) in content.lines().enumerate() {
            if line_has_fnv1a(line) {
                violations.push(Violation::new(
                    rel,
                    line_idx + 1,
                    line,
                    "FNV-1a is banned; standardize on `const fn xxh64` (`nve-hash` / `nve-abi`)",
                ));
            }
        }
    }
    assert!(scanned > 0, "zero_fnv1a_repo_wide must scan > 0 files");
    assert_no_violations(
        "Zero FNV-1a symbols or offset/prime constants repo-wide (use xxh64)",
        &violations,
    );
}

pub fn line_has_fnv1a(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| *c != '_').collect();
    compact.contains("fnv1a")
        || compact.contains("fnv-1a")
        || compact.contains("fnvhashmap")
        || compact.contains("fnvhasher")
        || compact.contains("cbf29ce484222325")
        || compact.contains("00000100000001b3")
        || compact.contains("0x100000001b3")
        || compact.contains("811c9dc5")
        || compact.contains("01000193")
        || compact.contains("14695981039346656037")
        || compact.contains("1099511628211")
        || compact.contains("2166136261")
        || compact.contains("16777619")
}

#[test]
fn zero_hardcoded_schema_hashes_in_rust_and_mdx() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for rel in &idx.tracked {
        if is_self_invariant_file(rel) {
            continue;
        }
        if rel.ends_with(".rs") {
            let Some(cached) = idx.read_rust(rel) else {
                continue;
            };
            scanned += 1;
            scan_hardcoded_schema_hashes_rs(rel, &cached.raw, &cached.all_clean, &mut violations);
        } else if rel.starts_with("docs/content/") && rel.ends_with(".mdx") {
            let Some(content) = idx.read_text(rel) else {
                continue;
            };
            scanned += 1;
            for (line_idx, line) in content.lines().enumerate() {
                if has_16_hex_literal(line) {
                    violations.push(Violation::new(
                        rel,
                        line_idx + 1,
                        line,
                        "Raw 16-hex-digit schema hash in docs/content/**/*.mdx goes stale; reference the symbol name instead",
                    ));
                }
            }
        }
    }
    assert!(scanned > 0, "zero_hardcoded_schema_hashes must scan > 0 files");
    assert_no_violations(
        "Zero hardcoded `*_SCHEMA_HASH = 0x...` in Rust and zero raw 16-hex hashes in docs/content/**/*.mdx",
        &violations,
    );
}

/// Scans multi-line `const ... SCHEMA_HASH ... = ...;` statements joined up to `;`.
pub fn scan_hardcoded_schema_hashes_rs(
    rel: &str,
    raw: &str,
    clean: &str,
    out: &mut Vec<Violation>,
) {
    let raw_lines: Vec<&str> = raw.lines().collect();
    let bytes = clean.as_bytes();
    let mut pos = 0usize;
    while let Some(rel_idx) = clean[pos..].find("SCHEMA_HASH") {
        let abs = pos + rel_idx;
        let line_no = clean[..abs].bytes().filter(|&b| b == b'\n').count() + 1;
        let mut end = abs + "SCHEMA_HASH".len();
        while end < bytes.len() && bytes[end] != b';' {
            end += 1;
        }
        let stmt = &clean[abs..end];
        if let Some((_, rhs)) = stmt.split_once('=') {
            let rhs_trim = rhs.trim_start();
            if rhs_trim.starts_with("0x") || rhs_trim.starts_with("0X") {
                out.push(Violation::new(
                    rel,
                    line_no,
                    raw_lines.get(line_no.saturating_sub(1)).copied().unwrap_or(""),
                    "Hardcoded `*_SCHEMA_HASH = 0x...` hex literal is banned; compute via `const fn xxh64`",
                ));
            }
        }
        pos = (end + 1).min(bytes.len());
    }
}

pub fn has_16_hex_literal(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 18 <= bytes.len() {
        if bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            let mut j = i + 2;
            let mut digits = 0usize;
            while j < bytes.len() && (bytes[j].is_ascii_hexdigit() || bytes[j] == b'_') {
                if bytes[j].is_ascii_hexdigit() {
                    digits += 1;
                }
                j += 1;
            }
            if digits == 16 {
                return true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    false
}

/// Enforces [INV:no-noderow-heap-fields].
#[test]
fn hot_path_perf_and_utf8_guards() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for &candidate in EXISTING_HOT_PATH_FILES {
        let cached = idx.read_rust(candidate).unwrap_or_else(|| {
            panic!("Expected hot-path file `{candidate}` to exist — update EXISTING_HOT_PATH_FILES if moved")
        });
        scanned += 1;
        scan_hot_path_str(candidate, &cached.prod_raw, true, &mut violations);
    }

    // Scan per-node heap containers across core/{store,engine,abi,tables,components} + backends/*
    // AND hot read/utf8 rules across packages/core/store/src.
    for prefix in [
        "packages/core/store/src",
        "packages/core/engine/src",
        "packages/core/abi/src",
        "packages/core/tables/src",
        "packages/core/components/src",
        "packages/backends",
    ] {
        for rel in idx.rs_files_under(prefix) {
            if is_test_or_bench_rs(&rel) || EXISTING_HOT_PATH_FILES.contains(&rel.as_str()) {
                continue;
            }
            if let Some(cached) = idx.read_rust(&rel) {
                scanned += 1;
                let is_hot = rel.starts_with("packages/core/store/src/")
                    && (rel.contains("row_writer")
                        || rel.contains("transaction")
                        || rel.contains("str_arena")
                        || rel.contains("string_arena")
                        || rel.contains("span_pool")
                        || rel.contains("dirty"));
                scan_hot_path_str(&rel, &cached.prod_raw, is_hot, &mut violations);
            }
        }
    }

    assert!(
        scanned >= EXISTING_HOT_PATH_FILES.len(),
        "hot_path_perf_and_utf8_guards must scan real files"
    );
    assert_no_violations(
        "Zero hot-path allocations/UTF-8 re-validation/NodeRow heap containers in core & backends",
        &violations,
    );
}

pub fn scan_hot_path_str(rel: &str, src: &str, is_hot_file: bool, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    let in_per_node_scope = rel.starts_with("packages/core/store/")
        || rel.starts_with("packages/core/engine/")
        || rel.starts_with("packages/core/abi/")
        || rel.starts_with("packages/core/tables/")
        || rel.starts_with("packages/core/components/")
        || rel.starts_with("packages/backends/");

    if in_per_node_scope {
        for st in parse_rust_structs(src) {
            let is_per_node = st.name == "NodeRow"
                || st.name.ends_with("Node")
                || matches!(
                    st.name.as_str(),
                    "NodeData" | "NodeColdData" | "NodeEntry" | "NodeRecord"
                );
            if is_per_node {
                for f in &st.fields {
                    if f.ty.contains("HashMap")
                        || f.ty.contains("Box<")
                        || f.ty.contains("Vec<NodeId>")
                    {
                        out.push(Violation::new(
                            rel,
                            f.line,
                            format!("{}: {}", f.name, f.ty),
                            format!(
                                "Per-node struct `{}` must stay flat/SoA without `{}` heap container",
                                st.name, f.ty
                            ),
                        ));
                    }
                }
            }
        }
    }

    let check_utf8 = (rel.starts_with("packages/core/store/src/")
        && !rel.contains("wire")
        && !rel.contains("ingress"))
        || is_hot_file;

    let hot_bans = [
        (".to_ascii_lowercase()", "allocates String on hot path; use `eq_ignore_ascii_case`"),
        ("format!(", "allocates String on hot path"),
        ("vec![", "heap allocation on hot path; reuse arena/scratch buffer"),
        (".to_vec()", "heap clone on hot path"),
        (".to_string()", "heap String allocation on hot path"),
        ("String::new()", "heap String allocation on hot path"),
        (".sort_unstable(", "sorting dirty list on hot path; use bitset/generation stamp"),
        (".dedup(", "deduping dirty list on hot path; use bitset/generation stamp"),
    ];
    let mut brace_depth = 0usize;
    let mut pending_cold_fn = false;
    let mut cold_fn_exit_depth: Option<usize> = None;

    for (idx, (orig, line)) in src.lines().zip(clean.lines()).enumerate() {
        let is_allowlisted_cold_fn = ((rel == "packages/core/store/src/store.rs"
            || rel == "packages/backends/headless/src/taffy_store/mod.rs")
            && line.contains("fn new("))
            || (rel == "packages/core/store/src/span_pool.rs" && line.contains("fn try_str("))
            || (rel == "packages/core/wire-core/src/frame.rs" && line.contains("fn to_owned("));
        if cold_fn_exit_depth.is_none() && is_allowlisted_cold_fn {
            pending_cold_fn = true;
        }
        let in_cold_fn = cold_fn_exit_depth.is_some() || pending_cold_fn;

        if check_utf8 && !in_cold_fn && line.contains("from_utf8(") && !line.contains("debug_assert")
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "`from_utf8` is banned in hot read/commit/layout paths; seal UTF-8 at ingress and use `from_utf8_unchecked` on reads",
            ));
        }
        if is_hot_file && !in_cold_fn {
            for (pat, why) in hot_bans {
                if line.contains(pat) {
                    out.push(Violation::new(
                        rel,
                        idx + 1,
                        orig,
                        format!("Hot-path ban `{pat}`: {why}"),
                    ));
                }
            }
        }

        for ch in line.chars() {
            if ch == '{' {
                if pending_cold_fn {
                    cold_fn_exit_depth = Some(brace_depth);
                    pending_cold_fn = false;
                }
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth = brace_depth.saturating_sub(1);
                if cold_fn_exit_depth == Some(brace_depth) {
                    cold_fn_exit_depth = None;
                }
            } else if ch == ';' && pending_cold_fn {
                pending_cold_fn = false;
            }
        }
    }
}

/// Enforces [INV:codegen-hygiene].
#[test]
fn codegen_hygiene_and_dart_js_53bit_safety() {
    let idx = repo_index();
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for rel in idx.rs_files_under("packages/core/codegen/src/targets") {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let Some(cached) = idx.read_rust(&rel) else {
            continue;
        };
        scanned += 1;
        scan_codegen_target_str(&rel, &cached.prod_raw, &mut violations);
        // Ban legacy target `wire.rs` and `reloc*` emitters post-1B-1e.
        let file = rel.rsplit('/').next().unwrap_or("");
        if file == "wire.rs" || file.starts_with("reloc") {
            violations.push(Violation::new(
                &rel,
                1,
                &rel,
                "Legacy codegen target (`wire.rs` / `reloc*`) must be deleted once replaced by Track 1B-1e templates",
            ));
        }
    }
    assert!(scanned > 0, "codegen hygiene scanner must scan > 0 target files");

    for legacy_path in [
        "packages/core/codegen/src/schema/wire.rs",
        "packages/core/codegen/src/schema/reloc",
        "packages/core/codegen/src/frontend/hybrid_rust",
        "packages/core/codegen/src/schema/scene_abi.rs",
    ] {
        if idx.root.join(legacy_path).exists() {
            violations.push(Violation::new(
                legacy_path,
                1,
                legacy_path,
                "Legacy duplicated schema/lexer path must be deleted in Track 1B-1e (`schema_bridge.rs` stays as the live NveAbi bridge)",
            ));
        }
    }

    if !Stage::Post1B1E.is_enforced() {
        assert!(
            !violations.is_empty(),
            "Expected codegen hygiene scanner to detect pre-1B-1e raw template blocks on main"
        );
    }
    assert_staged_violations(
        Stage::Post1B1E,
        "Codegen hygiene (no multi-line raw string blocks, no legacy wire/reloc/hybrid_rust duplicates, 53-bit Dart-web/JS int safety)",
        &violations,
    );
}

pub fn scan_codegen_target_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let lines: Vec<&str> = src.lines().collect();

    // 1. Lexer-based scan: ANY raw string literal (`r"..."`, `r#"..."#`, `r##"..."##`) spanning
    //    >= 8 lines in codegen target emitters (catches writeln!, push_str, format!, const TPL).
    for (start_line, _end_line, span) in extract_raw_string_spans(src) {
        if span >= 8 {
            out.push(Violation::new(
                rel,
                start_line,
                lines.get(start_line.saturating_sub(1)).copied().unwrap_or(""),
                format!(
                    "Multi-line raw target-language string ({span} lines) in codegen emitter is banned; move static code to checked-in files or templates"
                ),
            ));
        }
    }

    // 2. Dart-web / TypeScript 53-bit JS integer safety.
    let is_dart = rel.contains("/dart/");
    let is_ts = rel.contains("/typescript/");
    if is_dart || is_ts {
        for (idx, &line) in lines.iter().enumerate() {
            if is_dart {
                let is_dart_decl = line.contains("const int ")
                    || line.contains("static const int ")
                    || line.contains("final int ")
                    || (line.contains("const ") && line.contains('=') && !line.contains("BigInt"));
                if is_dart_decl
                    && (line.contains("0x{") || has_16_hex_literal(line))
                    && !line.contains("BigInt")
                {
                    out.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        "Unsafe 64-bit integer constant in Dart-web binding (JS Number precision is 53 bits); use `(lo: u32, hi: u32)` or `BigInt`",
                    ));
                }
            }
            if is_ts {
                let unsafe_ts_64 = (line.contains("Number(") && line.contains("getBigUint64"))
                    || line.contains("4294967296")
                    || (line.contains("<< 32") && !line.contains("BigInt") && !line.contains("32n"));
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
    }
}

/// Enforces [INV:core-platform-purity] and [INV:no-legacy-acronyms].
#[test]
fn platform_purity_and_banned_4_letter_acronyms() {
    let idx = repo_index();
    let mut v_purity = Vec::new();
    let mut scanned_core = 0usize;

    for rel in idx.rs_files_under("packages/core") {
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let Some(cached) = idx.read_rust(&rel) else {
            continue;
        };
        scanned_core += 1;
        scan_platform_purity_str(&rel, &cached.prod_raw, &cached.prod_clean, &mut v_purity);
    }
    assert!(scanned_core > 0, "platform purity scanner must scan > 0 core files");

    let mut v_acronyms = Vec::new();
    let mut scanned_acr = 0usize;
    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        scanned_acr += 1;
        scan_banned_acronyms_str(rel, &content, &mut v_acronyms);
    }
    assert!(scanned_acr > 0, "banned acronym scanner must scan > 0 tracked files");

    if !Stage::Post1B1D.is_enforced() {
        assert!(
            !v_purity.is_empty(),
            "Expected platform purity scanner to find pre-1B-1d CSS/HTML formatters in packages/core/components"
        );
        assert!(
            !v_acronyms.is_empty(),
            "Expected banned-acronym scanner to detect pre-cleanup 4-letter acronyms on main"
        );
    }
    assert_staged_violations(
        Stage::Post1B1D,
        "Platform purity: zero CSS/HTML formatters in packages/core/* and zero motion in packages/core/tables",
        &v_purity,
    );
    assert_staged_violations(
        Stage::Post1B1D,
        "Zero banned cryptic 4-letter format acronyms outside docs/CHANGELOG.md",
        &v_acronyms,
    );
}

pub fn scan_platform_purity_str(
    rel: &str,
    prod_raw: &str,
    prod_clean: &str,
    out: &mut Vec<Violation>,
) {
    let in_no_motion_core = rel.starts_with("packages/core/tables/")
        || rel.starts_with("packages/core/components/")
        || rel.starts_with("packages/core/engine/")
        || rel.starts_with("packages/core/runtime/");
    if in_no_motion_core && rel.contains("motion") {
        out.push(Violation::new(
            rel,
            1,
            rel,
            "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never in core crates",
        ));
    }
    for (idx, (orig, line)) in prod_raw.lines().zip(prod_clean.lines()).enumerate() {
        if line.contains("fn write_css")
            || line.contains("fn as_css")
            || line.contains("fn as_html")
            || line.contains("fn to_css_keyword")
            || line.contains("fn write_html")
            || line.contains("fn format_css")
            || line.contains("fn parse_box_shadow")
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Web CSS/HTML formatters/parsers belong in `nve-backend-web`, never `packages/core/*`",
            ));
        }
        if in_no_motion_core
            && (line.contains("mod motion") || contains_ident(line, "MotionClock"))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never in core crates",
            ));
        }
    }
    scan_core_table_string_tables(rel, prod_raw, prod_clean, out);
}

pub fn scan_banned_acronyms_str(rel: &str, content: &str, out: &mut Vec<Violation>) {
    for (idx, line) in content.lines().enumerate() {
        for &acr in BANNED_4_LETTER_ACRONYMS {
            if contains_ident(line, acr) {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    format!("Banned legacy 4-letter format acronym `{acr}`; use clear self-documenting names"),
                ));
            }
        }
    }
}
