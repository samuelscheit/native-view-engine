//! Deep unit, validation, determinism, and real-compiler verification tests for NVE's
//! Nitro-inspired Rust-first 6-target Hybrid FFI code generator.

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{
    generate_hybrid_all, generate_hybrid_target, parse_hybrid_rust_spec, CodegenError,
    HybridDefaultValue, HybridEnumSpec, HybridEnumVariant, HybridEventSpec, HybridFieldSpec,
    HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec, HybridPropSpec,
    HybridStructSpec, HybridTarget, HybridType, HybridViewSpec, NVE_HYBRID_INTERFACE_ID_BASE,
};

/// Realistic multi-item declarative Rust spec exercising every supported feature:
/// - `#[nve_hybrid_enum]` with explicit and auto-incremented discriminants
/// - `#[nve_hybrid_struct]` for both scalar-only POD structs (including nested POD) and owned/non-POD structs
/// - `#[nve_hybrid_module(name = "Crypto")]` covering primitives, strings, byte slices,
///   `Option<T>` (POD and non-POD), `Vec<T>`, `Result<T, String>`, `Result<(), String>`,
///   nested structs, enums, and `NveValue`
/// - `#[nve_hybrid_view(name = "VideoSurface")]` covering typed `#[prop]` with defaults,
///   typed `#[event]`, and imperative `#[method]`
const REALISTIC_HYBRID_SPEC: &str = r#"
use nve_abi::NveValue;

/// Supported cryptographic hash algorithms.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum HashAlgorithm {
    Blake3 = 0,
    Sha256 = 1,
    Sha512 = 2,
}

/// Video playback quality tier.
#[nve_hybrid_enum]
#[repr(u32)]
pub enum VideoQuality {
    Auto = 0,
    Sd480,
    Hd1080 = 10,
    Uhd4k,
}

/// Scalar-only POD point struct (passed directly by value across C ABI).
#[nve_hybrid_struct]
pub struct FramePoint {
    pub x: f32,
    pub y: f32,
}

/// Nested POD struct embedding another POD struct and an enum.
#[nve_hybrid_struct]
pub struct DigestHeader {
    pub algorithm: HashAlgorithm,
    pub byte_len: u32,
    pub verified: bool,
    pub anchor: FramePoint,
}

/// Non-POD owned record containing strings, bytes, options, vectors, and nested structs.
#[nve_hybrid_struct]
pub struct SignedPayload {
    pub header: DigestHeader,
    pub label: String,
    pub digest: Vec<u8>,
    pub tags: Vec<String>,
    pub nonce: Option<u64>,
}

/// High-performance zero-JSON native crypto module.
#[nve_hybrid_module(name = "Crypto")]
pub trait CryptoSpec {
    fn version(&self) -> String;
    fn add(&self, a: i64, b: i64) -> i64;
    fn blake3(&self, data: &[u8]) -> Vec<u8>;
    fn hash_with(&self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader;
    fn verify_payload(&self, payload: SignedPayload) -> Result<bool, String>;
    fn find_nonce(&self, label: &str, hint: Option<u64>) -> Option<u64>;
    fn lookup_payload(&self, id: u32) -> Option<SignedPayload>;
    fn batch_sign(&self, inputs: Vec<String>, algo: HashAlgorithm) -> Result<Vec<SignedPayload>, String>;
    fn inspect_dynamic(&self, meta: NveValue) -> NveValue;
    fn reset_cache(&mut self) -> Result<(), String>;
}

/// Native hardware-accelerated video surface view.
#[nve_hybrid_view(name = "VideoSurface")]
pub trait VideoSurfaceSpec {
    #[prop]
    fn source_url(&self) -> String;

    #[prop(default = false)]
    fn muted(&self) -> bool;

    #[prop(default = 1.0)]
    fn playback_rate(&self) -> f32;

    #[prop(default = VideoQuality::Auto)]
    fn quality(&self) -> VideoQuality;

    #[prop(default = None)]
    fn poster_url(&self) -> Option<String>;

    #[event]
    fn on_progress(&self, position_ms: u64, duration_ms: u64);

    #[event]
    fn on_error(&self, code: i32, message: String);

    #[method]
    fn seek(&mut self, position_ms: u64);

    #[method]
    fn capture_frame(&self, at_point: FramePoint) -> Result<Vec<u8>, String>;
}
"#;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_hybrid_codegen_test_{}_{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn tool_available(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn parses_realistic_multi_item_rust_spec_accurately() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("spec should parse cleanly");

    assert_eq!(spec.enums.len(), 2);
    assert_eq!(spec.enums[0].name, "HashAlgorithm");
    assert_eq!(spec.enums[0].variants.len(), 3);
    assert_eq!(spec.enums[1].name, "VideoQuality");
    assert_eq!(spec.enums[1].variants[1].value, 1);
    assert_eq!(spec.enums[1].variants[2].value, 10);
    assert_eq!(spec.enums[1].variants[3].value, 11);

    assert_eq!(spec.structs.len(), 3);
    assert!(spec.find_struct("FramePoint").unwrap().is_pod(&spec));
    assert!(spec.find_struct("DigestHeader").unwrap().is_pod(&spec));
    assert!(!spec.find_struct("SignedPayload").unwrap().is_pod(&spec));

    assert_eq!(spec.modules.len(), 1);
    let crypto = &spec.modules[0];
    assert_eq!(crypto.name, "Crypto");
    assert_eq!(crypto.trait_name, "CryptoSpec");
    assert_eq!(crypto.methods.len(), 10);
    assert!(crypto.interface_id() >= NVE_HYBRID_INTERFACE_ID_BASE);

    assert_eq!(spec.views.len(), 1);
    let video = &spec.views[0];
    assert_eq!(video.name, "VideoSurface");
    assert_eq!(video.trait_name, "VideoSurfaceSpec");
    assert_eq!(video.props.len(), 5);
    assert_eq!(video.events.len(), 2);
    assert_eq!(video.methods.len(), 2);
    assert!(video.interface_id() >= NVE_HYBRID_INTERFACE_ID_BASE);
    assert_ne!(crypto.interface_id(), video.interface_id());
    assert_ne!(crypto.schema_hash(&spec), video.schema_hash(&spec));
}

#[test]
fn generates_all_6_targets_deterministically_with_zero_json() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse spec");
    let first = generate_hybrid_all(&spec, HybridTarget::ALL).expect("generate all 6 targets");
    let second = generate_hybrid_all(&spec, HybridTarget::ALL).expect("generate second pass");

    assert_eq!(first.len(), 6);
    assert_eq!(first, second, "codegen output must be 100% deterministic");

    for output in &first {
        assert!(!output.files.is_empty());
        for file in &output.files {
            assert!(!file.content.is_empty());
            // Guarantee zero JSON runtime usage across all 6 targets
            assert!(
                !file.content.contains("serde_json"),
                "{} must not use serde_json",
                file.relative_path
            );
            assert!(
                !file.content.contains("org.json"),
                "{} must not use org.json",
                file.relative_path
            );
            assert!(
                !file.content.contains("JSON.parse") && !file.content.contains("JSON.stringify"),
                "{} must not use JSON.parse/stringify",
                file.relative_path
            );
            assert!(
                !file.content.contains("jsonEncode") && !file.content.contains("jsonDecode"),
                "{} must not use Dart JSON codec",
                file.relative_path
            );
            assert!(
                !file.content.contains("@objc") && !file.content.contains("NSObject"),
                "{} must not use Obj-C bridging",
                file.relative_path
            );
        }
    }
}

#[test]
fn schema_hash_changes_on_any_abi_mutation() {
    let base = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let base_crypto_hash = base.modules[0].schema_hash(&base);
    let base_video_hash = base.views[0].schema_hash(&base);

    // Mutate a method param type in Crypto -> Crypto hash changes, VideoSurface hash unchanged
    let mut mutated_param = base.clone();
    mutated_param.modules[0].methods[1].params[0].ty = HybridType::I32;
    assert_ne!(
        mutated_param.modules[0].schema_hash(&mutated_param),
        base_crypto_hash
    );
    assert_eq!(
        mutated_param.views[0].schema_hash(&mutated_param),
        base_video_hash
    );

    // Mutate a transitively referenced POD struct field -> both hashes change because both reference FramePoint
    let mut mutated_struct = base.clone();
    mutated_struct.structs[0].fields[0].ty = HybridType::F64;
    assert_ne!(
        mutated_struct.modules[0].schema_hash(&mutated_struct),
        base_crypto_hash
    );
    assert_ne!(
        mutated_struct.views[0].schema_hash(&mutated_struct),
        base_video_hash
    );
}

#[test]
fn programmatic_rust_dsl_spec_matches_parsed_spec() {
    let dsl = HybridFileSpec {
        namespace: Some("MiniMath".to_string()),
        enums: vec![HybridEnumSpec {
            name: "Rounding".to_string(),
            variants: vec![
                HybridEnumVariant {
                    name: "Floor".to_string(),
                    value: 0,
                    doc: None,
                },
                HybridEnumVariant {
                    name: "Ceil".to_string(),
                    value: 1,
                    doc: None,
                },
            ],
            doc: None,
        }],
        structs: vec![HybridStructSpec {
            name: "Vec2".to_string(),
            fields: vec![
                HybridFieldSpec {
                    name: "x".to_string(),
                    ty: HybridType::F64,
                    doc: None,
                },
                HybridFieldSpec {
                    name: "y".to_string(),
                    ty: HybridType::F64,
                    doc: None,
                },
            ],
            doc: None,
        }],
        modules: vec![HybridModuleSpec {
            name: "Math".to_string(),
            trait_name: "MathSpec".to_string(),
            methods: vec![HybridMethodSpec {
                name: "scale".to_string(),
                params: vec![
                    HybridParamSpec {
                        name: "v".to_string(),
                        ty: HybridType::Struct("Vec2".to_string()),
                    },
                    HybridParamSpec {
                        name: "mode".to_string(),
                        ty: HybridType::Enum("Rounding".to_string()),
                    },
                ],
                return_type: HybridType::Struct("Vec2".to_string()),
                doc: None,
            }],
            doc: None,
        }],
        views: vec![HybridViewSpec {
            name: "PlotView".to_string(),
            trait_name: "PlotViewSpec".to_string(),
            props: vec![HybridPropSpec {
                name: "zoom".to_string(),
                ty: HybridType::F64,
                default_value: Some(HybridDefaultValue::Float(1.0)),
                doc: None,
            }],
            events: vec![HybridEventSpec {
                name: "on_hover".to_string(),
                params: vec![HybridParamSpec {
                    name: "pt".to_string(),
                    ty: HybridType::Struct("Vec2".to_string()),
                }],
                doc: None,
            }],
            methods: vec![],
            doc: None,
        }],
    };

    dsl.validate().expect("programmatic spec should validate");
    let outputs = generate_hybrid_all(&dsl, HybridTarget::ALL).expect("generate all targets");
    assert_eq!(outputs.len(), 6);
    assert_eq!(outputs[0].files[0].relative_path, "mini_math_hybrid.rs");
    assert_eq!(outputs[1].files[0].relative_path, "MiniMathHybridAbi.h");
    assert_eq!(outputs[1].files[1].relative_path, "MiniMathHybridSpec.hpp");
}

#[test]
fn rejects_negative_and_malformed_specs() {
    // 1. Empty spec
    assert!(matches!(
        parse_hybrid_rust_spec("   // nothing here "),
        Err(CodegenError::Validation(_))
    ));

    // 2. Unterminated string literal
    assert!(matches!(
        parse_hybrid_rust_spec(
            r#"
            #[nve_hybrid_module(name = "Broken)]
            pub trait BrokenSpec {}
            "#
        ),
        Err(CodegenError::Parse { .. })
    ));

    // 3. Unterminated block comment
    assert!(matches!(
        parse_hybrid_rust_spec("/* unclosed comment"),
        Err(CodegenError::Parse { .. })
    ));

    // 4. Duplicate struct / enum names
    assert!(matches!(
        parse_hybrid_rust_spec(
            r#"
            #[nve_hybrid_enum]
            pub enum Dup { A = 0 }
            #[nve_hybrid_struct]
            pub struct Dup { pub x: i32 }
            "#
        ),
        Err(CodegenError::Validation(_))
    ));

    // 5. Unknown type reference
    assert!(matches!(
        parse_hybrid_rust_spec(
            r#"
            #[nve_hybrid_module(name = "BadRef")]
            pub trait BadRefSpec {
                fn go(&self, x: DoesNotExist) -> i32;
            }
            "#
        ),
        Err(CodegenError::Validation(_))
    ));

    // 6. Recursive infinite-size value struct (direct cycle A -> B -> A)
    let cyc = parse_hybrid_rust_spec(
        r#"
        #[nve_hybrid_struct]
        pub struct NodeA { pub b: NodeB }
        #[nve_hybrid_struct]
        pub struct NodeB { pub a: NodeA }
        "#,
    );
    assert!(
        matches!(cyc, Err(CodegenError::Validation(ref m)) if m.contains("cycle")),
        "expected cycle validation error, got {cyc:?}"
    );

    // 7. Recursive infinite-size value struct through Option<Self>
    let opt_cyc = parse_hybrid_rust_spec(
        r#"
        #[nve_hybrid_struct]
        pub struct SelfOpt { pub next: Option<SelfOpt> }
        "#,
    );
    assert!(
        matches!(opt_cyc, Err(CodegenError::Validation(ref m)) if m.contains("cycle")),
        "expected Option<Self> cycle validation error, got {opt_cyc:?}"
    );

    // 8. Heap-indirected Vec<Self> breaks value-size cycle and IS allowed
    let tree_ok = parse_hybrid_rust_spec(
        r#"
        #[nve_hybrid_struct]
        pub struct TreeNode {
            pub label: String,
            pub children: Vec<TreeNode>,
        }
        "#,
    );
    assert!(tree_ok.is_ok(), "Vec<Self> should be valid, got {tree_ok:?}");

    // 9. Invalid prop default type mismatch
    assert!(matches!(
        parse_hybrid_rust_spec(
            r#"
            #[nve_hybrid_view(name = "BadView")]
            pub trait BadViewSpec {
                #[prop(default = "wrong")]
                fn count(&self) -> i32;
            }
            "#
        ),
        Err(CodegenError::Validation(_))
    ));

    // 10. Invalid identifier via programmatic IR
    let mut bad_ident = HybridFileSpec::default();
    bad_ident.enums.push(HybridEnumSpec {
        name: "123Bad".to_string(),
        variants: vec![HybridEnumVariant {
            name: "Ok".to_string(),
            value: 0,
            doc: None,
        }],
        doc: None,
    });
    assert!(matches!(
        bad_ident.validate(),
        Err(CodegenError::Validation(_))
    ));
}

#[test]
fn real_compiler_verifies_c11_and_cpp20_headers() {
    assert!(tool_available("clang"), "clang must be installed");
    assert!(tool_available("clang++"), "clang++ must be installed");

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();
    let dir = temp_test_dir("cpp");
    cpp_out.write_to_dir(&dir).unwrap();

    let nve_inc = workspace_root().join("cxx/nve-fabric-bridge/include");
    let abi_h = dir.join("CryptoHybridAbi.h");

    // 1. Verify pure C11 header directly (`-x c-header`) AND included from a C11 translation unit
    let c_driver = dir.join("driver.c");
    std::fs::write(
        &c_driver,
        r#"
#include "CryptoHybridAbi.h"
int _check_c11(const NveHybridCryptoVTable* vt) {
  return vt && vt->abi_version == NVE_HYBRID_ABI_VERSION
      && vt->interface_id == NVE_HYBRID_CRYPTO_INTERFACE_ID
      && vt->schema_hash == NVE_HYBRID_CRYPTO_SCHEMA_HASH;
}
"#,
    )
    .unwrap();

    let c_header_check = Command::new("clang")
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-x",
            "c-header",
            "-fsyntax-only",
        ])
        .arg(format!("-I{}", nve_inc.display()))
        .arg(&abi_h)
        .output()
        .expect("run clang on generated C11 header");
    assert!(
        c_header_check.status.success(),
        "C11 header failed clang -std=c11 -Werror:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&c_header_check.stdout),
        String::from_utf8_lossy(&c_header_check.stderr)
    );

    let c_tu_check = Command::new("clang")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", nve_inc.display()))
        .arg(format!("-I{}", dir.display()))
        .arg(&c_driver)
        .output()
        .expect("run clang on C11 driver");
    assert!(
        c_tu_check.status.success(),
        "C11 driver failed clang -std=c11 -Werror:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&c_tu_check.stdout),
        String::from_utf8_lossy(&c_tu_check.stderr)
    );

    // 2. Verify C++20 spec & client header with `-std=c++20 -Wall -Wextra -Werror -fsyntax-only`
    let cpp_driver = dir.join("driver.cpp");
    std::fs::write(
        &cpp_driver,
        r#"
#include "CryptoHybridSpec.hpp"
void _check_instantiation(const NveHybridCryptoVTable* cvt, const NveHybridVideoSurfaceVTable* vvt) {
  nve::hybrid::CryptoClient crypto(cvt);
  (void)crypto.add(1, 2);
  nve::hybrid::VideoSurfaceClient video(vvt);
  nve::hybrid::VideoSurfaceProps props;
  props.sourceUrl = "https://example.com/video.mp4";
  props.muted = true;
  video.setProps(props);
  video.seek(4200);
}
"#,
    )
    .unwrap();

    let cpp_check = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", nve_inc.display()))
        .arg(format!("-I{}", dir.display()))
        .arg(&cpp_driver)
        .output()
        .expect("run clang++ on generated C++20 header");
    assert!(
        cpp_check.status.success(),
        "C++20 header failed clang++ -std=c++20 -Werror:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cpp_check.stdout),
        String::from_utf8_lossy(&cpp_check.stderr)
    );

    // 3. Also compile-check generated C++20 JNI glue (`CryptoHybridJni.cpp`) against a standard JNI stub header
    let kt_out = generate_hybrid_target(&spec, HybridTarget::Kotlin).unwrap();
    kt_out.write_to_dir(&dir).unwrap();
    std::fs::write(
        dir.join("jni.h"),
        r#"#pragma once
#include <cstdint>
#define JNIEXPORT
#define JNICALL
#define JNI_FALSE 0
#define JNI_TRUE 1
typedef uint8_t jboolean;
typedef int8_t jbyte;
typedef int32_t jint;
typedef int64_t jlong;
typedef float jfloat;
typedef double jdouble;
typedef int32_t jsize;
struct _jobject {};
typedef _jobject* jobject;
struct _jclass : _jobject {};
typedef _jclass* jclass;
struct _jstring : _jobject {};
typedef _jstring* jstring;
struct _jbyteArray : _jobject {};
typedef _jbyteArray* jbyteArray;
struct JNIEnv {
  jclass FindClass(const char*);
  jint ThrowNew(jclass, const char*);
  void* GetDirectBufferAddress(jobject);
  jlong GetDirectBufferCapacity(jobject);
  jbyteArray NewByteArray(jsize);
  void SetByteArrayRegion(jbyteArray, jsize, jsize, const jbyte*);
  const char* GetStringUTFChars(jstring, jboolean*);
  void ReleaseStringUTFChars(jstring, const char*);
  jstring NewStringUTF(const char*);
};
"#,
    )
    .unwrap();

    let jni_check = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", nve_inc.display()))
        .arg(format!("-I{}", dir.display()))
        .arg(dir.join("CryptoHybridJni.cpp"))
        .output()
        .expect("run clang++ on generated JNI C++20 glue");
    assert!(
        jni_check.status.success(),
        "C++20 JNI glue failed clang++ -std=c++20 -Werror:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&jni_check.stdout),
        String::from_utf8_lossy(&jni_check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_compiler_verifies_swift_direct_c_interop() {
    if !tool_available("swiftc") {
        return;
    }

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();
    let swift_out = generate_hybrid_target(&spec, HybridTarget::Swift).unwrap();

    let dir = temp_test_dir("swift");
    cpp_out.write_to_dir(&dir).unwrap();
    swift_out.write_to_dir(&dir).unwrap();

    let nve_inc = workspace_root().join("cxx/nve-fabric-bridge/include");
    let abi_h = dir.join("CryptoHybridAbi.h");
    let swift_file = dir.join("CryptoHybridSpec.swift");

    let swift_check = Command::new("swiftc")
        .arg("-typecheck")
        .arg("-import-objc-header")
        .arg(&abi_h)
        .arg("-I")
        .arg(&nve_inc)
        .arg("-I")
        .arg(&dir)
        .arg(&swift_file)
        .output()
        .expect("run swiftc -typecheck on generated Swift spec");
    assert!(
        swift_check.status.success(),
        "Swift direct C-interop spec failed swiftc -typecheck:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&swift_check.stdout),
        String::from_utf8_lossy(&swift_check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_compiler_verifies_generated_rust_against_nve_abi_with_zero_warnings() {
    // Ensure nve_abi is linked by this test crate so Cargo builds `libnve_abi-*.rlib` in deps.
    let _touch = nve_abi::NveValue::null();

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let rust_out = generate_hybrid_target(&spec, HybridTarget::Rust).unwrap();

    let dir = temp_test_dir("rust");
    rust_out.write_to_dir(&dir).unwrap();
    let rs_file = dir.join("crypto_hybrid.rs");

    // Locate `libnve_abi-*.rlib` in the test executable's `deps/` directory.
    let current_exe = std::env::current_exe().expect("current_exe");
    let deps_dir = current_exe.parent().expect("deps dir");
    let nve_abi_rlib = std::fs::read_dir(deps_dir)
        .expect("read deps dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
        })
        .expect("libnve_abi-*.rlib must exist in target deps dir");

    let out_rlib = dir.join("libcrypto_hybrid_check.rlib");
    let rustc_check = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=lib", "-D", "warnings"])
        .arg("-L")
        .arg(format!("dependency={}", deps_dir.display()))
        .arg("--extern")
        .arg(format!("nve_abi={}", nve_abi_rlib.display()))
        .arg(&rs_file)
        .arg("-o")
        .arg(&out_rlib)
        .output()
        .expect("run rustc on generated Rust module");

    assert!(
        rustc_check.status.success(),
        "Generated Rust module failed rustc --edition=2024 -D warnings:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rustc_check.stdout),
        String::from_utf8_lossy(&rustc_check.stderr)
    );

    // Also compile & RUN an executable test exercising the generated Rust vtables, POD structs,
    // binary NveBinValue round-trips, zero-copy `decode_props_direct`, and `emit_direct` events!
    let main_rs = dir.join("e2e_main.rs");
    std::fs::write(
        &main_rs,
        r#"
use crypto_hybrid::*;
use nve_abi::{BinValueView, NveBufferSlice, NveMapEntry, NveStrSlice, NveValue};

#[derive(Default)]
struct MyCrypto;

impl CryptoHybrid for MyCrypto {
    fn version(&mut self) -> String { "1.2.3".to_string() }
    fn add(&mut self, a: i64, b: i64) -> i64 { a + b }
    fn blake3(&mut self, data: &[u8]) -> Vec<u8> { data.iter().rev().copied().collect() }
    fn hash_with(&mut self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader {
        DigestHeader {
            algorithm: algo,
            byte_len: data.len() as u32,
            verified: true,
            anchor: FramePoint { x: 3.5, y: -2.0 },
        }
    }
    fn verify_payload(&mut self, payload: SignedPayload) -> Result<bool, String> {
        if payload.label.is_empty() {
            return Err("empty label".to_string());
        }
        Ok(payload.header.verified && payload.nonce == Some(77))
    }
    fn find_nonce(&mut self, _label: &str, hint: Option<u64>) -> Option<u64> {
        hint.map(|h| h + 1)
    }
    fn lookup_payload(&mut self, id: u32) -> Option<SignedPayload> {
        if id == 0 { return None; }
        Some(SignedPayload {
            header: DigestHeader {
                algorithm: HashAlgorithm::Sha256,
                byte_len: id,
                verified: true,
                anchor: FramePoint { x: 1.0, y: 2.0 },
            },
            label: format!("item-{id}"),
            digest: vec![0xAA, 0xBB],
            tags: vec!["fast".to_string(), "zero-json".to_string()],
            nonce: Some(99),
        })
    }
    fn batch_sign(&mut self, inputs: Vec<String>, algo: HashAlgorithm) -> Result<Vec<SignedPayload>, String> {
        Ok(inputs.into_iter().map(|s| SignedPayload {
            header: DigestHeader {
                algorithm: algo,
                byte_len: s.len() as u32,
                verified: true,
                anchor: FramePoint { x: 0.0, y: 0.0 },
            },
            label: s,
            digest: vec![1, 2, 3],
            tags: vec![],
            nonce: None,
        }).collect())
    }
    fn inspect_dynamic(&mut self, meta: NveHybridValue) -> NveHybridValue { meta }
    fn reset_cache(&mut self) -> Result<(), String> { Ok(()) }
}

#[derive(Default)]
struct MyVideo {
    props: VideoSurfaceProps,
    last_seek: u64,
}

impl VideoSurfaceHybrid for MyVideo {
    fn apply_props(&mut self, props: VideoSurfaceProps) { self.props = props; }
    fn seek(&mut self, position_ms: u64) { self.last_seek = position_ms; }
    fn capture_frame(&mut self, at_point: FramePoint) -> Result<Vec<u8>, String> {
        if at_point.x < 0.0 { return Err("negative x".to_string()); }
        Ok(vec![0xDE, 0xAD])
    }
}

fn main() {
    let vt = build_crypto_vtable::<MyCrypto>();
    assert_eq!(vt.abi_version, NVE_HYBRID_ABI_VERSION);
    assert_eq!(vt.interface_id, NVE_HYBRID_CRYPTO_INTERFACE_ID);
    assert_eq!(vt.schema_hash, NVE_HYBRID_CRYPTO_SCHEMA_HASH);

    unsafe {
        let h = (vt.create)();
        assert!(!h.is_null());
        assert_eq!((vt.add)(h, 19, 23), 42);

        let ver = (vt.version)(h);
        let ver_str = std::str::from_utf8(std::slice::from_raw_parts(ver.ptr, ver.len)).unwrap();
        assert_eq!(ver_str, "1.2.3");
        (vt.free_bytes)(ver);

        let data = [1u8, 2, 3, 4];
        let hdr = (vt.hash_with)(h, HashAlgorithm::Sha512.to_u32(), NveBufferSlice::from_slice(&data));
        assert_eq!(hdr.algorithm, HashAlgorithm::Sha512);
        assert_eq!(hdr.byte_len, 4);
        assert!(hdr.verified);
        assert_eq!(hdr.anchor.x, 3.5);

        let opt_some = (vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64 { is_some: true, value: 41 });
        assert!(opt_some.is_some && opt_some.value == 42);
        let opt_none = (vt.find_nonce)(h, NveStrSlice::from_str("k"), NveHybridOptU64 { is_some: false, value: 0 });
        assert!(!opt_none.is_some);

        // Non-POD struct binary round-trip + Result Ok/Err
        let mut payload_bin = Vec::new();
        SignedPayload {
            header: hdr,
            label: "hello".to_string(),
            digest: vec![9, 8],
            tags: vec!["a".to_string()],
            nonce: Some(77),
        }.encode_bin(&mut payload_bin);
        let mut ok_out = false;
        let mut err_out = NveHybridOwnedBytes::EMPTY;
        let st = (vt.verify_payload)(h, NveBufferSlice::from_slice(&payload_bin), &mut ok_out, &mut err_out);
        assert_eq!(st, NveHybridStatus::Ok as i32);
        assert!(ok_out);

        let mut bad_bin = Vec::new();
        SignedPayload::default().encode_bin(&mut bad_bin);
        let st_err = (vt.verify_payload)(h, NveBufferSlice::from_slice(&bad_bin), &mut ok_out, &mut err_out);
        assert_eq!(st_err, NveHybridStatus::Err as i32);
        let err_msg = std::str::from_utf8(std::slice::from_raw_parts(err_out.ptr, err_out.len)).unwrap();
        assert_eq!(err_msg, "empty label");
        (vt.free_bytes)(err_out);

        // View props direct zero-copy decoding + event emission
        let vvt = build_video_surface_vtable::<MyVideo>();
        let vh = (vvt.create)();
        let entries = [
            NveMapEntry::new("source_url", NveValue::from_str("https://nve.dev/v.mp4")),
            NveMapEntry::new("muted", NveValue::bool(true)),
            NveMapEntry::new("quality", NveValue::i64(VideoQuality::Uhd4k.to_u32() as i64)),
        ];
        assert_eq!((vvt.set_props_direct)(vh, entries.as_ptr(), entries.len() as u32), NveHybridStatus::Ok as i32);
        let decoded = VideoSurfaceProps::decode_props_direct(&entries).unwrap();
        assert_eq!(decoded.source_url, "https://nve.dev/v.mp4");
        assert!(decoded.muted);
        assert_eq!(decoded.playback_rate, 1.0); // default preserved!
        assert_eq!(decoded.quality, VideoQuality::Uhd4k);
        assert_eq!(decoded.poster_url, None);

        let ev = VideoSurfaceOnProgressEvent { position_ms: 1200, duration_ms: 9000 };
        let ev_bin = ev.encode_bin();
        let ev_view = BinValueView::parse(&ev_bin).unwrap();
        assert_eq!(ev_view.get("position_ms").unwrap().as_i64(), Some(1200));
        assert_eq!(ev_view.get("duration_ms").unwrap().as_i64(), Some(9000));
        let mut saw_direct = false;
        emit_video_surface_on_progress(1, 2, 333, 1200, 9000, |d| {
            saw_direct = true;
            assert_eq!(d.surface_id, 1);
            assert_eq!(d.node_id, 2);
            assert_eq!(d.timestamp_us, 333);
        });
        assert!(saw_direct);

        (vvt.destroy)(vh);
        (vt.destroy)(h);
    }
}
"#,
    )
    .unwrap();

    let e2e_bin = dir.join("e2e_bin");
    let e2e_compile = Command::new("rustc")
        .args(["--edition=2024", "-D", "warnings"])
        .arg("-L")
        .arg(format!("dependency={}", deps_dir.display()))
        .arg("--extern")
        .arg(format!("nve_abi={}", nve_abi_rlib.display()))
        .arg("--extern")
        .arg(format!("crypto_hybrid={}", out_rlib.display()))
        .arg(&main_rs)
        .arg("-o")
        .arg(&e2e_bin)
        .output()
        .expect("compile e2e Rust vtable binary");
    assert!(
        e2e_compile.status.success(),
        "e2e Rust vtable binary failed to compile:\nstderr:\n{}",
        String::from_utf8_lossy(&e2e_compile.stderr)
    );

    let e2e_run = Command::new(&e2e_bin)
        .output()
        .expect("run e2e Rust vtable binary");
    assert!(
        e2e_run.status.success(),
        "e2e Rust vtable binary failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&e2e_run.stdout),
        String::from_utf8_lossy(&e2e_run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_analyzer_verifies_generated_dart_ffi_bindings() {
    if !tool_available("dart") {
        return;
    }

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let dart_out = generate_hybrid_target(&spec, HybridTarget::Dart).unwrap();

    let dir = temp_test_dir("dart");
    let lib_dir = dir.join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    dart_out.write_to_dir(&lib_dir).unwrap();

    // Minimal pubspec so `dart analyze` uses Dart 3.x SDK rules without network access.
    std::fs::write(
        dir.join("pubspec.yaml"),
        "name: nve_hybrid_check\nenvironment:\n  sdk: '>=3.3.0 <4.0.0'\n",
    )
    .unwrap();

    let dart_check = Command::new("dart")
        .args(["analyze", "--fatal-infos"])
        .arg(&dir)
        .output()
        .expect("run dart analyze on generated Dart FFI file");

    assert!(
        dart_check.status.success(),
        "Generated Dart FFI file failed dart analyze:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&dart_check.stdout),
        String::from_utf8_lossy(&dart_check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_cross_language_cpp20_calls_generated_rust_vtable_end_to_end() {
    let _touch = nve_abi::NveValue::null();
    if !tool_available("clang++") {
        return;
    }

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let rust_out = generate_hybrid_target(&spec, HybridTarget::Rust).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();

    let dir = temp_test_dir("cross_lang_e2e");
    rust_out.write_to_dir(&dir).unwrap();
    cpp_out.write_to_dir(&dir).unwrap();

    let current_exe = std::env::current_exe().expect("current_exe");
    let deps_dir = current_exe.parent().expect("deps dir");
    let nve_abi_rlib = std::fs::read_dir(deps_dir)
        .expect("read deps dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
        })
        .expect("libnve_abi-*.rlib must exist in target deps dir");

    // 1. Compile generated `crypto_hybrid.rs` as an rlib
    let hybrid_rlib = dir.join("libcrypto_hybrid.rlib");
    let st = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=lib", "-D", "warnings"])
        .arg("-L")
        .arg(format!("dependency={}", deps_dir.display()))
        .arg("--extern")
        .arg(format!("nve_abi={}", nve_abi_rlib.display()))
        .arg(dir.join("crypto_hybrid.rs"))
        .arg("-o")
        .arg(&hybrid_rlib)
        .output()
        .expect("compile crypto_hybrid rlib");
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));

    // 2. Compile a Rust staticlib exporting `nve_get_interface` backed by `build_crypto_vtable` & `build_video_surface_vtable`
    let provider_rs = dir.join("provider.rs");
    std::fs::write(
        &provider_rs,
        r#"
use std::ffi::c_void;
use crypto_hybrid::*;

#[derive(Default)]
struct LiveCrypto;

impl CryptoHybrid for LiveCrypto {
    fn version(&mut self) -> String { "nve-hybrid-1.0".to_string() }
    fn add(&mut self, a: i64, b: i64) -> i64 { a + b }
    fn blake3(&mut self, data: &[u8]) -> Vec<u8> { data.iter().map(|b| b ^ 0x5A).collect() }
    fn hash_with(&mut self, algo: HashAlgorithm, data: &[u8]) -> DigestHeader {
        DigestHeader {
            algorithm: algo,
            byte_len: data.len() as u32,
            verified: true,
            anchor: FramePoint { x: 12.5, y: -4.25 },
        }
    }
    fn verify_payload(&mut self, payload: SignedPayload) -> Result<bool, String> {
        if payload.label.is_empty() {
            return Err("missing payload label".to_string());
        }
        Ok(payload.header.algorithm == HashAlgorithm::Sha256
            && payload.tags == vec!["alpha".to_string(), "beta".to_string()]
            && payload.nonce == Some(1337))
    }
    fn find_nonce(&mut self, label: &str, hint: Option<u64>) -> Option<u64> {
        hint.map(|h| h + label.len() as u64)
    }
    fn lookup_payload(&mut self, id: u32) -> Option<SignedPayload> {
        if id == 0 { return None; }
        Some(SignedPayload {
            header: DigestHeader {
                algorithm: HashAlgorithm::Blake3,
                byte_len: id * 4,
                verified: true,
                anchor: FramePoint { x: 7.0, y: 8.0 },
            },
            label: format!("rec-{id}"),
            digest: vec![0xDE, 0xAD, 0xBE, 0xEF],
            tags: vec!["fast".to_string(), "binary".to_string()],
            nonce: Some(4242),
        })
    }
    fn batch_sign(&mut self, inputs: Vec<String>, algo: HashAlgorithm) -> Result<Vec<SignedPayload>, String> {
        if inputs.is_empty() { return Err("empty batch".to_string()); }
        Ok(inputs.into_iter().map(|label| SignedPayload {
            header: DigestHeader {
                algorithm: algo,
                byte_len: label.len() as u32,
                verified: true,
                anchor: FramePoint { x: 1.0, y: 2.0 },
            },
            label,
            digest: vec![1, 2],
            tags: vec!["signed".to_string()],
            nonce: Some(1),
        }).collect())
    }
    fn inspect_dynamic(&mut self, meta: NveHybridValue) -> NveHybridValue { meta }
    fn reset_cache(&mut self) -> Result<(), String> { Ok(()) }
}

#[derive(Default)]
struct LiveVideo {
    props: VideoSurfaceProps,
    seek_pos: u64,
}

impl VideoSurfaceHybrid for LiveVideo {
    fn apply_props(&mut self, props: VideoSurfaceProps) { self.props = props; }
    fn seek(&mut self, position_ms: u64) { self.seek_pos = position_ms; }
    fn capture_frame(&mut self, at_point: FramePoint) -> Result<Vec<u8>, String> {
        if !self.props.muted || self.seek_pos != 9876 {
            return Err("unexpected view state".to_string());
        }
        Ok(vec![at_point.x as u8, at_point.y as u8, self.props.source_url.len() as u8])
    }
}

static CRYPTO_VTABLE: NveHybridCryptoVTable = build_crypto_vtable::<LiveCrypto>();
static VIDEO_VTABLE: NveHybridVideoSurfaceVTable = build_video_surface_vtable::<LiveVideo>();

#[unsafe(no_mangle)]
pub extern "C" fn nve_get_interface(interface_id: u32) -> *const c_void {
    match interface_id {
        NVE_HYBRID_CRYPTO_INTERFACE_ID => (&CRYPTO_VTABLE as *const NveHybridCryptoVTable).cast(),
        NVE_HYBRID_VIDEO_SURFACE_INTERFACE_ID => (&VIDEO_VTABLE as *const NveHybridVideoSurfaceVTable).cast(),
        _ => std::ptr::null(),
    }
}
"#,
    )
    .unwrap();

    let staticlib = dir.join("liblive_provider.a");
    let lib_build = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=staticlib", "-D", "warnings"])
        .arg("-L")
        .arg(format!("dependency={}", deps_dir.display()))
        .arg("--extern")
        .arg(format!("nve_abi={}", nve_abi_rlib.display()))
        .arg("--extern")
        .arg(format!("crypto_hybrid={}", hybrid_rlib.display()))
        .arg(&provider_rs)
        .arg("-o")
        .arg(&staticlib)
        .output()
        .expect("build Rust staticlib");
    assert!(
        lib_build.status.success(),
        "Rust staticlib build failed:\n{}",
        String::from_utf8_lossy(&lib_build.stderr)
    );

    // 3. Compile & link a real C++20 executable calling into the Rust staticlib via generated C++20 clients!
    let cpp_main = dir.join("main.cpp");
    std::fs::write(
        &cpp_main,
        r#"
#include <cassert>
#include <stdexcept>
#include "CryptoHybridSpec.hpp"

int main() {
  using namespace nve::hybrid;
  auto crypto = CryptoClient::resolve(&nve_get_interface);
  assert(crypto.version() == "nve-hybrid-1.0");
  assert(crypto.add(19, 23) == 42);

  std::vector<uint8_t> input{0x10, 0x20, 0x30};
  auto hashed = crypto.blake3(input);
  assert(hashed.size() == 3 && hashed[0] == 0x4A && hashed[1] == 0x7A && hashed[2] == 0x6A);

  auto hdr = crypto.hashWith(HashAlgorithm::Sha512, input);
  assert(hdr.algorithm == HashAlgorithm::Sha512);
  assert(hdr.byteLen == 3u);
  assert(hdr.verified == true);
  assert(hdr.anchor.x == 12.5f && hdr.anchor.y == -4.25f);

  auto nonce_some = crypto.findNonce("hello", 100u);
  assert(nonce_some.has_value() && *nonce_some == 105u);
  assert(!crypto.findNonce("hello", std::nullopt).has_value());

  SignedPayload p;
  p.header = DigestHeader{HashAlgorithm::Sha256, 8u, true, FramePoint{1.0f, 2.0f}};
  p.label = "ok";
  p.digest = {1, 2, 3};
  p.tags = {"alpha", "beta"};
  p.nonce = 1337u;
  assert(crypto.verifyPayload(p) == true);

  bool caught = false;
  try {
    SignedPayload bad;
    (void)crypto.verifyPayload(bad);
  } catch (const std::runtime_error& e) {
    caught = std::string(e.what()) == "missing payload label";
  }
  assert(caught);

  assert(!crypto.lookupPayload(0).has_value());
  auto found = crypto.lookupPayload(7);
  assert(found.has_value());
  assert(found->header.algorithm == HashAlgorithm::Blake3);
  assert(found->header.byteLen == 28u);
  assert(found->label == "rec-7");
  assert(found->digest.size() == 4 && found->digest[0] == 0xDE && found->digest[3] == 0xEF);
  assert(found->tags.size() == 2 && found->tags[0] == "fast" && found->tags[1] == "binary");
  assert(found->nonce.has_value() && *found->nonce == 4242u);

  auto batch = crypto.batchSign({"one", "two"}, HashAlgorithm::Sha256);
  assert(batch.size() == 2 && batch[0].label == "one" && batch[1].label == "two");
  crypto.resetCache();

  auto video = VideoSurfaceClient::resolve(&nve_get_interface);
  VideoSurfaceProps props;
  props.sourceUrl = "https://nve.dev/clip.mp4";
  props.muted = true;
  props.quality = VideoQuality::Uhd4k;
  video.setProps(props);
  video.seek(9876u);
  auto frame = video.captureFrame(FramePoint{5.0f, 9.0f});
  assert(frame.size() == 3 && frame[0] == 5 && frame[1] == 9 && frame[2] == 24);
  return 0;
}
"#,
    )
    .unwrap();

    let nve_inc = workspace_root().join("cxx/nve-fabric-bridge/include");
    let cpp_bin = dir.join("cpp_e2e_bin");
    let cpp_link = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror"])
        .arg(format!("-I{}", nve_inc.display()))
        .arg(format!("-I{}", dir.display()))
        .arg(&cpp_main)
        .arg(&staticlib)
        .arg("-o")
        .arg(&cpp_bin)
        .output()
        .expect("link C++20 binary against Rust staticlib");
    assert!(
        cpp_link.status.success(),
        "C++20 <-> Rust link failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cpp_link.stdout),
        String::from_utf8_lossy(&cpp_link.stderr)
    );

    let run = Command::new(&cpp_bin)
        .output()
        .expect("execute C++20 <-> Rust live binary");
    assert!(
        run.status.success(),
        "C++20 <-> Rust live execution failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
