//! Real-`rustc` compile-and-run tests of generated Rust hybrid bindings over the static
//! `nve_abi::hybrid` runtime: vtable trampolines (success, error, invalid-argument, null-handle
//! paths), the `NveBinValue` codec of every type, direct props, and direct events.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");

/// Every codec shape, raw C-ABI carriers, and names that shadow generated bindings (see the
/// fixture's header).
const EDGE_HYBRID_SPEC: &str = include_str!("fixtures/hybrid_rust/edge_hybrid.nve.rs");
const EDGE_MAIN_RS: &str = include_str!("fixtures/hybrid_rust/edge_main.rs");
const REALISTIC_MAIN_RS: &str = include_str!("fixtures/hybrid_rust/realistic_main.rs");

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_hybrid_rust_runtime_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

/// Generates the Rust bindings of `spec_src`, builds them as an rlib, then builds and runs
/// `main_src` against them; returns the program's stdout.
fn compile_and_run(tag: &str, spec_src: &str, crate_name: &str, main_src: &str) -> String {
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let out = generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
    let dir = temp_test_dir(tag);
    out.write_to_dir(&dir).expect("write bindings");
    assert_eq!(out.files.len(), 1, "the Rust target emits one module");

    let rustc = common::rustc::Rustc::locate();
    let generated = dir.join(&out.files[0].relative_path);
    let rlib = rustc.build_lib(&dir, crate_name, &generated);
    let main_rs = dir.join("main.rs");
    std::fs::write(&main_rs, main_src).expect("write main.rs");
    let bin = dir.join(format!("e2e{}", std::env::consts::EXE_SUFFIX));
    rustc.build(&dir, &[], &[(crate_name, &rlib)], &main_rs, &bin);

    let run = Command::new(&bin).output().expect("run e2e binary");
    assert!(
        run.status.success(),
        "e2e binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn generated_rust(spec_src: &str) -> String {
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let out = generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
    out.files.into_iter().map(|f| f.content).collect()
}

#[test]
fn realistic_spec_round_trips_through_rustc_compiled_vtables() {
    let stdout = compile_and_run(
        "realistic",
        REALISTIC_HYBRID_SPEC,
        "crypto_hybrid",
        REALISTIC_MAIN_RS,
    );
    assert!(stdout.contains("realistic hybrid e2e ok"));
}

#[test]
fn edge_spec_round_trips_every_codec_shape_and_error_path() {
    let stdout = compile_and_run("edge", EDGE_HYBRID_SPEC, "edge_hybrid", EDGE_MAIN_RS);
    assert!(stdout.contains("edge hybrid e2e ok"));
}

#[test]
fn generated_rust_reuses_the_static_runtime_instead_of_embedding_it() {
    for spec in [REALISTIC_HYBRID_SPEC, EDGE_HYBRID_SPEC] {
        let src = generated_rust(spec);
        for embedded in [
            "pub mod nve_bin_codec",
            "pub struct NveHybridOwnedBytes",
            "pub enum NveHybridStatus",
            "pub struct NveHybridValue",
            "fn nve_hybrid_alloc_bytes",
            "Vec::from_raw_parts",
            "to_le_bytes",
        ] {
            assert!(
                !src.contains(embedded),
                "generated Rust embeds runtime code `{embedded}`"
            );
        }
        assert!(src.contains("pub use nve_abi::hybrid::{"));
        assert!(
            !src.lines().any(|line| line.starts_with("use ")),
            "generated Rust names `nve_abi` items by path and imports nothing"
        );
        // Prelude types and traits are named by path too, so a spec type `Result` / `String` /
        // `Default` / ... can never shadow them. (Derives live in the macro namespace.)
        let code = src.lines().filter(|l| {
            let l = l.trim_start();
            !l.starts_with("//") && !l.starts_with("#[")
        });
        for line in code {
            for bare in [
                "Result", "Option", "Vec", "String", "Box", "Default", "From", "FnOnce",
            ] {
                for (at, _) in line.match_indices(bare) {
                    let before = line[..at].chars().next_back();
                    let after = line[at + bare.len()..].chars().next();
                    let part_of_ident =
                        |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                    assert!(
                        line[..at].ends_with("::") || part_of_ident(before) || part_of_ident(after),
                        "bare prelude name `{bare}` in generated line: {line}"
                    );
                }
            }
        }
        assert!(src.contains("nve_abi::hybrid_struct_codec!("));
        assert!(src.contains("nve_abi::hybrid_enum_codec!("));
        // `Value` props decode via the codec, which keeps the C tree's structure.
        assert!(
            !src.contains("NveValue::from_bytes(_"),
            "composite event params must not be wrapped in bytes"
        );
        assert!(!src.contains("unwrap()") && !src.contains(".expect(") && !src.contains("panic!"));
    }
}

#[test]
fn rust_target_rejects_members_that_cannot_cross_a_vtable() {
    let spec_src = r#"
#[nve_hybrid_struct]
pub struct Packet {
    pub data: *const u8,
    pub len: usize,
}

#[nve_hybrid_module(name = "Net")]
pub trait NetSpec {
    fn send(&self, packets: Vec<Packet>);
}
"#;
    let spec = parse_hybrid_rust_spec(spec_src).expect("spec parses");
    let err =
        generate_hybrid_target(&spec, HybridTarget::Rust).expect_err("pointer payloads are C-only");
    let message = err.to_string();
    assert!(
        message.contains("module 'Net' method 'send' param 'packets'"),
        "{message}"
    );
    assert!(message.contains("raw pointer"), "{message}");
}

/// Small specs that exercise few `nve_abi` items, plus forms that once broke `-D warnings`: a
/// string default naming runtime symbols, C-only records (union, raw pointer, handle alias), and
/// a method named `default`. Each must compile cleanly.
#[test]
fn minimal_and_unusual_specs_compile_cleanly() {
    let specs = [
        (
            "enum_only",
            "#[nve_hybrid_enum]\n#[repr(u32)]\npub enum Mode { A = 0, B = 1 }\n",
        ),
        (
            "pod_struct_only",
            "#[nve_hybrid_struct]\npub struct P { pub x: f32 }\n",
        ),
        (
            "scalar_module",
            "#[nve_hybrid_module(name = \"Calc\")]\npub trait CalcSpec { fn add(&self, a: i64, b: i64) -> i64; fn ping(&self); }\n",
        ),
        (
            "string_module",
            "#[nve_hybrid_module(name = \"Echo\")]\npub trait EchoSpec { fn echo(&self, s: &str) -> Result<String, String>; }\n",
        ),
        (
            "eventless_view",
            "#[nve_hybrid_view(name = \"Label\")]\npub trait LabelSpec {\n    #[prop]\n    fn text(&self) -> String;\n    #[prop(default = 1.0)]\n    fn scale(&self) -> f32;\n}\n",
        ),
        (
            "empty_view",
            "#[nve_hybrid_view(name = \"Spacer\")]\npub trait SpacerSpec {\n    #[method]\n    fn reset(&mut self);\n}\n",
        ),
        (
            "runtime_names_in_strings",
            "#[nve_hybrid_view(name = \"Note\")]\npub trait NoteSpec {\n    #[prop(default = \"mentions NveValue, DecodeError and NveMapRef\")]\n    fn text(&self) -> String;\n}\n",
        ),
        (
            "c_only_records",
            "#[nve_hybrid_alias]\npub type Cam = u64;\n#[nve_hybrid_union]\n#[repr(C)]\npub union Num { pub i: i64, pub f: f64 }\n#[nve_hybrid_struct]\npub struct Packet { pub data: *const u8, pub len: usize }\n#[nve_hybrid_module(name = \"Tick\")]\npub trait TickSpec { fn now(&self) -> u64; }\n",
        ),
        (
            "default_method_and_wide_arrays",
            "#[nve_hybrid_struct]\npub struct Big { pub on: [bool; 33], pub names: [String; 2] }\n#[nve_hybrid_module(name = \"Wide\")]\npub trait WideSpec { fn default(&self) -> [u8; 64]; fn take(&self, big: Big) -> Big; }\n",
        ),
    ];
    let rustc = common::rustc::Rustc::locate();
    for (tag, spec_src) in specs {
        let spec = parse_hybrid_rust_spec(spec_src).unwrap_or_else(|e| panic!("{tag}: {e}"));
        let out =
            generate_hybrid_target(&spec, HybridTarget::Rust).expect("Rust bindings generate");
        let dir = temp_test_dir(tag);
        out.write_to_dir(&dir).expect("write bindings");
        rustc.build_lib(&dir, tag, &dir.join(&out.files[0].relative_path));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn shared_rustc_helper_finds_nve_abi() {
    let rustc = common::rustc::Rustc::locate();
    let dir = temp_test_dir("probe");
    let src = dir.join("probe.rs");
    std::fs::write(
        &src,
        "pub fn probe() -> u32 { nve_abi::hybrid::NVE_HYBRID_ABI_VERSION }\n",
    )
    .expect("write probe");
    let rlib = rustc.build_lib(&dir, "probe", &src);
    assert!(Path::new(&rlib).exists());
    let _ = std::fs::remove_dir_all(&dir);
}
