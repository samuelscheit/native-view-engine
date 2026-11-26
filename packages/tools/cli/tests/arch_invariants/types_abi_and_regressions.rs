//! Staged Track 1B-1 / 1B-1d invariants: typed domain fields, sealed arenas, `#[repr(C)]` ABI
//! layout locks, legacy regression bans, and zero-boilerplate macro & converter hygiene.

use std::fs;
use std::path::Path;

use crate::support::{
    assert_staged_violations, contains_ident, find_rs_files, is_self_invariant_file,
    is_test_or_bench_rs, parse_rust_structs, production_code_prefix, rel_path, repo_root,
    strip_rust_comments_and_strings, Violation,
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
];

#[test]
fn bare_primitive_domain_fields_in_core_structs() {
    let root = repo_root();
    let violations = scan_bare_primitive_domain_fields(&root);
    if !Stage::PostNodeStore.is_enforced() {
        // Verify real scanner detects known pre-migration bare primitive fields in packages/core/abi today.
        assert!(
            !violations.is_empty(),
            "Expected bare primitive domain field scanner to find pre-NodeStore fields in packages/core/abi"
        );
    }
    assert_staged_violations(
        Stage::PostNodeStore,
        "Zero bare primitive domain fields in packages/core/{abi,store,engine,tables} pub structs",
        &violations,
    );
}

fn scan_bare_primitive_domain_fields(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    for dir in ["abi", "store", "engine", "tables"] {
        let base = root.join("packages/core").join(dir).join("src");
        if !base.exists() {
            continue;
        }
        for file in find_rs_files(&base) {
            let rel = rel_path(root, &file);
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let content = fs::read_to_string(&file).unwrap_or_default();
            scan_bare_primitive_domain_fields_in_str(&rel, production_code_prefix(&content), &mut violations);
        }
    }
    violations
}

fn scan_bare_primitive_domain_fields_in_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let lines: Vec<&str> = src.lines().collect();
    for st in parse_rust_structs(src) {
        if !st.is_pub {
            continue;
        }
        // Skip transparent newtype wrappers (single-field wrappers aren't braced anyway, or #[repr(transparent)])
        if st.attrs.iter().any(|a| a.contains("transparent")) {
            continue;
        }
        for field in &st.fields {
            let is_domain_name = DOMAIN_FIELD_EXACT.contains(&field.name.as_str())
                || DOMAIN_FIELD_SUFFIXES.iter().any(|s| field.name.ends_with(s));
            if is_domain_name && is_bare_primitive(&field.ty) {
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

fn is_bare_primitive(ty: &str) -> bool {
    let norm = ty.trim();
    if BARE_PRIMITIVES.contains(&norm) {
        return true;
    }
    if let Some(inner) = norm.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        return BARE_PRIMITIVES.contains(&inner.trim());
    }
    false
}

#[test]
fn untyped_span_and_transparent_arena_alias_guard() {
    let root = repo_root();
    let violations = scan_untyped_span_and_arena_aliases(&root);
    assert_staged_violations(
        Stage::PostNodeStore,
        "No untyped `Span` or transparent `pub type StrArena = SpanPool<u8>` aliases",
        &violations,
    );
}

fn scan_untyped_span_and_arena_aliases(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    for dir in ["abi", "store", "engine", "tables"] {
        let base = root.join("packages/core").join(dir).join("src");
        if !base.exists() {
            continue;
        }
        for file in find_rs_files(&base) {
            let rel = rel_path(root, &file);
            if is_test_or_bench_rs(&rel) {
                continue;
            }
            let content = fs::read_to_string(&file).unwrap_or_default();
            scan_untyped_span_in_str(&rel, production_code_prefix(&content), &mut violations);
        }
    }
    violations
}

fn scan_untyped_span_in_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    for (idx, (orig, line)) in src.lines().zip(clean.lines()).enumerate() {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("pubtypeStrArena=SpanPool<u8>")
            || compact.contains("typeStrArena=SpanPool<u8>")
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "StrArena must be a sealed UTF-8 newtype, never a transparent `SpanPool<u8>` alias",
            ));
        }
        if (compact.contains("pubstructSpan{") || compact.contains("pubstructSpan("))
            && !compact.contains("Span<")
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Untyped `pub struct Span` is banned; use typed `Span<T>` / `Utf8Span` handles",
            ));
        }
    }
}

#[test]
fn repr_c_abi_layout_asserts_and_cache_line_locks() {
    let root = repo_root();
    let violations = scan_repr_c_abi_locks(&root);
    assert_staged_violations(
        Stage::Post1B1D,
        "#[repr(C)] ABI structs have compile-time layout assertions & cache-line locks (NodeRow=32, NveRenderNode=64, NveEvent=64)",
        &violations,
    );
}

fn scan_repr_c_abi_locks(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    let abi_src = root.join("packages/core/abi/src");
    let mut all_abi_text = String::new();
    for file in find_rs_files(&abi_src) {
        let rel = rel_path(root, &file);
        if is_test_or_bench_rs(&rel) {
            continue;
        }
        let content = fs::read_to_string(&file).unwrap_or_default();
        all_abi_text.push_str(&content);
        all_abi_text.push('\n');
        scan_repr_c_structs_in_file(&rel, &content, &mut violations);
    }

    // Cache-line size locks across core/{abi,store}.
    let store_src = root.join("packages/core/store/src");
    let mut all_core_text = all_abi_text;
    if store_src.exists() {
        for file in find_rs_files(&store_src) {
            if let Ok(c) = fs::read_to_string(&file) {
                all_core_text.push_str(&c);
                all_core_text.push('\n');
            }
        }
    }
    let compact: String = all_core_text.chars().filter(|c| !c.is_whitespace()).collect();
    for (ty, bytes) in [("NodeRow", "32"), ("NveRenderNode", "64"), ("NveEvent", "64")] {
        let has_lock = compact.contains(&format!("size_of::<{ty}>()=={bytes}"))
            || compact.contains(&format!("size={bytes}"))
                && compact.contains(&format!("struct{ty}"));
        if !has_lock {
            violations.push(Violation::new(
                "packages/core/abi/src",
                0,
                format!("expected compile-time lock size_of::<{ty}>() == {bytes}"),
                format!("Missing compile-time cache-line size lock for `{ty}` ({bytes} bytes)"),
            ));
        }
    }
    violations
}

fn scan_repr_c_structs_in_file(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let compact: String = src.chars().filter(|c| !c.is_whitespace()).collect();
    for st in parse_rust_structs(production_code_prefix(src)) {
        if !st.is_pub {
            continue;
        }
        let is_repr_c = st.attrs.iter().any(|a| a.contains("repr(") && a.contains('C'));
        if !is_repr_c {
            continue;
        }
        let has_derive_nve_abi = st.attrs.iter().any(|a| a.contains("NveAbi"));
        let has_size_assert = compact.contains(&format!("size_of::<{}>()", st.name));
        let has_align_assert = compact.contains(&format!("align_of::<{}>()", st.name));
        let has_offset_assert =
            st.fields.is_empty() || compact.contains(&format!("offset_of!({},", st.name));
        if !has_derive_nve_abi && !(has_size_assert && has_align_assert && has_offset_assert) {
            out.push(Violation::new(
                rel,
                st.line,
                format!("pub struct {}", st.name),
                "#[repr(C)] ABI struct must derive `NveAbi` or declare compile-time `size_of`, `align_of`, and `offset_of!` assertions",
            ));
        }
    }
}

#[test]
fn legacy_regression_bans_post_nodestore_and_1b1d() {
    let root = repo_root();
    let v_nodestore = scan_banned_symbols(&root, BANNED_LEGACY_SYMBOLS_NODESTORE, true);
    let v_1b1d = scan_banned_symbols(&root, BANNED_LEGACY_SYMBOLS_1B1D, false);

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

fn scan_banned_symbols(root: &Path, symbols: &[&str], check_downcast: bool) -> Vec<Violation> {
    let mut violations = Vec::new();
    for file in find_rs_files(&root.join("packages")) {
        let rel = rel_path(root, &file);
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let content = fs::read_to_string(&file).unwrap_or_default();
        let prod = production_code_prefix(&content);
        let clean = strip_rust_comments_and_strings(prod);
        for (idx, (orig, line)) in prod.lines().zip(clean.lines()).enumerate() {
            for &sym in symbols {
                let matched = if sym.contains(' ') || sym.contains("::") || sym.ends_with('_') {
                    line.contains(sym)
                } else {
                    contains_ident(line, sym)
                };
                if matched {
                    violations.push(Violation::new(
                        &rel,
                        idx + 1,
                        orig,
                        format!("Legacy symbol `{sym}` is banned after migration"),
                    ));
                }
            }
            if check_downcast
                && (line.contains("Any::downcast_ref") || line.contains(".downcast_ref::<"))
            {
                violations.push(Violation::new(
                    &rel,
                    idx + 1,
                    orig,
                    "Per-node `Any::downcast_ref` is banned; use typed tables/handles",
                ));
            }
        }
    }
    violations
}

#[test]
fn zero_boilerplate_macros_and_converters() {
    let root = repo_root();
    let violations = scan_macro_and_converter_hygiene(&root);
    assert_staged_violations(
        Stage::Post1B1D,
        "Zero-boilerplate WordValue/NveTable macros, no Tier-1 byte codecs, no numeric String round-trips",
        &violations,
    );
}

fn scan_macro_and_converter_hygiene(root: &Path) -> Vec<Violation> {
    let mut violations = Vec::new();
    for file in find_rs_files(&root.join("packages")) {
        let rel = rel_path(root, &file);
        if is_test_or_bench_rs(&rel) || is_self_invariant_file(&rel) {
            continue;
        }
        let content = fs::read_to_string(&file).unwrap_or_default();
        scan_macro_and_converter_str(&rel, production_code_prefix(&content), &mut violations);
    }
    violations
}

fn scan_macro_and_converter_str(rel: &str, src: &str, out: &mut Vec<Violation>) {
    let clean = strip_rust_comments_and_strings(src);
    let in_store = rel.starts_with("packages/core/store/");
    let in_dioxus = rel.starts_with("packages/core/dioxus/");
    let lines: Vec<&str> = src.lines().collect();

    // 1. Manual `impl WordValue` outside nve-store
    if !in_store {
        for (idx, (orig, line)) in src.lines().zip(clean.lines()).enumerate() {
            if line.contains("impl WordValue for ") || line.contains("impl nve_store::WordValue for ") {
                out.push(Violation::new(
                    rel,
                    idx + 1,
                    orig,
                    "Manual `impl WordValue` outside `nve-store` is banned; derive via `#[derive(WordEnum)]`, `word_flags!`, or `#[derive(NveTable)]`",
                ));
            }
        }
    }

    // 2. Inspect `#[derive(NveTable)]` structs for redundant `#[bits(N)]`, sentinel defaults, split `*_enabled: bool`.
    for st in parse_rust_structs(src) {
        let is_table = st.attrs.iter().any(|a| a.contains("NveTable"));
        if !is_table {
            continue;
        }
        for field in &st.fields {
            let snippet = lines.get(field.line.saturating_sub(1)).copied().unwrap_or("");
            if field.attrs.iter().any(|a| a.contains("bits("))
                && matches!(field.ty.as_str(), "bool" | "u8" | "u16")
            {
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    format!(
                        "Redundant `#[bits(...)]` on `{}: {}` in #[derive(NveTable)]; bit width is derived from `<T as WordValue>::BITS`",
                        field.name, field.ty
                    ),
                ));
            }
            if field.attrs.iter().any(|a| {
                a.contains("default(u16::MAX)") || a.contains("default(f32::NAN)")
            }) {
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    "Magic sentinel `#[default(u16::MAX)]`/`#[default(f32::NAN)]` in NveTable is banned; use `Opt<T>`",
                ));
            }
            if field.ty == "bool" && field.name.ends_with("_enabled") {
                out.push(Violation::new(
                    rel,
                    field.line,
                    snippet,
                    format!(
                        "Split boolean sentinel `{}` in NveTable is banned; fold into `Opt<T>`",
                        field.name
                    ),
                ));
            }
        }
    }

    // 3. Manual `SET_*` constants in core tables, manual `encode_bytes`/`to_le_bytes` on Tier-1 repr(C) PODs,
    //    and `f64`/`i64` -> `String` -> parse round-trips in nve-dioxus.
    let has_repr_c = clean.contains("#[repr(C");
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
        if has_repr_c && rel.starts_with("packages/core/") && line.contains("fn encode_bytes(") {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Hand-written `encode_bytes` on Tier 1 `#[repr(C)]` POD is banned; pass typed memory or derive `#[derive(NveCodec)]`",
            ));
        }
        if in_dioxus
            && ((line.contains(".to_string()") && (line.contains("f64") || line.contains("i64")))
                || (line.contains("AttributeValue::Float(") && line.contains("format!("))
                || (line.contains("AttributeValue::Int(") && line.contains("format!(")))
        {
            out.push(Violation::new(
                rel,
                idx + 1,
                orig,
                "Numeric RSX `f64`/`i64` -> `String` -> parse round-trip in `nve-dioxus` is banned; write typed numeric value directly",
            ));
        }
    }
}

#[test]
fn unit_type_and_macro_scanners_catch_exact_violations() {
    let sample_bad = r#"
        #[derive(NveTable)]
        pub struct BadTable {
            #[bits(1)]
            pub tint_enabled: bool,
            #[default(f32::NAN)]
            pub opacity: f32,
            pub node_id: u32,
            pub parent: u64,
        }
        impl WordValue for BadTable {}
        pub type StrArena = SpanPool<u8>;
        pub struct Span { pub off: u32, pub len: u32 }
        #[repr(C)]
        pub struct UncheckedAbi { pub x: u32 }
    "#;
    let mut v_fields = Vec::new();
    scan_bare_primitive_domain_fields_in_str("synthetic.rs", sample_bad, &mut v_fields);
    assert_eq!(v_fields.len(), 2, "expected node_id + parent violations: {v_fields:?}");

    let mut v_span = Vec::new();
    scan_untyped_span_in_str("synthetic.rs", sample_bad, &mut v_span);
    assert_eq!(v_span.len(), 2, "expected StrArena alias + untyped Span: {v_span:?}");

    let mut v_abi = Vec::new();
    scan_repr_c_structs_in_file("synthetic.rs", sample_bad, &mut v_abi);
    assert_eq!(v_abi.len(), 1, "expected unchecked #[repr(C)] violation: {v_abi:?}");

    let mut v_macro = Vec::new();
    scan_macro_and_converter_str("packages/core/tables/src/bad.rs", sample_bad, &mut v_macro);
    assert_eq!(v_macro.len(), 4, "expected WordValue + bits + sentinel + _enabled: {v_macro:?}");

    let sample_clean = r#"
        #[derive(NveAbi)]
        #[repr(C)]
        pub struct CleanAbi {
            pub node_id: NodeId,
            pub parent: Option<NodeId>,
            pub width: f32,
        }
    "#;
    let mut v_clean = Vec::new();
    scan_bare_primitive_domain_fields_in_str("clean.rs", sample_clean, &mut v_clean);
    scan_repr_c_structs_in_file("clean.rs", sample_clean, &mut v_clean);
    assert!(v_clean.is_empty(), "clean code must have 0 violations: {v_clean:?}");
}
