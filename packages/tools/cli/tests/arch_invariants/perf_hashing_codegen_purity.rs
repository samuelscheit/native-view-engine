//! Staged Track 1B-1 / 1B-1d / 1B-1e invariants: zero FNV-1a, zero hardcoded schema hashes,
//! hot-path allocation & UTF-8 guards, codegen hygiene & 53-bit JS/Dart-web safety, platform
//! purity, banned 4-letter format acronyms, and core/backend benchmark presence.

use std::fs;
use std::path::Path;

use crate::support::{
    assert_staged_violations, contains_ident, find_files_named, find_rs_files, git_tracked_files,
    is_binary_ext, is_self_invariant_file, is_test_or_bench_rs, parse_cargo_toml,
    parse_rust_structs, production_code_prefix, rel_path, repo_root,
    strip_rust_comments_and_strings, Violation,
};
use crate::Stage;

const BANNED_4_LETTER_ACRONYMS: &[&str] = &[
    "NVH1", "NVW1", "NVT1", "NVD1", "NVC1", "NVWS1", "NVB1", "NVNP", "NVNC", "NVERELOC",
];

#[test]
fn zero_fnv1a_repo_wide() {
    let root = repo_root();
    let violations = scan_fnv1a(&root);
    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            !violations.is_empty(),
            "Expected FNV-1a scanner to detect pre-migration FNV-1a usage on main"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero FNV-1a symbols or offset/prime constants repo-wide (use xxh64)",
        &violations,
    );
}

fn scan_fnv1a(root: &Path) -> Vec<Violation> {
    let tracked = git_tracked_files(root);
    let mut violations = Vec::new();
    for rel in &tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || rel == "docs/CHANGELOG.md"
            || rel == "docs/BACKLOG.md"
            || rel == "AGENTS.md"
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Ok(content) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            if line_has_fnv1a(line) {
                violations.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    "FNV-1a is banned; standardize on `const fn xxh64` (`nve-hash` / `nve-abi`)",
                ));
            }
        }
    }
    violations
}

fn line_has_fnv1a(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| *c != '_').collect();
    compact.contains("fnv1a")
        || compact.contains("fnv-1a")
        || compact.contains("cbf29ce484222325")
        || compact.contains("00000100000001b3")
        || compact.contains("0x100000001b3")
}

#[test]
fn zero_hardcoded_schema_hashes_in_rust_and_mdx() {
    let root = repo_root();
    let violations = scan_hardcoded_schema_hashes(&root);
    if !Stage::PostNodeStore.is_enforced() {
        assert!(
            !violations.is_empty(),
            "Expected schema hash scanner to detect existing hardcoded hashes on main"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero hardcoded `*_SCHEMA_HASH = 0x...` in Rust and zero raw 16-hex hashes in docs/content/**/*.mdx",
        &violations,
    );
}

fn scan_hardcoded_schema_hashes(root: &Path) -> Vec<Violation> {
    let tracked = git_tracked_files(root);
    let mut violations = Vec::new();
    for rel in &tracked {
        if is_self_invariant_file(rel) {
            continue;
        }
        if rel.ends_with(".rs") {
            let Ok(content) = fs::read_to_string(root.join(rel)) else {
                continue;
            };
            let clean = strip_rust_comments_and_strings(&content);
            for (idx, (orig, line)) in content.lines().zip(clean.lines()).enumerate() {
                if has_hardcoded_schema_hash_const(line) {
                    violations.push(Violation::new(
                        rel,
                        idx + 1,
                        orig,
                        "Hardcoded `*_SCHEMA_HASH = 0x...` hex literal is banned; compute via `const fn xxh64`",
                    ));
                }
            }
        } else if rel.starts_with("docs/content/") && rel.ends_with(".mdx") {
            let Ok(content) = fs::read_to_string(root.join(rel)) else {
                continue;
            };
            for (idx, line) in content.lines().enumerate() {
                if has_16_hex_literal(line) {
                    violations.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        "Raw 16-hex-digit schema hash in docs/content/**/*.mdx goes stale; reference the symbol name instead",
                    ));
                }
            }
        }
    }
    violations
}

fn has_hardcoded_schema_hash_const(line: &str) -> bool {
    let Some(pos) = line.find("SCHEMA_HASH") else {
        return false;
    };
    let rest = &line[pos + "SCHEMA_HASH".len()..];
    let Some((_, rhs)) = rest.split_once('=') else {
        return false;
    };
    rhs.trim_start().starts_with("0x") || rhs.trim_start().starts_with("0X")
}

fn has_16_hex_literal(line: &str) -> bool {
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

#[test]
fn hot_path_perf_and_utf8_guards() {
    let root = repo_root();
    let violations = scan_hot_path_perf(&root);
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero hot-path allocations/UTF-8 re-validation/NodeRow heap containers in nve-store & hot loops",
        &violations,
    );
}

fn scan_hot_path_perf(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    let store_src = root.join("packages/core/store/src");
    if store_src.exists() {
        for file in find_rs_files(&store_src) {
            let rel = rel_path(root, &file);
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let content = fs::read_to_string(&file).unwrap_or_default();
            scan_hot_path_str(&rel, production_code_prefix(&content), &mut violations);
        }
    }
    // Also inspect known hot-path layout/wire files in core & backends.
    for candidate in [
        "packages/core/engine/src/taffy_layout.rs",
        "packages/core/wire-core/src/frame.rs",
    ] {
        let full = root.join(candidate);
        if let Ok(content) = fs::read_to_string(&full) {
            scan_hot_path_str(candidate, production_code_prefix(&content), &mut violations);
        }
    }
    violations
}

fn scan_hot_path_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    let is_str_arena = rel.contains("str_arena") || rel.contains("string_arena");
    let is_hot_file = rel.contains("row_writer")
        || rel.contains("transaction")
        || is_str_arena
        || rel.contains("span_pool")
        || rel.contains("taffy_layout")
        || rel.contains("dirty");

    // 1. Check per-node structs in `nve-store` for `HashMap`, `Box<`, `Vec<NodeId>`.
    if rel.starts_with("packages/core/store/") {
        for st in parse_rust_structs(src) {
            if st.name == "NodeRow" || st.name.ends_with("Node") {
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

    // 2. Check `from_utf8` in StrArena reads and allocation/sort anti-patterns in hot files.
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
    for (idx, (orig, line)) in src.lines().zip(clean.lines()).enumerate() {
        if is_str_arena && line.contains("from_utf8(") {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "`from_utf8` is banned in `StrArena` reads; seal UTF-8 at ingress and use `from_utf8_unchecked` on reads",
            ));
        }
        if is_hot_file {
            for (pat, why) in hot_bans {
                if line.contains(pat) {
                    out.push(Violation::new(rel, idx + 1, orig, format!("Hot-path ban `{pat}`: {why}")));
                }
            }
        }
    }
}

#[test]
fn codegen_hygiene_and_dart_js_53bit_safety() {
    let root = repo_root();
    let violations = scan_codegen_hygiene(&root);
    if !Stage::Post1B1E.is_enforced() {
        assert!(
            !violations.is_empty(),
            "Expected codegen hygiene scanner to detect pre-1B-1e raw writeln! templates on main"
        );
    }
    assert_staged_violations(
        Stage::Post1B1E,
        "Codegen hygiene (no multi-line writeln! raw blocks, no duplicated schemas, 53-bit Dart-web/JS int safety)",
        &violations,
    );
}

fn scan_codegen_hygiene(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    let targets_dir = root.join("packages/core/codegen/src/targets");
    if targets_dir.exists() {
        for file in find_rs_files(&targets_dir) {
            let rel = rel_path(root, &file);
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let content = fs::read_to_string(&file).unwrap_or_default();
            scan_codegen_target_str(&rel, production_code_prefix(&content), &mut violations);
        }
    }

    // Duplicated hand-written schema bridge check (`schema_bridge.rs` / `core_abi.rs` hand specs).
    for legacy_schema in [
        "packages/core/codegen/src/schema/hybrid/schema_bridge.rs",
        "packages/core/codegen/src/schema/wire.rs",
    ] {
        if root.join(legacy_schema).exists() {
            violations.push(Violation::new(
                legacy_schema,
                1,
                legacy_schema,
                "Hand-duplicated schema file must be deleted once replaced by colocated `#[derive(NveAbi)]`/`#[derive(NveTable)]`",
            ));
        }
    }
    violations
}

fn scan_codegen_target_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let lines: Vec<&str> = src.lines().collect();
    // 1. Detect multi-line raw strings inside `writeln!(..., r#"..."#)` spanning >= 8 lines.
    for (idx, &line) in lines.iter().enumerate() {
        if line.contains("writeln!(") || line.contains("write!(") {
            let window = lines[idx..lines.len().min(idx + 3)].join(" ");
            if window.contains("r#\"") {
                let mut span = 1usize;
                for end_line in &lines[idx + 1..] {
                    span += 1;
                    if end_line.contains("\"#") {
                        break;
                    }
                }
                if span >= 8 {
                    out.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        format!(
                            "Multi-line raw target-language block ({span} lines) inside `writeln!` is banned; move static code to checked-in files or Askama/MiniJinja templates"
                        ),
                    ));
                }
            }
        }
    }

    // 2. Dart-web / TypeScript 53-bit JS integer safety: reject unguarded 64-bit hex literals without BigInt/lo+hi split.
    let is_js_or_dart = rel.contains("/typescript/") || rel.contains("/dart/");
    if is_js_or_dart {
        for (idx, &line) in lines.iter().enumerate() {
            let has_unsafe_dart_64 = rel.contains("/dart/")
                && line.contains("const int ")
                && (line.contains("0x{") || has_16_hex_literal(line))
                && !line.contains("BigInt");
            let has_unsafe_ts_64 = rel.contains("/typescript/")
                && (line.contains("number") && (line.contains("getBigUint64") || line.contains("<< 32")))
                && !line.contains("BigInt");
            if has_unsafe_dart_64 || has_unsafe_ts_64 {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    line,
                    "Unsafe 64-bit integer op/constant in Dart-web/JS binding (JS Number precision is 53 bits); use `(lo: u32, hi: u32)` or `BigInt`",
                ));
            }
        }
    }
}

#[test]
fn platform_purity_and_banned_4_letter_acronyms() {
    let root = repo_root();
    let v_purity = scan_platform_purity(&root);
    let v_acronyms = scan_banned_acronyms(&root);

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

fn scan_platform_purity(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    for file in find_rs_files(&root.join("packages/core")) {
        let rel = rel_path(root, &file);
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let content = fs::read_to_string(&file).unwrap_or_default();
        let prod = production_code_prefix(&content);
        let clean = strip_rust_comments_and_strings(prod);
        for (idx, (orig, line)) in prod.lines().zip(clean.lines()).enumerate() {
            if line.contains("fn write_css")
                || line.contains("fn as_css")
                || line.contains("fn as_html")
            {
                violations.push(Violation::new(
                    &rel,
                    idx + 1,
                    orig,
                    "Web CSS/HTML formatters (`write_css*`, `as_css`, `as_html*`) belong in `nve-backend-web`, never `packages/core/*`",
                ));
            }
            if rel.starts_with("packages/core/tables/")
                && (rel.contains("motion") || line.contains("mod motion") || contains_ident(line, "MotionClock"))
            {
                violations.push(Violation::new(
                    &rel,
                    idx + 1,
                    orig,
                    "Stateful runtime engine `motion` must live in standalone `packages/modules/motion/`, never `packages/core/tables/`",
                ));
            }
        }
    }
    violations
}

fn scan_banned_acronyms(root: &Path) -> Vec<Violation> {
    let tracked = git_tracked_files(root);
    let mut violations = Vec::new();
    for rel in &tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || rel == "docs/CHANGELOG.md"
            || rel == "docs/BACKLOG.md"
            || rel == "docs/quality-audit.md"
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Ok(content) = fs::read_to_string(root.join(rel)) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            for &acr in BANNED_4_LETTER_ACRONYMS {
                if contains_ident(line, acr) {
                    violations.push(Violation::new(
                        rel,
                        idx + 1,
                        line,
                        format!("Banned legacy 4-letter format acronym `{acr}`; use clear self-documenting names"),
                    ));
                }
            }
        }
    }
    violations
}

#[test]
fn core_and_backend_crates_have_benchmarks() {
    let root = repo_root();
    let violations = scan_benchmark_presence(&root);
    assert_staged_violations(
        Stage::PostNodeStore,
        "Every core and backend crate ships Criterion benchmarks",
        &violations,
    );
}

fn scan_benchmark_presence(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    for layer in ["packages/core", "packages/backends"] {
        for manifest in find_files_named(&root.join(layer), "Cargo.toml") {
            let crate_dir = manifest.parent().expect("Cargo.toml has parent");
            let rel_dir = rel_path(root, crate_dir);
            // Proc-macro crates cannot run standalone runtime benches.
            if rel_dir.ends_with("abi-macros") {
                continue;
            }
            let content = fs::read_to_string(&manifest).unwrap_or_default();
            let parsed = parse_cargo_toml(&content);
            let benches_dir = crate_dir.join("benches");
            let has_bench_rs = benches_dir.exists() && !find_rs_files(&benches_dir).is_empty();
            if !parsed.has_bench_table && !has_bench_rs {
                violations.push(Violation::new(
                    rel_path(root, &manifest),
                    1,
                    rel_dir,
                    "Core/backend crate must ship benchmarks (`[[bench]]` or `benches/*.rs`)",
                ));
            }
        }
    }
    violations
}

#[test]
fn unit_perf_hash_codegen_scanners_catch_exact_violations() {
    assert!(line_has_fnv1a("const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;"));
    assert!(line_has_fnv1a("fn fnv1a64(b: &[u8]) -> u64"));
    assert!(!line_has_fnv1a("pub const fn xxh64(b: &[u8]) -> u64"));

    assert!(has_hardcoded_schema_hash_const("pub const NVE_FOO_SCHEMA_HASH: u64 = 0x1234_5678;"));
    assert!(!has_hardcoded_schema_hash_const("pub const NVE_FOO_SCHEMA_HASH: u64 = xxh64(b\"foo\");"));
    assert!(has_16_hex_literal("Schema hash is `0x6D45E53E0D9535D1`."));
    assert!(!has_16_hex_literal("Service id is `0x0010`."));

    let mut v_hot = Vec::new();
    scan_hot_path_str(
        "packages/core/store/src/str_arena.rs",
        "pub struct NodeRow { pub children: Vec<NodeId> }\nfn get(&self) { let _ = std::str::from_utf8(b); let _ = format!(\"x\"); }",
        &mut v_hot,
    );
    assert_eq!(v_hot.len(), 3, "expected NodeRow Vec + from_utf8 + format!: {v_hot:?}");

    let mut v_cg = Vec::new();
    scan_codegen_target_str(
        "packages/core/codegen/src/targets/dart/wire.rs",
        "writeln!(s, r#\"\n1\n2\n3\n4\n5\n6\n7\n8\"#);\nconst int foo = 0x6D45E53E0D9535D1;",
        &mut v_cg,
    );
    assert_eq!(v_cg.len(), 2, "expected multiline writeln + unsafe dart 64-bit const: {v_cg:?}");
}
