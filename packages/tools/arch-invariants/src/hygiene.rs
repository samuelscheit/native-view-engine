//! Repository hygiene invariants: open-only BACKLOG, generated-file naming & zero tracked
//! artifacts, `Zero-JSON` ban, personal paths/endpoints ratchet, crate `README.md` + doctest
//! hook enforcement, and the monotonic 650-line production `.rs` ratchet across `packages/` + `hosts/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::spanned::Spanned;

use crate::{
    find_files_named, is_binary_ext, is_cfg_test_attr, is_self_invariant_file,
    is_test_or_bench_rs, rel_path, RepoIndex, Violation, DOC_EXCLUDES,
};

pub const MAX_PROD_LINES: usize = 650;

/// Strictly sorted monotonic allowlist of production `.rs` files > 650 lines across `packages/`, `hosts/` and `examples/`.
/// Fails if any listed file exceeds its ceiling OR drops to <= 650 lines (forcing ratchet downward to zero).
pub const EXISTING_OVERSIZE_ALLOWLIST: &[(&str, usize)] = &[];

/// Monotonic ratchet of pre-existing personal-path hits: `(path, max_hits)` (empty since Track 1B-1e).
const PENDING_CLEANUP_PERSONAL_PATHS: &[(&str, usize)] = &[];

const PERSONAL_SCAN_EXTS: &[&str] = &[
    ".rs", ".ts", ".js", ".mjs", ".swift", ".kt", ".kts", ".java", ".dart", ".cpp", ".hpp", ".h",
    ".c", ".mm", ".sh", ".py", ".toml", ".json", ".yml", ".yaml", ".properties", ".mdx", ".md",
];

pub fn check_backlog_open_only(idx: &RepoIndex) -> Vec<Violation> {
    let content = idx
        .read_text("docs/BACKLOG.md")
        .expect("docs/BACKLOG.md must exist");
    let mut violations = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("- [x]")
            || t.starts_with("- [X]")
            || t.starts_with("* [x]")
            || t.starts_with("* [X]")
        {
            violations.push(Violation::new(
                "docs/BACKLOG.md",
                i + 1,
                line,
                "docs/BACKLOG.md must stay open-only; move completed items to docs/CHANGELOG.md",
            ));
        }
    }
    violations
}

/// Askama templates of `nve-codegen`: they contain the `@generated` banner of their output.
const CODEGEN_TEMPLATES_DIR: &str = "packages/core/codegen/templates/";

pub fn check_generated_files_and_artifacts(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rel in &idx.tracked {
        if rel.contains(".generated.") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Generated `*.generated.*` files must be gitignored and never tracked",
            ));
        }
        if rel.contains("dev/nve/host/generated/") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Generated Java files under `dev/nve/host/generated/` must never be tracked",
            ));
        }
        if rel.ends_with(".trace.json") || rel.ends_with(".perfetto.json") {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Trace artifacts (`*.trace.json` / `*.perfetto.json`) must never be tracked",
            ));
        }
        let is_host_build = rel.starts_with("hosts/")
            && rel
                .split('/')
                .any(|seg| matches!(seg, "build" | ".build" | ".gradle" | ".dart_tool"));
        if rel.starts_with("cxx/build") || is_host_build {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                "Build directories (`cxx/build*`, `hosts/**/{build,.build,.gradle,.dart_tool}`) must never be tracked",
            ));
        }

        if is_binary_ext(rel) || rel == "Cargo.lock" || is_self_invariant_file(rel) {
            continue;
        }
        if let Some(content) = idx.read_text(rel) {
            for (i, line) in content.lines().take(25).enumerate() {
                let t = line.trim();
                let is_comment = t.starts_with("//")
                    || t.starts_with("/*")
                    || t.starts_with('*')
                    || t.starts_with('#')
                    || t.starts_with("<!--");
                if is_comment && t.contains("@generated") {
                    // Codegen templates carry the banner they emit; they are sources, not output.
                    if rel.starts_with(CODEGEN_TEMPLATES_DIR) {
                        break;
                    }
                    let in_java_gen = rel.contains("dev/nve/host/generated/");
                    let has_dot_gen = Path::new(rel)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.contains(".generated."));
                    if !in_java_gen && !has_dot_gen {
                        violations.push(Violation::new(
                            rel,
                            i + 1,
                            line,
                            "Non-Java `@generated` file must include `.generated.` in its filename",
                        ));
                    }
                    break;
                }
            }
        }
    }
    violations
}

pub fn check_zero_json_buzzword(idx: &RepoIndex) -> Vec<Violation> {
    let needle = ["zero", "-", "json"].concat();
    let mut violations = Vec::new();
    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        if let Some(content) = idx.read_text(rel) {
            for (i, line) in content.lines().enumerate() {
                if contains_ascii_ci(line, &needle) {
                    violations.push(Violation::new(
                        rel,
                        i + 1,
                        line,
                        "Marketing buzzword `Zero-JSON` is banned in manifests, code, and active docs",
                    ));
                }
            }
        }
    }
    violations
}

pub fn check_personal_endpoints_or_paths(idx: &RepoIndex) -> Vec<Violation> {
    let pending: BTreeMap<&str, usize> = PENDING_CLEANUP_PERSONAL_PATHS.iter().copied().collect();
    let mut pending_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut violations = Vec::new();

    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || is_self_invariant_file(rel)
            || rel == "docs/CHANGELOG.md"
            || !PERSONAL_SCAN_EXTS.iter().any(|ext| rel.ends_with(ext))
        {
            continue;
        }
        let is_test_or_doc = is_test_or_bench_rs(rel)
            || rel.contains("/tests/")
            || rel.contains("examples/")
            || rel.ends_with(".md")
            || rel.ends_with(".mdx");
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        let cfg_test_ranges = if !is_test_or_doc && rel.ends_with(".rs") && content.contains("test")
        {
            syn::parse_file(&content)
                .map(|ast| {
                    if is_cfg_test_attr(&ast.attrs) {
                        vec![(1, usize::MAX)]
                    } else {
                        collect_cfg_test_line_ranges(&ast.items)
                    }
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for (i, line) in content.lines().enumerate() {
            let line_1based = i + 1;
            let in_cfg_test = is_test_or_doc
                || cfg_test_ranges
                    .iter()
                    .any(|&(s, e)| line_1based >= s && line_1based <= e);
            if let Some(reason) = detect_personal_endpoint_or_path(line, in_cfg_test) {
                if let Some(&max_hits) = pending.get(rel.as_str()) {
                    let count = pending_counts.entry(rel.clone()).or_insert(0);
                    *count += 1;
                    if *count > max_hits {
                        violations.push(Violation::new(
                            rel,
                            i + 1,
                            line,
                            format!("Personal-path hits in `{rel}` grew to {count} (ceiling {max_hits}): {reason}"),
                        ));
                    }
                } else {
                    violations.push(Violation::new(rel, i + 1, line, reason));
                }
            }
        }
    }

    for &(p, max_hits) in PENDING_CLEANUP_PERSONAL_PATHS {
        let actual = pending_counts.get(p).copied().unwrap_or(0);
        if actual < max_hits {
            violations.push(Violation::new(
                p,
                0,
                format!("actual={actual}, allowlist={max_hits}"),
                "PENDING_CLEANUP_PERSONAL_PATHS entry dropped below ceiling — lower or delete it",
            ));
        }
    }
    violations
}

pub fn detect_personal_endpoint_or_path(line: &str, is_test_or_doc: bool) -> Option<&'static str> {
    if contains_ascii_ci(line, ".ts.net")
        || contains_ascii_ci(line, "ngrok.io")
        || contains_ascii_ci(line, "trycloudflare.com")
    {
        return Some("Hardcoded personal tunnel/Tailscale endpoint is forbidden");
    }
    if has_user_home_prefix(line, "/Users/")
        || has_user_home_prefix(line, "/home/")
        || ["C:\\Users\\", "c:\\Users\\", "C:/Users/", "c:/Users/"]
            .iter()
            .any(|p| has_user_home_prefix(line, p))
    {
        return Some("Hardcoded local user home directory path is forbidden");
    }
    if !is_test_or_doc && contains_rfc1918_ip(line) {
        return Some("Hardcoded private RFC1918 IP address in production code is forbidden");
    }
    None
}

/// Checks every `packages/**` crate has `README.md` (>= 5 non-empty lines) and checks whether
/// every library crate's `src/lib.rs` includes `doc = include_str!("../README.md")` under `doctest`.
pub fn check_crate_readmes_and_doctest_hooks(idx: &RepoIndex) -> (Vec<Violation>, Vec<Violation>) {
    let manifests = find_files_named(&idx.root.join("packages"), "Cargo.toml");
    assert!(!manifests.is_empty(), "expected crates under packages/");
    let mut v_readme = Vec::new();
    let mut v_doctest = Vec::new();

    for manifest in manifests {
        let crate_dir = manifest.parent().expect("Cargo.toml has parent dir");
        let readme = crate_dir.join("README.md");
        let rel = rel_path(&idx.root, &readme);
        match idx.read_text(&rel) {
            None => v_readme.push(Violation::new(
                rel,
                0,
                "",
                "Every workspace crate under packages/ must have a factual README.md",
            )),
            Some(content) => {
                let non_empty = content.lines().filter(|l| !l.trim().is_empty()).count();
                if non_empty < 5 {
                    v_readme.push(Violation::new(
                        rel,
                        1,
                        format!("{non_empty} non-empty lines"),
                        "Crate README.md must have >= 5 non-empty lines",
                    ));
                }
            }
        }

        let lib_rs = crate_dir.join("src/lib.rs");
        if lib_rs.is_file() {
            let lib_rel = rel_path(&idx.root, &lib_rs);
            let has_hook = idx
                .parse_rust(&lib_rel)
                .is_some_and(|parsed| has_readme_doctest_attr(&parsed.ast));
            if !has_hook {
                v_doctest.push(Violation::new(
                    lib_rel,
                    1,
                    "missing #![cfg_attr(doctest, doc = include_str!(\"../README.md\"))]",
                    "Every `packages/**` library crate must wire README.md into rustdoc doctests",
                ));
            }
        }
    }
    (v_readme, v_doctest)
}

fn cfg_meta_mentions_doctest(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(p) => p.is_ident("doctest"),
        syn::Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| items.iter().any(cfg_meta_mentions_doctest)),
        _ => false,
    }
}

fn has_readme_doctest_attr(ast: &syn::File) -> bool {
    ast.attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg_attr") {
            return false;
        }
        let Ok(items) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        let mut iter = items.iter();
        let Some(cond) = iter.next() else {
            return false;
        };
        if !cfg_meta_mentions_doctest(cond) {
            return false;
        }
        iter.any(|m| {
            matches!(
                m,
                syn::Meta::NameValue(nv)
                    if nv.path.is_ident("doc")
                        && matches!(
                            &nv.value,
                            syn::Expr::Macro(em)
                                if em.mac.path.is_ident("include_str")
                                    && em.mac.tokens.to_string().replace(' ', "") == "\"../README.md\""
                        )
            )
        })
    })
}

/// Monotonic 650-line production `.rs` file ratchet across `packages/`, `hosts/` and `examples/`.
pub fn check_production_file_sizes(idx: &RepoIndex) -> Vec<Violation> {
    assert!(
        EXISTING_OVERSIZE_ALLOWLIST.windows(2).all(|w| w[0].0 < w[1].0),
        "EXISTING_OVERSIZE_ALLOWLIST must stay strictly sorted"
    );
    let allowlist: BTreeMap<&str, usize> = EXISTING_OVERSIZE_ALLOWLIST.iter().copied().collect();
    let mut still_oversize = BTreeSet::new();
    let mut violations = Vec::new();

    let mut rs_files = idx.rs_files_under("packages");
    rs_files.extend(idx.rs_files_under("hosts"));
    rs_files.extend(idx.rs_files_under("examples"));
    assert!(!rs_files.is_empty(), "expected Rust files under packages/, hosts/ and examples/");

    for rel in rs_files {
        if is_test_or_bench_rs(&rel) || rel.contains(".generated.") {
            continue;
        }
        let Some(content) = idx.read_text(&rel) else {
            continue;
        };
        let lines = content.lines().count();
        if lines > MAX_PROD_LINES {
            if let Some(&max_lines) = allowlist.get(rel.as_str()) {
                still_oversize.insert(rel.clone());
                if lines > max_lines {
                    violations.push(Violation::new(
                        rel,
                        0,
                        format!("{lines} lines (ratchet ceiling {max_lines})"),
                        "Allowlisted oversize file grew past its monotonic ceiling; split instead of growing",
                    ));
                }
            } else {
                violations.push(Violation::new(
                    rel,
                    0,
                    format!("{lines} lines (limit {MAX_PROD_LINES})"),
                    "Production Rust file exceeds 650 lines; split into focused submodules",
                ));
            }
        }
    }

    for &(listed, _) in EXISTING_OVERSIZE_ALLOWLIST {
        if !still_oversize.contains(listed) {
            violations.push(Violation::new(
                listed,
                0,
                listed,
                "Allowlisted file is now <= 650 lines — delete from EXISTING_OVERSIZE_ALLOWLIST",
            ));
        }
    }
    violations
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

fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let nb = needle.as_bytes();
    !nb.is_empty()
        && haystack
            .as_bytes()
            .windows(nb.len())
            .any(|w| w.eq_ignore_ascii_case(nb))
}

fn has_user_home_prefix(line: &str, prefix: &str) -> bool {
    let mut pos = 0;
    while let Some(idx) = line[pos..].find(prefix) {
        let rest = &line[pos + idx + prefix.len()..];
        let seg: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .collect();
        if !seg.is_empty() && !matches!(seg.as_str(), "Shared" | "runner" | "user_placeholder") {
            return true;
        }
        pos += idx + prefix.len();
    }
    false
}

fn collect_cfg_test_line_ranges(items: &[syn::Item]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    for item in items {
        let attrs = match item {
            syn::Item::Mod(m) => Some(&m.attrs),
            syn::Item::Fn(f) => Some(&f.attrs),
            syn::Item::Struct(s) => Some(&s.attrs),
            syn::Item::Enum(e) => Some(&e.attrs),
            syn::Item::Trait(t) => Some(&t.attrs),
            syn::Item::Macro(m) => Some(&m.attrs),
            syn::Item::Type(t) => Some(&t.attrs),
            syn::Item::Impl(i) => Some(&i.attrs),
            syn::Item::Const(c) => Some(&c.attrs),
            syn::Item::Static(s) => Some(&s.attrs),
            _ => None,
        };
        if attrs.is_some_and(|a| is_cfg_test_attr(a)) {
            let span = item.span();
            ranges.push((span.start().line, span.end().line));
        } else if let syn::Item::Mod(m) = item
            && let Some((_, sub)) = &m.content
        {
            ranges.extend(collect_cfg_test_line_ranges(sub));
        }
    }
    ranges
}

fn contains_rfc1918_ip(line: &str) -> bool {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if !bytes[i].is_ascii_digit()
            || (i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.'))
        {
            continue;
        }
        let end = bytes[i..]
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b'.')
            .count()
            + i;
        let octets: Vec<u16> = line[i..end]
            .split('.')
            .filter_map(|s| (!s.is_empty()).then(|| s.parse::<u16>().ok()).flatten())
            .collect();
        if octets.len() == 4 && octets.iter().all(|o| *o <= 255) {
            let (a, b) = (octets[0], octets[1]);
            if a == 10 || (a == 192 && b == 168) || (a == 172 && (16..=31).contains(&b)) {
                return true;
            }
        }
    }
    false
}
