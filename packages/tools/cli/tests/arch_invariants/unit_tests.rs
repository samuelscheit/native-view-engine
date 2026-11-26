//! Comprehensive positive AND clean-code negative unit tests for the `rust_lex`, `json_meta`,
//! `WorkspaceGraph`, `extract_impl_fn_body`, and staged architectural invariant scanners.

use crate::arch_inv::crate_dag::{
    check_core_backend_module_deps, check_core_zero_dioxus, check_core_zero_platform_crates,
    check_packages_to_hosts_deps, check_zero_fnv_crate_dep, check_zero_serde_json,
    extract_impl_fn_body, scan_web_backend_new_body,
};
use crate::arch_inv::dag_and_hygiene::detect_personal_endpoint_or_path;
use crate::arch_inv::bench_and_alloc::check_bench_not_stub;
use crate::arch_inv::perf_hashing_codegen_purity::{
    has_16_hex_literal, line_has_fnv1a, scan_banned_acronyms_str, scan_codegen_target_str,
    scan_hardcoded_schema_hashes_rs, scan_hot_path_str, scan_platform_purity_str,
};
use crate::arch_inv::enum_and_macro_hygiene::{
    scan_duplicate_enums_and_converters, scan_macro_and_converter_str,
};
use crate::arch_inv::types_abi_and_regressions::{
    has_repr_token, scan_banned_symbols_in_str, scan_bare_primitive_domain_fields_in_str,
    scan_repr_c_abi_locks, scan_untyped_span_in_str,
};
use crate::json_meta::WorkspaceGraph;
use crate::rust_lex::{
    extract_raw_string_spans, parse_rust_structs, strip_cfg_test_items,
    strip_rust_comments_and_strings,
};
use crate::support::{is_test_or_bench_rs, Violation};

#[test]
fn unit_lexer_comments_raw_c_strings_and_utf8_char_literals() {
    let src = r####"
        /* outer /* nested */ still comment */
        let a = r###"inside raw " with ##"###;
        let b = cr##"c raw "# ok"##;
        let c = c"c string";
        let q = '\'' | '"';
        let dash = '—';
        let euro = '€';
        fn real_fn<'a>(x: &'a str) -> &'a str { x }
    "####;
    let clean = strip_rust_comments_and_strings(src);
    assert_eq!(src.lines().count(), clean.lines().count());
    assert!(!clean.contains("nested"));
    assert!(!clean.contains("inside raw"));
    assert!(!clean.contains("c raw"));
    assert!(!clean.contains("c string"));
    assert!(clean.contains("fn real_fn<'a>(x: &'a str) -> &'a str { x }"));

    let spans = extract_raw_string_spans("const A: &str = r#\"\n1\n2\n3\n\"#;\nlet b = r\"one\";");
    assert_eq!(spans, vec![(1, 5, 5), (6, 6, 1)]);
}

#[test]
fn unit_strip_cfg_test_items_preserves_subsequent_production_code_and_crlf() {
    let src = concat!(
        "#[cfg(test)]\r\n",
        "mod tests;\r\n",
        "\r\n",
        "pub struct RealAfterTopTestMod {\r\n",
        "    pub node_id: u32,\r\n",
        "}\r\n",
        "\r\n",
        "impl RealAfterTopTestMod {\r\n",
        "    #[cfg(\r\n",
        "        all(test, feature = \"extra\")\r\n",
        "    )]\r\n",
        "    fn test_only_helper() { let x = 1; }\r\n",
        "\r\n",
        "    #[cfg(not(test))]\r\n",
        "    pub fn prod_only_fn() -> u32 { 42 }\r\n",
        "}\r\n",
    );
    let stripped = strip_cfg_test_items(src);
    assert_eq!(src.lines().count(), stripped.lines().count());
    assert!(!stripped.contains("mod tests;"));
    assert!(!stripped.contains("test_only_helper"));
    assert!(stripped.contains("pub struct RealAfterTopTestMod"));
    assert!(stripped.contains("pub node_id: u32"));
    assert!(stripped.contains("pub fn prod_only_fn() -> u32 { 42 }"));
}

#[test]
fn unit_parse_rust_structs_inline_mod_vis_where_and_tuple() {
    let src = r#"
        pub(crate) struct CratePrivate { pub node_id: u32 }
        #[repr(align(64), C)]
        pub struct Complex<F> where F: Fn(u8) -> Result<u32, u64> {
            pub cb: F,
            pub node_id: u32,
        }
        mod inner {
            #[repr(C)]
            pub struct TupleNodeId(pub u32);
        }
    "#;
    let structs = parse_rust_structs(src);
    assert_eq!(structs.len(), 3, "got: {structs:?}");
    assert_eq!(structs[0].name, "CratePrivate");
    assert!(!structs[0].is_pub, "pub(crate) must not be treated as bare pub");
    assert_eq!(structs[1].name, "Complex");
    assert!(structs[1].is_pub);
    assert!(has_repr_token(&structs[1].attrs, "C"));
    assert_eq!(structs[1].fields.len(), 2);
    assert_eq!(structs[2].name, "TupleNodeId");
    assert!(structs[2].is_pub && structs[2].is_tuple);
    assert_eq!(structs[2].fields[0].ty, "u32");
}

#[test]
fn unit_is_test_or_bench_rs_does_not_exclude_production_hit_test() {
    assert!(!is_test_or_bench_rs("packages/backends/skia/src/hit_test.rs"));
    assert!(is_test_or_bench_rs("packages/backends/skia/src/hit_tests.rs"));
    assert!(is_test_or_bench_rs("packages/backends/skia/src/tests.rs"));
    assert!(is_test_or_bench_rs("packages/backends/skia/tests/foo.rs"));
    assert!(is_test_or_bench_rs("packages/tools/bench/benches/abi.rs"));
}

#[test]
fn unit_workspace_graph_catches_inherited_renamed_transitive_and_host_deps() {
    let synthetic_meta = r#"{
        "workspace_root": "/repo",
        "workspace_members": [
            "path+file:///repo/packages/core/engine#0.1.0",
            "path+file:///repo/packages/core/helper#0.1.0",
            "path+file:///repo/packages/tools/extra#0.1.0",
            "path+file:///repo/packages/modules/cam#0.1.0",
            "path+file:///repo/hosts/skia-shell#0.1.0"
        ],
        "packages": [
            {
                "id": "path+file:///repo/packages/core/engine#0.1.0",
                "name": "nve-engine",
                "manifest_path": "/repo/packages/core/engine/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "nve-helper", "kind": null, "optional": false, "target": null, "path": "/repo/packages/core/helper" },
                    { "name": "dioxus-core", "rename": "dx_alias", "kind": "build", "optional": false, "target": null, "path": null },
                    { "name": "nve-cam", "kind": null, "optional": true, "target": null, "path": "/repo/packages/modules/cam" }
                ]
            },
            {
                "id": "path+file:///repo/packages/core/helper#0.1.0",
                "name": "nve-helper",
                "manifest_path": "/repo/packages/core/helper/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "serde_json", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "web-sys", "kind": null, "optional": false, "target": null, "path": null },
                    { "name": "fnv", "kind": null, "optional": false, "target": null, "path": null }
                ]
            },
            {
                "id": "path+file:///repo/packages/tools/extra#0.1.0",
                "name": "nve-extra-tool",
                "manifest_path": "/repo/packages/tools/extra/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": [
                    { "name": "nve-skia-shell", "kind": null, "optional": false, "target": null, "path": "/repo/hosts/skia-shell" }
                ]
            },
            {
                "id": "path+file:///repo/packages/modules/cam#0.1.0",
                "name": "nve-cam",
                "manifest_path": "/repo/packages/modules/cam/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": []
            },
            {
                "id": "path+file:///repo/hosts/skia-shell#0.1.0",
                "name": "nve-skia-shell",
                "manifest_path": "/repo/hosts/skia-shell/Cargo.toml",
                "targets": [],
                "features": {},
                "dependencies": []
            }
        ],
        "resolve": { "nodes": [] }
    }"#;
    let graph = WorkspaceGraph::parse_metadata_json(synthetic_meta).expect("valid synthetic metadata");
    assert!(!check_core_zero_dioxus(&graph).is_empty(), "must catch renamed build dep dioxus-core");
    assert!(!check_core_zero_platform_crates(&graph).is_empty(), "must catch transitive web-sys");
    assert!(
        check_zero_serde_json(&graph)
            .iter()
            .any(|v| v.path.contains("core/engine")),
        "must catch transitive serde_json in nve-engine"
    );
    assert!(!check_zero_fnv_crate_dep(&graph).is_empty(), "must catch transitive fnv dep");
    assert!(!check_core_backend_module_deps(&graph).is_empty(), "must catch optional core->module dep");
    assert!(
        check_packages_to_hosts_deps(&graph)
            .iter()
            .any(|v| v.snippet.contains("nve-extra-tool -> nve-skia-shell")),
        "must catch packages/tools/** -> hosts/** violation"
    );
}

#[test]
fn unit_extract_impl_fn_body_ignores_comment_braces_and_clean_frame_dirty_svgs() {
    let src = r#"
        impl WebBackend {
            // comment with } brace and "nve_camera"
            pub fn new() -> Self {
                let s = "register_module { }";
                Self { frame_dirty_svgs: Vec::new() }
            }
        }
    "#;
    let (line, orig, clean) =
        extract_impl_fn_body(src, "WebBackend", "pub fn new(").expect("must find WebBackend::new");
    let mut v_clean = Vec::new();
    scan_web_backend_new_body("web.rs", line, &orig, &clean, &mut v_clean);
    assert!(v_clean.is_empty(), "frame_dirty_svgs & string literals must not trigger: {v_clean:?}");

    let bad_src = r#"
        impl WebBackend {
            pub fn new() -> Self {
                Self::register_module(nve_camera::spec());
            }
        }
    "#;
    let (line, orig, clean) =
        extract_impl_fn_body(bad_src, "WebBackend", "pub fn new(").expect("must find bad new");
    let mut v_bad = Vec::new();
    scan_web_backend_new_body("web.rs", line, &orig, &clean, &mut v_bad);
    assert_eq!(v_bad.len(), 2, "expected register_module + nve_camera: {v_bad:?}");
}

#[test]
fn unit_personal_endpoint_and_path_detector() {
    assert!(detect_personal_endpoint_or_path("let p = \"/Users/alice/dev\";", false).is_some());
    assert!(detect_personal_endpoint_or_path("let p = \"C:\\Users\\alice\\repo\";", false).is_some());
    assert!(detect_personal_endpoint_or_path("let p = \"C:/Users/alice/repo\";", false).is_some());
    assert!(detect_personal_endpoint_or_path("https://box.tail1234.ts.net", false).is_some());
    assert!(detect_personal_endpoint_or_path("192.168.1.42:8080", false).is_some());
    assert!(detect_personal_endpoint_or_path("192.168.1.42:8080", true).is_none());
    assert!(detect_personal_endpoint_or_path("127.0.0.1:8080", false).is_none());
}

#[test]
fn unit_type_abi_macro_and_regression_scanners_positive_and_negative() {
    let bad = r#"
        #[derive(NveTable)]
        pub struct BadTable {
            #[bits(1)]
            pub tint_enabled: bool,
            #[default(f32::NAN)]
            pub opacity: f32,
            pub node_id: core::option::Option<u32>,
            pub parent_ids: Vec<u64>,
            pub slot_id: [u16; 2],
        }
        impl<T> crate::WordValue for BadTable {}
        type StrArena =
            crate::SpanPool<u8>;
        struct Span { off: u32 }
        #[repr(align(64), C)]
        pub struct UncheckedAbi { pub x: u32 }
    "#;
    let mut v_fields = Vec::new();
    scan_bare_primitive_domain_fields_in_str("bad.rs", bad, &mut v_fields);
    assert_eq!(v_fields.len(), 2, "expected Option<u32> + [u16; 2]: {v_fields:?}");

    let mut v_span = Vec::new();
    scan_untyped_span_in_str("bad.rs", bad, &mut v_span);
    assert_eq!(v_span.len(), 2, "expected multiline StrArena + private Span: {v_span:?}");

    let mut v_macro = Vec::new();
    scan_macro_and_converter_str("packages/core/tables/src/bad.rs", bad, &mut v_macro);
    assert_eq!(v_macro.len(), 4, "expected generic WordValue + bits + sentinel + _enabled: {v_macro:?}");

    // Multi-line dioxus numeric round-trip
    let bad_dioxus = "match v {\n AttributeValue::Float(f) => {\n   let s = f.to_string();\n },\n}";
    let mut v_dx = Vec::new();
    scan_macro_and_converter_str("packages/core/dioxus/src/w.rs", bad_dioxus, &mut v_dx);
    assert_eq!(v_dx.len(), 1, "expected multiline dioxus float round-trip: {v_dx:?}");

    // ABI locks: cross-file asserts MUST pass; missing NveEvent MUST fail even if NveEventBatch + usize=64 exist!
    let abi_a = (
        "abi/a.rs".to_string(),
        "#[repr(C)] pub struct NveEventBatch { pub a: u32 }\n#[derive(NveAbi)] #[nve_abi(size = 64)] #[repr(C)] pub struct NveRenderNode { pub x: u32 }\n#[repr(C)] pub struct NveApi { pub f: usize }\nconst X: usize = 64;".to_string(),
    );
    let abi_b = (
        "abi/layout_asserts.rs".to_string(),
        "const _: () = { assert!(size_of::<NveApi>() == 8); assert!(align_of::<NveApi>() == 8); assert!(offset_of!(NveApi, f) == 0); assert!(size_of::<NveEventBatch>() == 4); assert!(align_of::<NveEventBatch>() == 4); assert!(offset_of!(NveEventBatch, a) == 0); };".to_string(),
    );
    let store_a = (
        "store/row.rs".to_string(),
        "#[repr(C)] pub struct NodeRow { pub x: u32 }\nconst _: () = { assert!(size_of::<NodeRow>() == 32); };".to_string(),
    );
    let v_abi = scan_repr_c_abi_locks(&[abi_a, abi_b], &[store_a]);
    assert_eq!(v_abi.len(), 1, "only missing NveEvent should fail: {v_abi:?}");
    assert!(v_abi[0].reason.contains("`NveEvent`"));

    // Clean negative checks
    let clean = "#[derive(NveAbi)]\n#[repr(C)]\npub struct Clean { pub node_id: NodeId, pub w: f32 }\npub struct Span<T>(T);\n";
    let mut v_clean = Vec::new();
    scan_bare_primitive_domain_fields_in_str("clean.rs", clean, &mut v_clean);
    scan_untyped_span_in_str("clean.rs", clean, &mut v_clean);
    scan_macro_and_converter_str("packages/core/tables/src/clean.rs", clean, &mut v_clean);
    scan_banned_symbols_in_str("clean.rs", clean, clean, &["ComponentKind"], true, &mut v_clean);
    assert!(v_clean.is_empty(), "clean code must have 0 violations: {v_clean:?}");

    let mut v_dc = Vec::new();
    let dc_src = "let x = any.downcast_ref();";
    scan_banned_symbols_in_str("bad.rs", dc_src, dc_src, &[], true, &mut v_dc);
    assert_eq!(v_dc.len(), 1, "must catch `.downcast_ref(` without turbofish");
}

#[test]
fn unit_perf_hash_codegen_purity_scanners_positive_and_negative() {
    assert!(line_has_fnv1a("const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;"));
    assert!(line_has_fnv1a("const OFFSET32: u32 = 0x811c_9dc5;"));
    assert!(line_has_fnv1a("use fnv::FnvHashMap;"));
    assert!(!line_has_fnv1a("pub const fn xxh64(b: &[u8]) -> u64"));

    // Multi-line schema hash const
    let ml_hash = "pub const NVE_FOO_SCHEMA_HASH: u64 =\n    0x1234_5678_9abc_def0;";
    let mut v_hash = Vec::new();
    scan_hardcoded_schema_hashes_rs("h.rs", ml_hash, ml_hash, &mut v_hash);
    assert_eq!(v_hash.len(), 1);
    let clean_hash = "pub const NVE_FOO_SCHEMA_HASH: u64 =\n    xxh64(b\"foo\");";
    let mut v_hash_clean = Vec::new();
    scan_hardcoded_schema_hashes_rs("h.rs", clean_hash, clean_hash, &mut v_hash_clean);
    assert!(v_hash_clean.is_empty());
    assert!(has_16_hex_literal("0x6D45E53E0D9535D1") && !has_16_hex_literal("0x0010"));

    // Hot path positive + negative
    let mut v_hot = Vec::new();
    scan_hot_path_str(
        "packages/core/store/src/str_arena.rs",
        "pub struct NodeRow { pub children: Vec<NodeId> }\nfn get(&self) { let _ = std::str::from_utf8(b); let _ = format!(\"x\"); }",
        true,
        &mut v_hot,
    );
    assert_eq!(v_hot.len(), 3);
    let mut v_hot_clean = Vec::new();
    scan_hot_path_str(
        "packages/core/store/src/str_arena.rs",
        "pub struct NodeRow { pub parent: NodeId }\nfn get(&self, s: &[u8]) -> &str { unsafe { std::str::from_utf8_unchecked(s) } }",
        true,
        &mut v_hot_clean,
    );
    assert!(v_hot_clean.is_empty());

    // Codegen positive (raw string via push_str, Dart static const int, TS 53-bit) + negative
    let mut v_cg = Vec::new();
    scan_codegen_target_str(
        "packages/core/codegen/src/targets/dart/wire.rs",
        "s.push_str(r##\"\n1\n2\n3\n4\n5\n6\n7\n8\"##);\nstatic const int k = 0x6D45E53E0D9535D1;",
        &mut v_cg,
    );
    assert_eq!(v_cg.len(), 2, "expected push_str raw block + dart static const int: {v_cg:?}");
    let mut v_ts = Vec::new();
    scan_codegen_target_str(
        "packages/core/codegen/src/targets/typescript/wire.rs",
        "const id = Number(view.getBigUint64(0, true));\nconst x = hi * 4294967296 + lo;",
        &mut v_ts,
    );
    assert_eq!(v_ts.len(), 2, "expected TS 53-bit violations: {v_ts:?}");
    let mut v_ts_clean = Vec::new();
    scan_codegen_target_str(
        "packages/core/codegen/src/targets/typescript/wire.rs",
        "const id: bigint = view.getBigUint64(0, true);\nconst hi = BigInt(h) << 32n;",
        &mut v_ts_clean,
    );
    assert!(v_ts_clean.is_empty(), "clean BigInt TS must pass: {v_ts_clean:?}");

    // Platform purity (motion filename reported ONCE) + clean negative
    let mut v_pur = Vec::new();
    scan_platform_purity_str(
        "packages/core/tables/src/motion.rs",
        "fn a() {}\nfn b() {}\nfn c() {}",
        "fn a() {}\nfn b() {}\nfn c() {}",
        &mut v_pur,
    );
    assert_eq!(v_pur.len(), 1, "motion file in tables must be reported once, not per line");

    let mut v_acr = Vec::new();
    scan_banned_acronyms_str("clean.rs", "pub struct NveWireFrame;", &mut v_acr);
    assert!(v_acr.is_empty());
    let _: Vec<Violation> = v_acr;
}

#[test]
fn unit_duplicate_enums_converters_and_strict_macro_rules() {
    // 1. Duplicate `pub enum TextAlign` across two core files + internal `impl From` + manual
    //    `decode_text_align` / `map_text_align` MUST fail; external SDK bridge `to_skia_align`
    //    and `to_taffy_display` MUST pass cleanly!
    let core_a = (
        "packages/core/tables/src/visual.rs".to_string(),
        "pub enum TextAlign { Left, Center, Right }\npub enum Display { Flex, None }\n".to_string(),
    );
    let core_b = (
        "packages/core/components/src/style_patch.rs".to_string(),
        "pub enum TextAlign { Start, End }\npub enum PatchAlign { Start, End }\nimpl From<PatchAlign> for TextAlign { fn from(x: PatchAlign) -> Self { Self::Start } }\n".to_string(),
    );
    let backend_skia = (
        "packages/backends/skia/src/text.rs".to_string(),
        r#"
            pub fn decode_text_align(raw: u8) -> TextAlign { TextAlign::Left }
            pub fn map_text_align(a: PatchAlign) -> TextAlign { TextAlign::Left }
            pub fn to_skia_align(a: TextAlign) -> skia_safe::textlayout::TextAlign { todo!() }
            pub fn to_taffy_display(d: Display) -> taffy::Display { todo!() }
        "#
        .to_string(),
    );
    let mut v_enum = Vec::new();
    scan_duplicate_enums_and_converters(&[core_a, core_b], &[backend_skia], &mut v_enum);
    assert_eq!(
        v_enum.len(),
        5,
        "expected 2 duplicate TextAlign + 1 impl From + 2 manual skia converters (and 0 external bridges): {v_enum:?}"
    );
    assert!(!v_enum.iter().any(|v| v.snippet.contains("to_skia_align")));
    assert!(!v_enum.iter().any(|v| v.snippet.contains("to_taffy_display")));

    // 2. Strict `#[bits(N)]`, magic sentinels, `Opt*` wrapper, `decode_bytes`, private `ByteReader`.
    let macro_src = r#"
        #[derive(NveTable)]
        pub struct TableRules {
            #[bits(4)]
            pub valid_narrow_u8: u8,
            #[bits(8)]
            pub redundant_u8: u8,
            #[bits(2)]
            pub bad_enum_bits: MyEnum,
            #[default(u8::MAX)]
            pub bad_sentinel: u8,
        }
        pub struct OptAutofillHint(pub u8);
        #[repr(C)]
        pub struct Pod { pub x: u32 }
        impl Pod { pub fn decode_bytes(_b: &[u8]) -> Self { Self { x: 0 } } }
        struct ByteReader<'a>(&'a [u8]);
        impl<'a> ByteReader<'a> { fn read_u32(&mut self) -> u32 { 0 } }
    "#;
    let mut v_macro = Vec::new();
    scan_macro_and_converter_str("packages/core/tables/src/rules.rs", macro_src, &mut v_macro);
    assert_eq!(
        v_macro.len(),
        6,
        "expected redundant_u8 + bad_enum_bits + bad_sentinel + OptAutofillHint + decode_bytes + ByteReader (and NOT valid_narrow_u8): {v_macro:?}"
    );
    assert!(!v_macro.iter().any(|v| v.reason.contains("valid_narrow_u8")));

    // 3. Anti-stub benchmark scanner (positive + negative) and clean byte-slice wire decoder.
    assert!(check_bench_not_stub("fn main() {}").is_some(), "empty main is a stub");
    assert!(
        check_bench_not_stub("fn b(c: &mut Criterion) { c.bench_function(\"x\", |b| b.iter(|| 1)); }")
            .is_some(),
        "missing black_box must fail"
    );
    assert!(
        check_bench_not_stub(
            "use std::hint::black_box; fn b(c: &mut Criterion) { c.bench_function(\"x\", |b| b.iter(|| black_box(1))); } criterion_main!(benches);"
        )
        .is_none(),
        "real Criterion + black_box bench must pass"
    );

    // Byte-slice wire decoders & multi-arg FFI/struct constructors must NOT false-positive.
    let wire_file = (
        "packages/core/abi/src/events.rs".to_string(),
        r#"
            pub enum DecodeError { Bad }
            pub enum CanvasOp { Rect }
            pub enum RawPropValue { Bytes }
            pub fn decode_events(bytes: &[u8]) -> Result<Vec<Event>, DecodeError> { Ok(vec![]) }
            pub fn decode_raw_prop(prop: &RawPropValue) -> Result<Vec<CanvasOp>, DecodeError> { Ok(vec![]) }
            impl Foo {
                pub unsafe fn from_raw_parts(ptr: *const u8, count: u32) -> Self { Self }
                pub const fn from_args(opcode: u16, args: &[u32]) -> Self { Self }
                pub fn from_captured(error: &DecodeError, reset_count: u32) -> Self { Self }
            }
        "#
        .to_string(),
    );
    let mut v_wire = Vec::new();
    scan_duplicate_enums_and_converters(&[wire_file], &[], &mut v_wire);
    assert!(
        v_wire.is_empty(),
        "wire decoders and multi-arg struct constructors must not false-positive: {v_wire:?}"
    );
}

#[test]
fn unit_content_based_purity_split_booleans_word_enum_and_unset_variants() {
    use crate::content_purity::{
        scan_core_table_string_tables, scan_general_codec_duplication_and_lowercase,
    };
    use crate::rust_lex::strip_rust_comments_and_strings;

    // 1. Split booleans (`_set`, `_present`, `has_<sibling>`), legacy `word_enum!`, and `Unset` variant.
    let interaction_like = r#"
        #[derive(NveTable)]
        pub struct Interaction {
            pub disabled: bool,
            pub has_focus: bool,
            pub tab_index_set: bool,
            pub tab_index: i16,
            pub has_label: bool,
            pub label: Str,
            pub tint_present: bool,
        }
        word_enum! {
            pub enum AriaLive: 2 { Unset = 0, Off = 1, Polite = 2, Assertive = 3 }
        }
        pub enum CleanEnum {
            Alpha = 0,
            Beta = 1,
        }
    "#;
    let mut v_macro = Vec::new();
    scan_macro_and_converter_str(
        "packages/core/tables/src/interaction.rs",
        interaction_like,
        &mut v_macro,
    );
    assert_eq!(
        v_macro.len(),
        5,
        "expected tab_index_set + has_label + tint_present + word_enum! + Unset (and NOT disabled/has_focus): {v_macro:?}"
    );
    assert!(!v_macro.iter().any(|v| v.reason.contains("disabled")));
    assert!(!v_macro.iter().any(|v| v.reason.contains("has_focus")));

    // 2. General mirrored `<->` encoder/decoder detection works ANYWHERE in `packages/` regardless
    //    of function name (enum <-> str, enum <-> int, enum <-> enum) + repo-wide `.to_ascii_lowercase()`.
    let mirrored_any_crate = r#"
        impl MyToken {
            pub const fn forward_fn(self) -> &'static str {
                match self {
                    Self::Alpha => "alpha",
                    Self::Beta => "beta",
                    Self::Gamma => "gamma",
                }
            }
            pub fn reverse_fn(raw: &str) -> Option<Self> {
                match raw.trim().to_ascii_lowercase().as_str() {
                    "alpha" => Some(Self::Alpha),
                    "beta" => Some(Self::Beta),
                    "gamma" => Some(Self::Gamma),
                    _ => None,
                }
            }
            pub fn single_int_decode(code: u8) -> Option<Self> {
                match code {
                    0 => Some(Self::Alpha),
                    1 => Some(Self::Beta),
                    2 => Some(Self::Gamma),
                    _ => None,
                }
            }
            pub fn documented_lowercase(c: char) -> char {
                // allow(to_ascii_lowercase): single char by value, zero heap allocation
                c.to_ascii_lowercase()
            }
        }
        // Clean negative cases: GPU priority scoring on external `vk::` enum & Display impl MUST NOT fire.
        fn pick_gpu(ty: vk::PhysicalDeviceType) -> u32 {
            match ty {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 100,
                vk::PhysicalDeviceType::CPU => 10,
                _ => 0,
            }
        }
        impl core::fmt::Display for MyError {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let s = match self {
                    Self::A => "a",
                    Self::B => "b",
                    Self::C => "c",
                };
                f.write_str(s)
            }
        }
    "#;
    let clean_any = strip_rust_comments_and_strings(mirrored_any_crate);
    let mut v_general = Vec::new();
    scan_general_codec_duplication_and_lowercase(
        "packages/backends/web/src/keywords.rs",
        mirrored_any_crate,
        &clean_any,
        &mut v_general,
    );
    assert_eq!(
        v_general.len(),
        3,
        "expected unexplained .to_ascii_lowercase + mirrored forward/reverse pair + single_int_decode (and NOT documented_lowercase or Display): {v_general:?}"
    );
    assert!(!v_general.iter().any(|v| v.snippet.contains("c.to_ascii_lowercase()")));

    // 3. Inside core tables, even one-way enum <-> str keyword tables are banned (and Error impls exempt).
    let core_one_way = r#"
        impl AriaRole {
            pub const fn totally_innocent_name(self) -> &'static str {
                match self {
                    Self::Button => "button",
                    Self::Link => "link",
                    Self::Dialog => "dialog",
                }
            }
        }
        impl CodecError {
            pub const fn describe(self) -> &'static str {
                match self {
                    Self::Bad => "bad",
                    Self::Short => "short",
                    Self::Trailing => "trailing",
                }
            }
        }
    "#;
    let clean_core = strip_rust_comments_and_strings(core_one_way);
    let mut v_core = Vec::new();
    scan_core_table_string_tables(
        "packages/core/tables/src/interaction.rs",
        core_one_way,
        &clean_core,
        &mut v_core,
    );
    assert_eq!(v_core.len(), 1, "expected only AriaRole enum->str table (not CodecError): {v_core:?}");
}
