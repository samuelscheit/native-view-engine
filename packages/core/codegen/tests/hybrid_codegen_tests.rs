//! Deep unit, validation, determinism, and real-compiler verification tests for NVE's
//! Nitro-inspired Rust-first 6-target Hybrid FFI code generator.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{
    CodegenError, HybridDefaultValue, HybridEnumSpec, HybridEnumVariant, HybridEventSpec,
    HybridFieldSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec,
    HybridPropSpec, HybridStructSpec, HybridTarget, HybridType, HybridViewSpec,
    NVE_HYBRID_INTERFACE_ID_BASE, generate_hybrid_all, generate_hybrid_target,
    parse_hybrid_rust_spec,
};

/// Realistic multi-item declarative Rust spec exercising every supported feature:
/// - `#[nve_hybrid_enum]` with explicit and auto-incremented discriminants
/// - `#[nve_hybrid_struct]` for both scalar-only POD structs (including nested POD) and owned/non-POD structs
/// - `#[nve_hybrid_module(name = "Crypto")]` covering primitives, strings, byte slices,
///   `Option<T>` (POD and non-POD), `Vec<T>`, `Result<T, String>`, `Result<(), String>`,
///   nested structs, enums, and `NveValue`
/// - `#[nve_hybrid_view(name = "VideoSurface")]` covering typed `#[prop]` with defaults,
///   typed `#[event]`, and imperative `#[method]`
const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");
/// A functional fake `<jni.h>` (`JNIEnv` records thrown exceptions), so JNI glue compiles and runs.
const FAKE_JNI_H: &str = include_str!("fixtures/fake_jni.h");

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("workspace root 3 levels above CARGO_MANIFEST_DIR")
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

    assert_eq!(spec.structs.len(), 4);
    assert!(spec.find_struct("FramePoint").unwrap().is_pod(&spec));
    assert!(spec.find_struct("DigestHeader").unwrap().is_pod(&spec));
    assert!(!spec.find_struct("SignedPayload").unwrap().is_pod(&spec));
    assert!(!spec.find_struct("Cursor").unwrap().is_pod(&spec));

    assert_eq!(spec.modules.len(), 1);
    let crypto = &spec.modules[0];
    assert_eq!(crypto.name, "Crypto");
    assert_eq!(crypto.trait_name, "CryptoSpec");
    assert_eq!(crypto.methods.len(), 11);
    assert!(crypto.interface_id() >= NVE_HYBRID_INTERFACE_ID_BASE);

    assert_eq!(spec.views.len(), 1);
    let video = &spec.views[0];
    assert_eq!(video.name, "VideoSurface");
    assert_eq!(video.trait_name, "VideoSurfaceSpec");
    assert_eq!(video.props.len(), 9);
    assert_eq!(video.events.len(), 2);
    assert_eq!(video.methods.len(), 3);
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
            let fname = Path::new(&file.relative_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap();
            assert!(
                fname.contains(".generated."),
                "{} must obey the `<name>.generated.<ext>` naming convention",
                file.relative_path
            );
            // Guarantee no runtime JSON serialization across all 6 targets
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

/// `#[prop] fn metadata(&self) -> NveValue` (`HybridType::Value`) keeps its true `NveValue` tags on
/// every target's `setProps` path: never encoded through a scratch `BinWriter` into a `Bytes` value.
#[test]
fn value_direct_props_keep_true_tags_on_all_6_targets() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse spec");
    let outputs = generate_hybrid_all(&spec, HybridTarget::ALL).expect("generate all 6 targets");
    assert_eq!(outputs.len(), 6);
    // The setProps line(s) that carry `metadata` as a value tree / inline `NveBinValue`.
    let direct: &[(HybridTarget, &[&str])] = &[
        (
            HybridTarget::Rust,
            &[r#""metadata" => props.metadata = nve_abi::hybrid::HybridCodec::decode_value_ref(value)?,"#],
        ),
        (HybridTarget::Cpp, &["const ::nve::host::NveValueTree _prop_tree_8(props.metadata);"]),
        (HybridTarget::Swift, &["let _nveProp8 = try _nveTree.value(props.metadata)"]),
        (HybridTarget::Kotlin, &["_nveW.writeMapKey(\"metadata\")", "props.metadata.writeTo(_nveW)"]),
        (HybridTarget::Dart, &["_nveB.writeDynamicValue('metadata', metadata);"]),
        (HybridTarget::TypeScript, &["_nve_w.writeMapKey('metadata');", "props.metadata.writeTo(_nve_w);"]),
    ];
    for output in &outputs {
        let (_, expected) = direct
            .iter()
            .find(|(target, _)| *target == output.target)
            .expect("every target has an expected direct-prop form");
        let src: String = output.files.iter().map(|f| f.content.as_str()).collect();
        let lines: Vec<&str> = src.lines().map(str::trim).collect();
        // Consecutive lines: the map key, then the value written in place.
        assert!(
            lines.windows(expected.len()).any(|w| w == *expected),
            "{:?}: `metadata` is not carried directly; expected {expected:?}",
            output.target
        );
        for line in lines.iter().filter(|l| l.contains("metadata") && !l.contains("urrent")) {
            for banned in ["Bytes", "bytes(", "encoded", "_prop_bin_", "_nveBw", "BinWriter"] {
                assert!(
                    !line.contains(banned),
                    "{:?}: `metadata` (HybridType::Value) routed through `{banned}`: {line}",
                    output.target
                );
            }
        }
        if output.target == HybridTarget::Cpp {
            let key = lines
                .iter()
                .position(|l| l.contains("reinterpret_cast<const uint8_t*>(\"metadata\")"))
                .expect("C++ setProps has a `metadata` entry");
            assert_eq!(lines[key + 1], "entry.value = _prop_tree_8.root();");
            assert!(!src.contains("_prop_bin_8"), "no scratch BinWriter for `metadata`");
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
            ..Default::default()
        }],
        structs: vec![HybridStructSpec {
            name: "Vec2".to_string(),
            fields: vec![
                HybridFieldSpec {
                    name: "x".to_string(),
                    ty: HybridType::F64,
                    doc: None,
                    offset: None,
                    offset_64: None,
                },
                HybridFieldSpec {
                    name: "y".to_string(),
                    ty: HybridType::F64,
                    doc: None,
                    offset: None,
                    offset_64: None,
                },
            ],
            doc: None,
            ..Default::default()
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
                has_self: true,
                optional: false,
            }],
            doc: None,
            ..Default::default()
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
            ..Default::default()
        }],
        ..Default::default()
    };

    dsl.validate().expect("programmatic spec should validate");
    let outputs = generate_hybrid_all(&dsl, HybridTarget::ALL).expect("generate all targets");
    assert_eq!(outputs.len(), 6);
    assert_eq!(
        outputs[0].files[0].relative_path,
        "mini_math_hybrid.generated.rs"
    );
    assert_eq!(
        outputs[1].files[0].relative_path,
        "MiniMathHybridAbi.generated.h"
    );
    assert_eq!(
        outputs[1].files[1].relative_path,
        "MiniMathHybridSpec.generated.hpp"
    );
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
    assert!(
        tree_ok.is_ok(),
        "Vec<Self> should be valid, got {tree_ok:?}"
    );

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
        ..Default::default()
    });
    assert!(matches!(
        bad_ident.validate(),
        Err(CodegenError::Validation(_))
    ));

    // 11. Reserved C++ hybrid runtime symbols must be rejected at top level
    for reserved in [
        "BinReader",
        "BinWriter",
        "DynamicValue",
        "NveValueTree",
        "OwnedBytes",
    ] {
        let src = format!("#[nve_hybrid_struct]\npub struct {reserved} {{ pub x: u32 }}");
        assert!(
            matches!(parse_hybrid_rust_spec(&src), Err(CodegenError::Validation(ref m)) if m.contains("reserved")),
            "expected reserved error for struct {reserved}"
        );
    }
    for reserved in ["checked_bytes", "to_slice", "require_done"] {
        let src = format!("#[nve_hybrid_c_fn]\npub extern \"C\" fn {reserved}() -> u32;");
        assert!(
            matches!(parse_hybrid_rust_spec(&src), Err(CodegenError::Validation(ref m)) if m.contains("reserved")),
            "expected reserved error for c_fn {reserved}"
        );
    }
}

#[test]
fn real_compiler_verifies_c11_and_cpp20_headers() {
    if !common::require_tool("clang") || !common::require_tool("clang++") {
        return;
    }

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();
    let dir = temp_test_dir("cpp");
    common::write_generated_c_abi_headers(&dir);
    cpp_out.write_to_dir(&dir).unwrap();

    let abi_h = dir.join("CryptoHybridAbi.generated.h");

    // 1. Verify pure C11 header directly (`-x c-header`) AND included from a C11 translation unit
    let c_driver = dir.join("driver.c");
    std::fs::write(
        &c_driver,
        r#"
#include "CryptoHybridAbi.generated.h"
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
        .arg(format!("-I{}", dir.display()))
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
#include "CryptoHybridSpec.generated.hpp"
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
    std::fs::write(dir.join("jni.h"), FAKE_JNI_H).unwrap();

    let jni_check = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", dir.display()))
        .arg(dir.join("CryptoHybridJni.generated.cpp"))
        .output()
        .expect("run clang++ on generated JNI C++20 glue");
    assert!(
        jni_check.status.success(),
        "C++20 JNI glue failed clang++ -std=c++20 -Werror:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&jni_check.stdout),
        String::from_utf8_lossy(&jni_check.stderr)
    );

    // 4. Also compile-check shadowing names (`raw` in POD `to_abi()`, and client member names
    //    `vtable_`/`handle_`/`own`/`take_string`/`take_bytes` as method parameters).
    let shadow_spec = parse_hybrid_rust_spec(
        r#"
#[nve_hybrid_struct]
pub struct RawPod {
    pub raw: u32,
}

#[nve_hybrid_module(name = "Shadow")]
pub trait ShadowSpec {
    fn collide(
        &self,
        vtable: RawPod,
        handle: String,
        own: Vec<u8>,
        take_string: Vec<i32>,
        take_bytes: NveValue,
    ) -> Result<String, String>;
    fn echo_val(&self, own: Vec<u8>) -> NveValue;
}
"#,
    )
    .unwrap();
    generate_hybrid_target(&shadow_spec, HybridTarget::Cpp)
        .unwrap()
        .write_to_dir(&dir)
        .unwrap();
    let shadow_driver = dir.join("shadow.cpp");
    std::fs::write(
        &shadow_driver,
        r#"
#include "ShadowHybridSpec.generated.hpp"
std::string _check_shadow(const NveHybridShadowVTable* vt) {
  nve::hybrid::ShadowClient c(vt);
  (void)c.echoVal({});
  return c.collide(nve::hybrid::RawPod{1u}, "a", {}, {1}, nve::hybrid::DynamicValue{});
}
"#,
    )
    .unwrap();
    let shadow_check = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", dir.display()))
        .arg(&shadow_driver)
        .output()
        .expect("run clang++ on shadowing C++20 header");
    assert!(
        shadow_check.status.success(),
        "Shadowing C++20 header failed clang++:\n{}",
        String::from_utf8_lossy(&shadow_check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn write_offline_dart_package_config(dir: &Path, pkg_name: &str) {
    let flutter_plugin_root = common::prepare_offline_nve_flutter_pkg(dir, &workspace_root());
    let canonical_plugin =
        std::fs::canonicalize(&flutter_plugin_root).unwrap_or(flutter_plugin_root);
    let host_root = common::prepare_offline_nve_host_pkg(dir);
    let canonical_host = std::fs::canonicalize(&host_root).unwrap_or(host_root);
    let canonical_dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let to_file_uri = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        if s.starts_with('/') {
            format!("file://{s}")
        } else {
            format!("file:///{s}")
        }
    };
    let dart_tool = dir.join(".dart_tool");
    std::fs::create_dir_all(&dart_tool).expect("create temp .dart_tool dir");
    let config = format!(
        r#"{{
  "configVersion": 2,
  "packages": [
    {{
      "name": "{pkg_name}",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }},
    {{
      "name": "nve_flutter",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }},
    {{
      "name": "nve_host",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }}
  ]
}}
"#,
        to_file_uri(&canonical_dir),
        to_file_uri(&canonical_plugin),
        to_file_uri(&canonical_host),
    );
    std::fs::write(dart_tool.join("package_config.json"), config)
        .expect("write temp package_config.json");
}

#[test]
fn real_analyzer_verifies_generated_dart_ffi_bindings() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let dart_out = generate_hybrid_target(&spec, HybridTarget::Dart).unwrap();
    let dart_src = &dart_out.files[0].content;

    // Verify generated Dart reuses the static `package:nve_host` runtime instead of duplicating codecs/structs.
    assert!(dart_src.contains("import 'package:nve_host/nve_host.dart';"));
    assert!(dart_src.contains("import 'package:nve_host/nve_host_ffi.dart';"));
    assert!(
        !dart_src.contains("package:nve_flutter"),
        "generated Dart hybrid client must depend on package:nve_host, not package:nve_flutter"
    );
    assert!(dart_src.contains("implements HybridPropsEncodable"));
    assert!(dart_src.contains("HybridPropsWriter.withEncodedProps"));
    assert!(dart_src.contains("NveValueTree.fromDynamic"));
    assert!(dart_src.contains("writeDynamicValue('metadata', metadata)"));
    assert!(
        !dart_src.contains("class NveBinWriter"),
        "generated Dart must not duplicate NveBinWriter"
    );
    assert!(
        !dart_src.contains("class NveBinReader"),
        "generated Dart must not duplicate NveBinReader"
    );
    assert!(
        !dart_src.contains("final class NveMapEntry"),
        "generated Dart must not duplicate core ABI FFI structs"
    );
    assert!(
        !dart_src.contains("_NveGetInterfaceNative"),
        "generated Dart must reuse static FFI typedefs from package:nve_host"
    );

    if !common::require_tool("dart") {
        return;
    }

    let dir = temp_test_dir("dart");
    let lib_dir = dir.join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    dart_out.write_to_dir(&lib_dir).unwrap();

    // Minimal pubspec + offline package_config.json resolving `package:nve_host/` to disk.
    std::fs::write(
        dir.join("pubspec.yaml"),
        "name: nve_hybrid_check\nenvironment:\n  sdk: '>=3.4.0 <4.0.0'\n",
    )
    .unwrap();
    write_offline_dart_package_config(&dir, "nve_hybrid_check");

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
fn rust_offset_of_matches_c_offsetof_in_nve_abi_header() {
    use core::mem::{align_of, offset_of, size_of};
    use nve_abi::{
        NveBufferSlice, NveDirectEvent, NveEventBatch, NveMapEntry, NveStrSlice, NveValue,
        NveValueArray, NveValueData, NveValueMap,
    };

    if !common::require_tool("clang") {
        return;
    }

    let mut c_src = String::from(
        "#include <stddef.h>\n#include <stdalign.h>\n#include <nve/generated/nve_abi.generated.h>\n\n",
    );

    let mut check_layout = |c_ty: &str, sz: usize, al: usize, fields: &[(&str, usize)]| {
        c_src.push_str(&format!(
            "_Static_assert(sizeof({c_ty}) == {sz}u, \"sizeof({c_ty}) mismatch vs Rust\");\n"
        ));
        c_src.push_str(&format!(
            "_Static_assert(alignof({c_ty}) == {al}u, \"alignof({c_ty}) mismatch vs Rust\");\n"
        ));
        for &(field, off) in fields {
            c_src.push_str(&format!(
                "_Static_assert(offsetof({c_ty}, {field}) == {off}u, \"offsetof({c_ty}, {field}) mismatch vs Rust\");\n"
            ));
        }
    };

    check_layout(
        "NveBufferSlice",
        size_of::<NveBufferSlice>(),
        align_of::<NveBufferSlice>(),
        &[
            ("ptr", offset_of!(NveBufferSlice, ptr)),
            ("len", offset_of!(NveBufferSlice, len)),
        ],
    );
    check_layout(
        "NveStrSlice",
        size_of::<NveStrSlice>(),
        align_of::<NveStrSlice>(),
        &[
            ("ptr", offset_of!(NveStrSlice, ptr)),
            ("len", offset_of!(NveStrSlice, len)),
        ],
    );
    check_layout(
        "NveValueArray",
        size_of::<NveValueArray>(),
        align_of::<NveValueArray>(),
        &[
            ("ptr", offset_of!(NveValueArray, ptr)),
            ("len", offset_of!(NveValueArray, len)),
        ],
    );
    check_layout(
        "NveValueMap",
        size_of::<NveValueMap>(),
        align_of::<NveValueMap>(),
        &[
            ("ptr", offset_of!(NveValueMap, ptr)),
            ("len", offset_of!(NveValueMap, len)),
        ],
    );
    check_layout(
        "NveValueData",
        size_of::<NveValueData>(),
        align_of::<NveValueData>(),
        &[
            ("bool_val", offset_of!(NveValueData, bool_val)),
            ("i64_val", offset_of!(NveValueData, i64_val)),
            ("f64_val", offset_of!(NveValueData, f64_val)),
            ("str_val", offset_of!(NveValueData, str_val)),
            ("bytes_val", offset_of!(NveValueData, bytes_val)),
            ("array_val", offset_of!(NveValueData, array_val)),
            ("map_val", offset_of!(NveValueData, map_val)),
        ],
    );
    check_layout(
        "NveValue",
        size_of::<NveValue>(),
        align_of::<NveValue>(),
        &[
            ("tag", offset_of!(NveValue, tag)),
            ("data", offset_of!(NveValue, data)),
        ],
    );
    check_layout(
        "NveMapEntry",
        size_of::<NveMapEntry>(),
        align_of::<NveMapEntry>(),
        &[
            ("key", offset_of!(NveMapEntry, key)),
            ("value", offset_of!(NveMapEntry, value)),
        ],
    );
    check_layout(
        "NveDirectEvent",
        size_of::<NveDirectEvent>(),
        align_of::<NveDirectEvent>(),
        &[
            ("surface_id", offset_of!(NveDirectEvent, surface_id)),
            ("node_id", offset_of!(NveDirectEvent, node_id)),
            ("timestamp_us", offset_of!(NveDirectEvent, timestamp_us)),
            ("kind", offset_of!(NveDirectEvent, kind)),
            ("payload_tag", offset_of!(NveDirectEvent, payload_tag)),
            ("name", offset_of!(NveDirectEvent, name)),
            ("x", offset_of!(NveDirectEvent, x)),
            ("y", offset_of!(NveDirectEvent, y)),
            ("width", offset_of!(NveDirectEvent, width)),
            ("height", offset_of!(NveDirectEvent, height)),
            ("text", offset_of!(NveDirectEvent, text)),
            ("value", offset_of!(NveDirectEvent, value)),
        ],
    );
    check_layout(
        "NveEventBatch",
        size_of::<NveEventBatch>(),
        align_of::<NveEventBatch>(),
        &[
            ("ptr", offset_of!(NveEventBatch, ptr)),
            ("len", offset_of!(NveEventBatch, len)),
        ],
    );

    let dir = temp_test_dir("abi_offsetof_parity");
    common::write_generated_c_abi_headers(&dir);
    let c_file = dir.join("check_offsets.c");
    std::fs::write(&c_file, &c_src).unwrap();

    let check = Command::new("clang")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(format!("-I{}", dir.display()))
        .arg(&c_file)
        .output()
        .expect("run clang to verify Rust offset_of! matches C offsetof in nve_abi.generated.h");
    assert!(
        check.status.success(),
        "Rust offset_of! vs C nve_abi.generated.h offsetof mismatch:\nstdout:\n{}\nstderr:\n{}\nC source:\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
        c_src
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_cross_language_cpp20_calls_generated_rust_vtable_end_to_end() {
    if !common::require_tool("clang++") {
        return;
    }

    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let rust_out = generate_hybrid_target(&spec, HybridTarget::Rust).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();

    let dir = temp_test_dir("cross_lang_e2e");
    rust_out.write_to_dir(&dir).unwrap();
    cpp_out.write_to_dir(&dir).unwrap();

    let rustc = common::rustc::Rustc::locate();

    // 1. Compile generated `crypto_hybrid.generated.rs` as an rlib
    let hybrid_rlib = rustc.build_lib(&dir, "crypto_hybrid", &dir.join("crypto_hybrid.generated.rs"));

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
    fn transpose(&mut self, cursor: Cursor) -> Cursor {
        let cols = cursor.grid.first().map_or(0, Vec::len);
        let grid = (0..cols).map(|c| cursor.grid.iter().map(|row| row[c]).collect()).collect();
        Cursor {
            mark: cursor.mark + 1,
            w: cursor.w.iter().rev().copied().collect(),
            r: cursor.r.map(|s| s.to_uppercase()),
            count: cursor.count * 2,
            tag: cursor.tag.map(|v| -v),
            grid,
        }
    }
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
        if at_point.x < 0.0 {
            return Err("bad \u{1F600}".to_string());
        }
        if !self.props.muted || self.seek_pos != 9876 {
            return Err("unexpected view state".to_string());
        }
        let mut frame = vec![at_point.x as u8, at_point.y as u8, self.props.source_url.len() as u8];
        if let Some(focus) = self.props.focus_point {
            frame.push(focus.x as u8);
        }
        if let Some(poster) = &self.props.poster_url {
            frame.push(poster.len() as u8);
        }
        frame.extend_from_slice(&self.props.drm_key);
        for &ch in &self.props.chapters {
            frame.push(ch as u8);
        }
        Ok(frame)
    }
    fn current_metadata(&mut self) -> NveHybridValue { self.props.metadata.clone() }
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
    rustc.build(
        &dir,
        &["--crate-type=staticlib"],
        &[("crypto_hybrid", &hybrid_rlib)],
        &provider_rs,
        &staticlib,
    );

    // 3. Compile & link a real C++20 executable calling into the Rust staticlib via generated C++20 clients!
    let cpp_main = dir.join("main.cpp");
    std::fs::write(
        &cpp_main,
        r#"
#include <cassert>
#include <stdexcept>
#include "CryptoHybridSpec.generated.hpp"

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
  // A dynamic value crosses as an `NveValue` tree and comes back byte-identical.
  BinWriter dw;
  const auto dmap = dw.begin_map(2);
  dw.write_map_key("k");
  const auto darr = dw.begin_array(3);
  dw.write_i64(1);
  dw.write_str("x");
  dw.write_null();
  dw.end(darr);
  dw.write_map_key("f");
  dw.write_f64(2.5);
  dw.end(dmap);
  const auto meta = DynamicValue::from_bytes(dw.bytes());
  assert(meta.has_value());
  assert(crypto.inspectDynamic(*meta) == *meta);

  // Absent non-direct options cross as `Null`; present ones, `Bytes`, `Vec`, and `NveValue`
  // props round-trip losslessly (nested `NveValue` is never double-wrapped as `Bytes`).
  props.posterUrl = "p.png";
  props.focusPoint = FramePoint{7.0f, 0.0f};
  props.drmKey = {0xAA, 0xBB};
  props.chapters = {10, 20, 30};
  props.metadata = *meta;
  video.setProps(props);
  frame = video.captureFrame(FramePoint{5.0f, 9.0f});
  assert(frame.size() == 10 && frame[3] == 7 && frame[4] == 5);
  assert(frame[5] == 0xAA && frame[6] == 0xBB && frame[7] == 10 && frame[8] == 20 && frame[9] == 30);
  assert(video.currentMetadata() == *meta);
  props.posterUrl = std::nullopt;
  props.focusPoint = std::nullopt;
  props.drmKey.clear();
  props.chapters.clear();
  props.metadata = DynamicValue{};
  video.setProps(props);
  assert(video.captureFrame(FramePoint{5.0f, 9.0f}).size() == 3);
  assert(video.currentMetadata().is_null());

  // Field names shadowing codec names (`w`, `mark`, `r`, `count`), two `Option`s, and a nested sequence.
  Cursor cursor;
  cursor.mark = 1u;
  cursor.w = {1, 2, 3};
  cursor.r = "ab";
  cursor.count = 4u;
  cursor.tag = 42;
  cursor.grid = {{1, 2, 3}, {4, 5, 6}};
  const Cursor t = crypto.transpose(cursor);
  assert(t.mark == 2u && t.count == 8u && t.r == std::optional<std::string>("AB"));
  assert(t.tag == std::optional<int32_t>(-42));
  assert((t.w == std::vector<uint8_t>{3, 2, 1}));
  assert((t.grid == std::vector<std::vector<int32_t>>{{1, 4}, {2, 5}, {3, 6}}));

  // Invalid UTF-8 never crosses the ABI: the encoding fails before the call.
  bool utf8_caught = false;
  try {
    (void)crypto.batchSign({"\xff"}, HashAlgorithm::Sha256);
  } catch (const std::runtime_error& e) {
    utf8_caught = std::string(e.what()).find("InvalidUtf8") != std::string::npos;
  }
  assert(utf8_caught);
  return 0;
}
"#,
    )
    .unwrap();

    common::write_generated_c_abi_headers(&dir);
    let cpp_bin = dir.join("cpp_e2e_bin");
    let cpp_link = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror"])
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

    // 4. The Android JNI glue, linked against the same Rust staticlib and *run* on a fake `JNIEnv`
    //    that records what Java would see (pending exceptions, arrays, strings).
    generate_hybrid_target(&spec, HybridTarget::Kotlin)
        .unwrap()
        .write_to_dir(&dir)
        .unwrap();
    std::fs::write(dir.join("jni.h"), FAKE_JNI_H).unwrap();
    std::fs::write(dir.join("jni_test_support.h"), JNI_TEST_SUPPORT_H).unwrap();
    let jni_main = dir.join("jni_main.cpp");
    std::fs::write(&jni_main, JNI_HARNESS_CPP).unwrap();
    let jni_bin = dir.join("jni_e2e_bin");
    // ASan/UBSan: the glue borrows the direct buffer for every prop, so a dangling slice fails.
    let jni_link = Command::new("clang++")
        .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-g"])
        .args(["-fsanitize=address,undefined", "-fno-sanitize-recover=all"])
        .arg(format!("-I{}", dir.display()))
        .arg(&jni_main)
        .arg(dir.join("CryptoHybridJni.generated.cpp"))
        .arg(&staticlib)
        .arg("-o")
        .arg(&jni_bin)
        .output()
        .expect("link the JNI glue against the Rust staticlib");
    assert!(
        jni_link.status.success(),
        "JNI glue <-> Rust link failed:\n{}",
        String::from_utf8_lossy(&jni_link.stderr)
    );
    let jni_run = Command::new(&jni_bin)
        .output()
        .expect("run the JNI harness");
    assert!(
        jni_run.status.success(),
        "JNI harness failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&jni_run.stdout),
        String::from_utf8_lossy(&jni_run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drives the generated JNI entry points like the Kotlin client does (direct buffers, Java
/// strings), against the live Rust provider, through every success and error path.
const JNI_HARNESS_CPP: &str = include_str!("fixtures/jni_harness.cpp");
const JNI_TEST_SUPPORT_H: &str = include_str!("fixtures/jni_test_support.h");

#[test]
fn explicit_interface_id_and_fixed_layout_static_asserts_work_in_hybrid_pipeline() {
    let spec_src = r#"
#[repr(C, align(16))]
#[nve_hybrid_struct(size = 16)]
pub struct AlignedRect {
    #[offset(0)]
    pub x: f32,
    #[offset(4)]
    pub y: f32,
    #[offset(8)]
    pub w: f32,
    #[offset(12)]
    pub h: f32,
}

#[nve_hybrid_module(name = "FixedSurface", interface_id = 1)]
pub trait FixedSurfaceSpec {
    fn bounds(&self) -> AlignedRect;
}

#[nve_hybrid_module(name = "UserAnalytics")]
pub trait UserAnalyticsSpec {
    fn track(&self, code: u32) -> bool;
}
"#;

    let spec = parse_hybrid_rust_spec(spec_src).expect("parse fixed interface_id + layout spec");
    assert_eq!(spec.modules[0].interface_id(), 1);
    assert!(spec.modules[1].interface_id() >= NVE_HYBRID_INTERFACE_ID_BASE);
    assert_eq!(spec.structs[0].align, Some(16));
    assert_eq!(spec.structs[0].expected_size, Some(16));
    assert_eq!(spec.structs[0].fields[3].offset, Some(12));

    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();
    let abi_h = &cpp_out.files[0].content;
    assert!(abi_h.contains("#define NVE_HYBRID_FIXED_SURFACE_INTERFACE_ID UINT32_C(0x00000001)"));
    assert!(abi_h.contains("NVE_STATIC_ASSERT(sizeof(NveHybridAlignedRect) == 16u"));
    assert!(abi_h.contains("NVE_STATIC_ASSERT(offsetof(NveHybridAlignedRect, h) == 12u"));

    // Explicit interface_id = 0 must be rejected.
    let zero_id = parse_hybrid_rust_spec(
        r#"
#[nve_hybrid_module(name = "InvalidZero", interface_id = 0)]
pub trait InvalidZeroSpec {
    fn ping(&self) -> u32;
}
"#,
    );
    assert!(matches!(zero_id, Err(CodegenError::Validation(_))));

    // Duplicate explicit interface_id must be rejected.
    let dup_id = parse_hybrid_rust_spec(
        r#"
#[nve_hybrid_module(name = "ModA", interface_id = 7)]
pub trait ModASpec {
    fn a(&self) -> u32;
}
#[nve_hybrid_module(name = "ModB", interface_id = 7)]
pub trait ModBSpec {
    fn b(&self) -> u32;
}
"#,
    );
    assert!(matches!(dup_id, Err(CodegenError::Validation(_))));
}

#[test]
fn hybrid_emitters_escape_interpolation_quotes_and_newlines_across_all_6_targets() {
    let adversarial_spec = r#"
/// Doc line 1
/// Doc line 2
#[nve_hybrid_view(name = "SafeBanner")]
pub trait SafeBannerSpec {
    #[prop(default = "$HOME/${USER}")]
    fn env_template(&self) -> String;

    #[prop(default = "it's a \\ \"quoted\" \\(swift)\nline2\r\t$price")]
    fn rich_literal(&self) -> String;

    #[prop(default = -9223372036854775808)]
    fn min_i64(&self) -> i64;

    #[event]
    fn on_banner(&self, text: String);

    #[method]
    fn echo(&self, input: &str) -> String;
}
"#;

    let spec = parse_hybrid_rust_spec(adversarial_spec).expect("parse adversarial hybrid spec");
    assert_eq!(
        spec.views[0].props[2].default_value,
        Some(HybridDefaultValue::Int(i64::MIN))
    );

    let all = generate_hybrid_all(&spec, HybridTarget::ALL).expect("generate all 6 targets");
    assert_eq!(all.len(), 6);

    // 1. Kotlin: `$` must be escaped to `\$` so `$HOME`, `${USER}`, `$price` never interpolate.
    let kt_out = generate_hybrid_target(&spec, HybridTarget::Kotlin).unwrap();
    let kt_src = &kt_out.files[0].content;
    assert!(kt_src.contains("\\$HOME/\\${USER}"));
    assert!(kt_src.contains("\\$price"));
    assert!(!kt_src.replace("\\$", "").contains("$HOME"));
    assert!(!kt_src.replace("\\$", "").contains("${USER}"));

    // 2. Dart: `'`, `$`, `\`, `\n`, `\r`, `\t` must all be escaped inside single-quoted literals.
    let dart_out = generate_hybrid_target(&spec, HybridTarget::Dart).unwrap();
    let dart_src = &dart_out.files[0].content;
    assert!(dart_src.contains("'\\$HOME/\\${USER}'"));
    assert!(dart_src.contains("it\\'s a \\\\ \"quoted\" \\\\(swift)\\nline2\\r\\t\\$price"));
    assert!(!dart_src.replace("\\$", "").contains("'$HOME"));
    assert!(!dart_src.replace("\\$", "").contains("${USER}"));

    // 3. Swift: `\` must be escaped so `\(swift)` becomes `\\(swift)` (no Swift string interpolation).
    let swift_out = generate_hybrid_target(&spec, HybridTarget::Swift).unwrap();
    let swift_src = &swift_out.files[0].content;
    assert!(swift_src.contains("\\\\(swift)"));
    assert!(swift_src.contains("\\\"quoted\\\""));

    // 4. Rust + C++20: verify real compilers accept the emitted code cleanly!
    let rust_out = generate_hybrid_target(&spec, HybridTarget::Rust).unwrap();
    let cpp_out = generate_hybrid_target(&spec, HybridTarget::Cpp).unwrap();

    if common::require_tool("clang++") {
        let dir = temp_test_dir("adversarial_cpp");
        common::write_generated_c_abi_headers(&dir);
        cpp_out.write_to_dir(&dir).unwrap();
        let driver = dir.join("check.cpp");
        std::fs::write(
            &driver,
            r#"
#include <cassert>
#include "SafeBannerHybridSpec.generated.hpp"
void check() {
  nve::hybrid::SafeBannerProps p;
  assert(p.envTemplate == "$HOME/${USER}");
}
"#,
        )
        .unwrap();
        let out = Command::new("clang++")
            .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
            .arg(format!("-I{}", dir.display()))
            .arg(&driver)
            .output()
            .expect("clang++ adversarial hybrid cpp");
        assert!(
            out.status.success(),
            "adversarial hybrid C++20 failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Generated Swift against the static host runtime (`common::swift`); the full Swift <-> Rust
    // run lives in `hybrid_swift_runtime_tests`.
    if common::require_tool("swiftc") {
        let dir = temp_test_dir("adversarial_swift");
        let host = common::swift::build_host_swift(&dir, &[]);
        cpp_out.write_to_dir(&dir).unwrap();
        swift_out.write_to_dir(&dir).unwrap();
        common::swift::typecheck_generated(
            &host,
            &dir.join("SafeBannerHybridAbi.generated.h"),
            &dir.join("SafeBannerHybridSpec.generated.swift"),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    if common::require_tool("dart") {
        let dir = temp_test_dir("adversarial_dart");
        let lib_dir = dir.join("lib");
        std::fs::create_dir_all(&lib_dir).unwrap();
        dart_out.write_to_dir(&lib_dir).unwrap();
        std::fs::write(
            dir.join("pubspec.yaml"),
            "name: nve_adv_check\nenvironment:\n  sdk: '>=3.4.0 <4.0.0'\n",
        )
        .unwrap();
        write_offline_dart_package_config(&dir, "nve_adv_check");
        let out = Command::new("dart")
            .args(["analyze", "--fatal-infos"])
            .arg(&dir)
            .output()
            .expect("dart analyze adversarial hybrid dart");
        assert!(
            out.status.success(),
            "adversarial hybrid Dart failed dart analyze:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Real rustc verification of adversarial Rust output
    let dir = temp_test_dir("adversarial_rust");
    rust_out.write_to_dir(&dir).unwrap();
    common::rustc::Rustc::locate().build_lib(&dir, "safe_banner_hybrid", &dir.join("safe_banner_hybrid.generated.rs"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fuzz_hybrid_parser_never_panics_on_adversarial_and_truncated_inputs() {
    // 1. Sampled UTF-8 prefix truncations across the entire realistic hybrid spec
    let indices: Vec<usize> = REALISTIC_HYBRID_SPEC
        .char_indices()
        .map(|(i, _)| i)
        .collect();
    for (step, &byte_idx) in indices.iter().enumerate() {
        if step % 3 == 0 || byte_idx + 64 >= REALISTIC_HYBRID_SPEC.len() {
            let _ = parse_hybrid_rust_spec(&REALISTIC_HYBRID_SPEC[..byte_idx]);
        }
    }

    // 2. Deep nesting & numeric edge cases (must return Err cleanly, never panic/stack overflow)
    let edge_cases = [
        format!(
            "#[nve_hybrid_module(name = \"Deep\")] pub trait DeepSpec {{ fn f(&self) -> {}u32{}; }}",
            "Option<".repeat(256),
            ">".repeat(256)
        ),
        format!(
            "#[nve_hybrid_module(name = \"Ptr\")] pub trait PtrSpec {{ fn f(&self) -> {}u8; }}",
            "*const ".repeat(256)
        ),
        format!(
            "#[nve_hybrid_module(name = \"Ref\")] pub trait RefSpec {{ fn f(&self) -> {}u8; }}",
            "&".repeat(256)
        ),
        "#[nve_hybrid_view(name = \"V\")] pub trait VSpec { #[prop(default = -999999999999999999999999)] fn x(&self) -> i64; }".to_owned(),
        "#[nve_hybrid_enum] pub enum E { A = 4294967295, B }".to_owned(),
        "#[nve_hybrid_enum] pub enum E { A = 9999999999999999999999999 }".to_owned(),
        "#[nve_hybrid_module(name = \"M\")] #[nve_hybrid_view(name = \"V\")] pub trait Dual {}".to_owned(),
        "\"\\".to_owned(),
        "/* /* nested unclosed".to_owned(),
        "0x".to_owned(),
        "1e+".to_owned(),
        "#[".to_owned(),
        "#[a(".to_owned(),
        "pub(crate".to_owned(),
    ];
    for case in &edge_cases {
        assert!(
            parse_hybrid_rust_spec(case).is_err(),
            "must be rejected: {case}"
        );
    }

    // 3. 1,200 deterministic PRNG adversarial token streams
    let tokens: &[&str] = &[
        "#[nve_hybrid_enum]",
        "#[nve_hybrid_struct]",
        "#[nve_hybrid_union]",
        "#[nve_hybrid_module(name = \"M\")]",
        "#[nve_hybrid_view(name = \"V\")]",
        "#[nve_hybrid_const]",
        "#[nve_hybrid_alias]",
        "#[nve_hybrid_c_fn]",
        "#[prop(default = \"$HOME/${x}\")]",
        "#[prop(default = -1)]",
        "#[event]",
        "#[method]",
        "#[repr(C, align(8))]",
        "pub",
        "enum",
        "struct",
        "union",
        "trait",
        "fn",
        "const",
        "type",
        "extern",
        "\"C\"",
        "use",
        "&self",
        "&mut self",
        "Option",
        "Vec",
        "Result",
        "i32",
        "u64",
        "f32",
        "String",
        "NveValue",
        "{",
        "}",
        "(",
        ")",
        "[",
        "]",
        "<",
        ">",
        ":",
        "::",
        ";",
        ",",
        "=",
        "->",
        "&",
        "-",
        "*",
        ".",
        "!",
        "#",
        "/// doc\n",
        "/* c */",
        "'a",
        "0xFF",
        "1.5e-2",
        "\"\\n\\t\"",
        "foo",
    ];
    let mut seed: u64 = 0xD1B5_4A32_D192_ED03;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for _ in 0..1200 {
        let len = (next_u64() as usize % 32) + 1;
        let mut s = String::with_capacity(len * 8);
        for _ in 0..len {
            s.push_str(tokens[(next_u64() as usize) % tokens.len()]);
            s.push(' ');
        }
        let _ = parse_hybrid_rust_spec(&s);
    }
}
