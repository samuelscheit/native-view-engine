//! Dedicated verification for the Dart Hybrid FFI codegen target and static `host-dart` runtime:
//! - Structural invariants (zero `nve_flutter` dependency, zero raw typedef preludes, lossless
//!   `NveValueTree` / `writeDynamicValue` without `NVE_VAL_BYTES` double-wrapping, zero-arena POD calls)
//! - Strict `dart analyze --fatal-infos` with strict lints (`no_leading_underscores_for_local_identifiers`,
//!   `curly_braces_in_flow_control_structures`, `prefer_final_locals`, `avoid_init_to_null`,
//!   `unnecessary_this`, `strict-casts`, `strict-inference`, `strict-raw-types`) across realistic,
//!   enum-only, and multi-`Option` + heavy shadowing specs
//! - Real live cross-language Dart VM <-> Rust `cdylib` end-to-end execution over `dart:ffi`!

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");

const MULTI_OPTION_SHADOW_SPEC: &str = r#"
use nve_abi::NveValue;

#[nve_hybrid_struct]
pub struct RawPod {
    pub raw: u32,
}

#[nve_hybrid_struct]
pub struct ShadowRecord {
    pub w: String,
    pub r: Option<String>,
    pub mark: Option<i32>,
    pub count: Vec<u8>,
    pub value: NveValue,
}

#[nve_hybrid_module(name = "MultiOptShadow")]
pub trait MultiOptShadowSpec {
    fn collide(
        &self,
        vtable: RawPod,
        handle_name: String,
        fn_arg: Option<u64>,
        arena: Option<String>,
        builder: Option<RawPod>,
        props: Option<Vec<i32>>,
        record: Option<ShadowRecord>,
        dyn_val: NveValue,
    ) -> Result<ShadowRecord, String>;
    fn pod_only(&self, a: i64, b: i64) -> i64;
}

#[nve_hybrid_view(name = "ShadowView")]
pub trait ShadowViewSpec {
    #[prop]
    fn builder(&self) -> String;
    #[prop(default = None)]
    fn opt_pod(&self) -> Option<RawPod>;
    #[prop(default = None)]
    fn opt_rec(&self) -> Option<ShadowRecord>;
    #[prop]
    fn meta(&self) -> NveValue;
    #[method]
    fn current_meta(&self) -> NveValue;
    #[method]
    fn summary(&self) -> String;
}
"#;

const STRICT_ANALYSIS_OPTIONS: &str = r#"
analyzer:
  language:
    strict-casts: true
    strict-inference: true
    strict-raw-types: true

linter:
  rules:
    - always_declare_return_types
    - annotate_overrides
    - avoid_dynamic_calls
    - avoid_init_to_null
    - curly_braces_in_flow_control_structures
    - no_leading_underscores_for_local_identifiers
    - non_constant_identifier_names
    - prefer_const_constructors
    - prefer_final_fields
    - prefer_final_locals
    - type_annotate_public_apis
    - unnecessary_this
"#;

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_hybrid_dart_rt_test_{}_{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn to_file_uri(p: &Path) -> String {
    let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = canonical.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn setup_offline_dart_pkg(dir: &Path, pkg_name: &str) {
    let host_root = common::prepare_offline_nve_host_pkg(dir);
    let dart_tool = dir.join(".dart_tool");
    std::fs::create_dir_all(&dart_tool).expect("create .dart_tool");
    std::fs::write(
        dir.join("pubspec.yaml"),
        format!(
            "name: {pkg_name}\nenvironment:\n  sdk: '>=3.4.0 <4.0.0'\ndependencies:\n  nve_host:\n    path: nve_host_pkg\n"
        ),
    )
    .expect("write pubspec.yaml");
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
      "name": "nve_host",
      "rootUri": "{}",
      "packageUri": "lib/",
      "languageVersion": "3.4"
    }}
  ]
}}
"#,
        to_file_uri(dir),
        to_file_uri(&host_root),
    );
    std::fs::write(dart_tool.join("package_config.json"), config)
        .expect("write package_config.json");
}

fn locate_nve_abi_and_dep_dirs() -> (PathBuf, Vec<PathBuf>) {
    let current_exe = std::env::current_exe().expect("current_exe");
    let exe_dir = current_exe.parent().expect("exe parent dir");

    let rlib_has_wire_feature = |path: &Path| -> bool {
        const NEEDLE: &[u8] = b"encode_bin_value_to_vec";
        std::fs::read(path)
            .map(|bytes| bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE))
            .unwrap_or(false)
    };
    let find_best_in = |dir: &Path| -> Option<PathBuf> {
        let mut matches: Vec<(bool, std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
            })
            .map(|p| {
                let mtime = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                (rlib_has_wire_feature(&p), mtime, p)
            })
            .collect();
        matches.sort_by_key(|(has_wire, mtime, _)| (*has_wire, *mtime));
        matches.pop().map(|(_, _, path)| path)
    };

    if let Some(rlib) = find_best_in(exe_dir) {
        return (rlib, vec![exe_dir.to_path_buf()]);
    }

    let build_root = current_exe
        .ancestors()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("build"))
        .expect("expected Cargo deps/ or build/ ancestor directory");
    let mut candidates: Vec<(bool, std::time::SystemTime, PathBuf)> = Vec::new();
    let mut dep_dirs: Vec<PathBuf> = Vec::new();

    if let Ok(pkgs) = std::fs::read_dir(build_root) {
        for pkg_entry in pkgs.filter_map(|e| e.ok()) {
            let pkg_path = pkg_entry.path();
            if !pkg_path.is_dir() {
                continue;
            }
            let is_nve_abi = pkg_entry.file_name() == "nve-abi";
            let Ok(hashes) = std::fs::read_dir(&pkg_path) else {
                continue;
            };
            for hash_entry in hashes.filter_map(|e| e.ok()) {
                let out_dir = hash_entry.path().join("out");
                if !out_dir.is_dir() {
                    continue;
                }
                if is_nve_abi && let Some(rlib) = find_best_in(&out_dir) {
                    let mtime = std::fs::metadata(&rlib)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    candidates.push((rlib_has_wire_feature(&rlib), mtime, rlib));
                }
                let has_lib = std::fs::read_dir(&out_dir).ok().is_some_and(|it| {
                    it.filter_map(|e| e.ok()).any(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| matches!(ext, "rlib" | "dll" | "so" | "dylib"))
                    })
                });
                if has_lib {
                    dep_dirs.push(out_dir);
                }
            }
        }
    }
    candidates.sort_by_key(|(has_wire, mtime, _)| (*has_wire, *mtime));
    let rlib = candidates
        .pop()
        .map(|(_, _, p)| p)
        .expect("libnve_abi-*.rlib must exist");
    (rlib, dep_dirs)
}

fn add_dep_dirs(cmd: &mut Command, temp_dir: &Path, dep_dirs: &[PathBuf]) {
    if dep_dirs.len() <= 8 {
        for dir in dep_dirs {
            cmd.arg("-L").arg(format!("dependency={}", dir.display()));
        }
        return;
    }
    let rsp = temp_dir.join("rustc_deps.rsp");
    let mut body = String::new();
    for dir in dep_dirs {
        body.push_str("-L\ndependency=");
        body.push_str(&dir.to_string_lossy());
        body.push('\n');
    }
    std::fs::write(&rsp, body).expect("write dep rsp");
    cmd.arg(format!("@{}", rsp.display()));
}

#[test]
fn generated_dart_uses_static_host_dart_runtime_without_double_wrapping() {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse realistic spec");
    let dart_out = generate_hybrid_target(&spec, HybridTarget::Dart).expect("generate dart");
    let src = &dart_out.files[0].content;

    assert!(src.contains("// ignore_for_file: no_leading_underscores_for_local_identifiers"));
    assert!(src.contains("import 'package:nve_host/nve_host.dart';"));
    assert!(src.contains("import 'package:nve_host/nve_host_ffi.dart';"));
    assert!(!src.contains("package:nve_flutter"));
    assert!(!src.contains("_NveGetInterfaceNative"));
    assert!(!src.contains("NveHybridDynamicValue"));
    assert!(!src.contains("NveBinWriter"));
    assert!(!src.contains("NveBinReader"));
    assert!(
        !src.contains(".tag = 5;"),
        "HybridType::Value must never be hardcoded to tag = 5 (NVE_VAL_BYTES)"
    );
    assert!(src.contains("_nveB.writeDynamicValue('metadata', metadata);"));
    assert!(src.contains("NveValueTree.fromDynamic(_nveArena, meta).root"));

    // Pure-POD methods (`add`, `seek`) must not allocate a `HybridArena`.
    let add_start = src.find("int add(int a, int b)").expect("add method");
    let add_end = src[add_start..].find("\n  }").expect("add end") + add_start;
    assert!(
        !src[add_start..add_end].contains("HybridArena"),
        "pure-POD method `add` must not allocate a HybridArena:\n{}",
        &src[add_start..add_end]
    );

    // Enum-only spec must omit unused `dart:ffi` / `nve_host_ffi.dart` imports.
    let enum_only = parse_hybrid_rust_spec(
        "#[nve_hybrid_enum]\npub enum OnlyEnum { Alpha = 0, Beta = 1 }",
    )
    .expect("parse enum-only spec");
    let enum_src = &generate_hybrid_target(&enum_only, HybridTarget::Dart)
        .expect("generate enum-only dart")
        .files[0]
        .content;
    assert!(enum_src.contains("import 'package:nve_host/nve_host.dart';"));
    assert!(!enum_src.contains("import 'dart:ffi'"));
    assert!(!enum_src.contains("nve_host_ffi.dart"));
}

#[test]
fn strict_dart_analyzer_accepts_realistic_enum_only_and_multi_option_shadowing_specs() {
    if !common::require_tool("dart") {
        return;
    }

    let dir = temp_test_dir("strict_analyze");
    let lib_dir = dir.join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    setup_offline_dart_pkg(&dir, "nve_strict_dart_check");
    std::fs::write(dir.join("analysis_options.yaml"), STRICT_ANALYSIS_OPTIONS).unwrap();

    for spec_src in [
        REALISTIC_HYBRID_SPEC,
        MULTI_OPTION_SHADOW_SPEC,
        "#[nve_hybrid_enum]\npub enum SoloKind { First = 0, Second = 2 }",
        "#[nve_hybrid_module(name = \"HandleShadow\")]\npub trait HandleShadowSpec { fn ping(&self, handle: String, vtable: i32, arena: Option<String>, builder: Option<i64>) -> i32; }",
    ] {
        let spec = parse_hybrid_rust_spec(spec_src).expect("parse spec");
        generate_hybrid_target(&spec, HybridTarget::Dart)
            .expect("generate dart")
            .write_to_dir(&lib_dir)
            .expect("write dart");
    }

    let check = Command::new("dart")
        .args(["analyze", "--fatal-infos"])
        .arg(&dir)
        .output()
        .expect("run strict dart analyze");
    assert!(
        check.status.success(),
        "Generated Dart failed strict `dart analyze --fatal-infos`:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_cross_language_dart_vm_calls_generated_rust_cdylib_end_to_end() {
    let _touch = nve_abi::NveValue::null();
    if !common::require_tool("dart") {
        return;
    }

    let dir = temp_test_dir("dart_rust_e2e");
    let lib_dir = dir.join("lib");
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::create_dir_all(&bin_dir).unwrap();

    let realistic = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).unwrap();
    let multi_opt = parse_hybrid_rust_spec(MULTI_OPTION_SHADOW_SPEC).unwrap();

    generate_hybrid_target(&realistic, HybridTarget::Rust)
        .unwrap()
        .write_to_dir(&dir)
        .unwrap();
    generate_hybrid_target(&multi_opt, HybridTarget::Rust)
        .unwrap()
        .write_to_dir(&dir)
        .unwrap();
    generate_hybrid_target(&realistic, HybridTarget::Dart)
        .unwrap()
        .write_to_dir(&lib_dir)
        .unwrap();
    generate_hybrid_target(&multi_opt, HybridTarget::Dart)
        .unwrap()
        .write_to_dir(&lib_dir)
        .unwrap();

    let (nve_abi_rlib, dep_dirs) = locate_nve_abi_and_dep_dirs();

    // Compile both generated Rust modules as rlibs.
    let compile_rlib = |crate_name: &str, src_name: &str| -> PathBuf {
        let out = dir.join(format!("lib{crate_name}.rlib"));
        let mut cmd = Command::new("rustc");
        cmd.args([
            "--edition=2024",
            "--crate-type=lib",
            "-D",
            "warnings",
            "--crate-name",
            crate_name,
        ]);
        add_dep_dirs(&mut cmd, &dir, &dep_dirs);
        let st = cmd
            .arg("--extern")
            .arg(format!("nve_abi={}", nve_abi_rlib.display()))
            .arg(dir.join(src_name))
            .arg("-o")
            .arg(&out)
            .output()
            .expect("compile generated rust rlib");
        assert!(
            st.status.success(),
            "rustc {crate_name} failed:\n{}",
            String::from_utf8_lossy(&st.stderr)
        );
        out
    };

    let crypto_rlib = compile_rlib("crypto_hybrid", "crypto_hybrid.generated.rs");
    let multi_rlib = compile_rlib(
        "multi_opt_shadow_hybrid",
        "multi_opt_shadow_hybrid.generated.rs",
    );

    // Build a real Rust cdylib implementing all four interfaces (`Crypto`, `VideoSurface`,
    // `MultiOptShadow`, `ShadowView`) and exporting `nve_get_interface`.
    let provider_rs = dir.join("provider.rs");
    std::fs::write(
        &provider_rs,
        r#"
use std::ffi::c_void;
use crypto_hybrid::*;
use multi_opt_shadow_hybrid as ms;

#[derive(Default)]
struct LiveCrypto;

impl CryptoHybrid for LiveCrypto {
    fn version(&mut self) -> String { "\u{FEFF}nve-dart-1.0".to_string() }
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

#[derive(Default)]
struct LiveMultiOpt;

impl ms::MultiOptShadowHybrid for LiveMultiOpt {
    fn collide(
        &mut self,
        vtable: ms::RawPod,
        handle_name: &str,
        fn_arg: Option<u64>,
        arena: Option<String>,
        builder: Option<ms::RawPod>,
        props: Option<Vec<i32>>,
        record: Option<ms::ShadowRecord>,
        dyn_val: ms::NveHybridValue,
    ) -> Result<ms::ShadowRecord, String> {
        let mut parts = vec![format!("{handle_name}:{}", vtable.raw)];
        if let Some(v) = fn_arg { parts.push(format!("fn={v}")); }
        if let Some(a) = &arena { parts.push(format!("arena={a}")); }
        if let Some(b) = builder { parts.push(format!("builder={}", b.raw)); }
        if let Some(p) = &props { parts.push(format!("props={}", p.len())); }
        if let Some(rec) = &record { parts.push(format!("rec={}", rec.w)); }
        Ok(ms::ShadowRecord {
            w: parts.join("|"),
            r: arena,
            mark: builder.map(|b| b.raw as i32),
            count: props.map(|p| p.into_iter().map(|x| x as u8).collect()).unwrap_or_default(),
            value: dyn_val,
        })
    }
    fn pod_only(&mut self, a: i64, b: i64) -> i64 { a * b }
}

#[derive(Default)]
struct LiveShadowView {
    props: ms::ShadowViewProps,
}

impl ms::ShadowViewHybrid for LiveShadowView {
    fn apply_props(&mut self, props: ms::ShadowViewProps) { self.props = props; }
    fn current_meta(&mut self) -> ms::NveHybridValue { self.props.meta.clone() }
    fn summary(&mut self) -> String {
        format!(
            "{}|{}|{}",
            self.props.builder,
            self.props.opt_pod.map_or(-1, |p| p.raw as i64),
            self.props.opt_rec.as_ref().map_or("none", |r| r.w.as_str()),
        )
    }
}

static CRYPTO_VTABLE: NveHybridCryptoVTable = build_crypto_vtable::<LiveCrypto>();
static VIDEO_VTABLE: NveHybridVideoSurfaceVTable = build_video_surface_vtable::<LiveVideo>();
static MULTI_VTABLE: ms::NveHybridMultiOptShadowVTable = ms::build_multi_opt_shadow_vtable::<LiveMultiOpt>();
static SHADOW_VIEW_VTABLE: ms::NveHybridShadowViewVTable = ms::build_shadow_view_vtable::<LiveShadowView>();

#[unsafe(no_mangle)]
pub extern "C" fn nve_get_interface(interface_id: u32) -> *const c_void {
    match interface_id {
        NVE_HYBRID_CRYPTO_INTERFACE_ID => (&CRYPTO_VTABLE as *const NveHybridCryptoVTable).cast(),
        NVE_HYBRID_VIDEO_SURFACE_INTERFACE_ID => (&VIDEO_VTABLE as *const NveHybridVideoSurfaceVTable).cast(),
        ms::NVE_HYBRID_MULTI_OPT_SHADOW_INTERFACE_ID => (&MULTI_VTABLE as *const ms::NveHybridMultiOptShadowVTable).cast(),
        ms::NVE_HYBRID_SHADOW_VIEW_INTERFACE_ID => (&SHADOW_VIEW_VTABLE as *const ms::NveHybridShadowViewVTable).cast(),
        _ => std::ptr::null(),
    }
}
"#,
    )
    .unwrap();

    let cdylib_name = format!(
        "{}nve_dart_hybrid_provider{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let cdylib_path = dir.join(&cdylib_name);
    let mut cdylib_cmd = Command::new("rustc");
    cdylib_cmd.args(["--edition=2024", "--crate-type=cdylib", "-D", "warnings"]);
    add_dep_dirs(&mut cdylib_cmd, &dir, &dep_dirs);
    let cdylib_out = cdylib_cmd
        .arg("--extern")
        .arg(format!("nve_abi={}", nve_abi_rlib.display()))
        .arg("--extern")
        .arg(format!("crypto_hybrid={}", crypto_rlib.display()))
        .arg("--extern")
        .arg(format!("multi_opt_shadow_hybrid={}", multi_rlib.display()))
        .arg(&provider_rs)
        .arg("-o")
        .arg(&cdylib_path)
        .output()
        .expect("build Rust cdylib");
    assert!(
        cdylib_out.status.success(),
        "Rust cdylib build failed:\n{}",
        String::from_utf8_lossy(&cdylib_out.stderr)
    );

    setup_offline_dart_pkg(&dir, "nve_dart_e2e");
    std::fs::write(dir.join("analysis_options.yaml"), STRICT_ANALYSIS_OPTIONS).unwrap();

    // Write the live Dart VM e2e test driver!
    let main_dart = bin_dir.join("e2e.dart");
    std::fs::write(
        &main_dart,
        r#"
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:nve_dart_e2e/crypto_hybrid.generated.dart';
import 'package:nve_dart_e2e/multi_opt_shadow_hybrid.generated.dart';
import 'package:nve_host/nve_host.dart';

void expect(bool cond, String msg) {
  if (!cond) {
    throw StateError('ASSERTION FAILED: $msg');
  }
}

void expectEq(Object? actual, Object? expected, String msg) {
  if (actual != expected) {
    throw StateError('ASSERTION FAILED ($msg): expected $expected, got $actual');
  }
}

bool sameBytes(List<int> a, List<int> b) {
  if (a.length != b.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true;
}

void main(List<String> args) {
  final lib = ffi.DynamicLibrary.open(args.first);

  // ---- 1. Crypto module ----
  final crypto = CryptoHybridClient.resolve(lib);
  expectEq(crypto.version(), '\uFEFFnve-dart-1.0', 'version preserves BOM');
  expectEq(crypto.add(19, 23), 42, 'add');

  final input = Uint8List.fromList(<int>[0x10, 0x20, 0x30]);
  final hashed = crypto.blake3(input);
  expect(sameBytes(hashed, <int>[0x4A, 0x7A, 0x6A]), 'blake3 xor');

  final hdr = crypto.hashWith(HashAlgorithm.sha512, input);
  expectEq(hdr.algorithm, HashAlgorithm.sha512, 'hdr.algorithm');
  expectEq(hdr.byteLen, 3, 'hdr.byteLen');
  expectEq(hdr.verified, true, 'hdr.verified');
  expectEq(hdr.anchor.x, 12.5, 'hdr.anchor.x');
  expectEq(hdr.anchor.y, -4.25, 'hdr.anchor.y');

  expectEq(crypto.findNonce('hello', 100), 105, 'findNonce some');
  expectEq(crypto.findNonce('hello', null), null, 'findNonce none');

  final payload = SignedPayload(
    header: const DigestHeader(
      algorithm: HashAlgorithm.sha256,
      byteLen: 8,
      verified: true,
      anchor: FramePoint(x: 1.0, y: 2.0),
    ),
    label: 'ok',
    digest: Uint8List.fromList(<int>[1, 2, 3]),
    tags: const <String>['alpha', 'beta'],
    nonce: 1337,
  );
  expectEq(crypto.verifyPayload(payload), true, 'verifyPayload ok');

  var caughtLabel = false;
  try {
    crypto.verifyPayload(
      SignedPayload(
        header: payload.header,
        label: '',
        digest: Uint8List(0),
        tags: const <String>[],
        nonce: null,
      ),
    );
  } on HybridException catch (e) {
    caughtLabel = e.status == HybridStatus.err && e.message == 'missing payload label';
  }
  expect(caughtLabel, 'verifyPayload err throws HybridException');

  expectEq(crypto.lookupPayload(0), null, 'lookupPayload none');
  final found = crypto.lookupPayload(7);
  expect(found != null, 'lookupPayload some');
  expectEq(found!.header.algorithm, HashAlgorithm.blake3, 'found.algorithm');
  expectEq(found.header.byteLen, 28, 'found.byteLen');
  expectEq(found.label, 'rec-7', 'found.label');
  expect(sameBytes(found.digest, <int>[0xDE, 0xAD, 0xBE, 0xEF]), 'found.digest');
  expectEq(found.tags.length, 2, 'found.tags len');
  expectEq(found.nonce, 4242, 'found.nonce');

  final batch = crypto.batchSign(const <String>['one', 'two'], HashAlgorithm.sha256);
  expectEq(batch.length, 2, 'batchSign len');
  expectEq(batch[0].label, 'one', 'batch[0].label');
  expectEq(batch[1].label, 'two', 'batch[1].label');
  crypto.resetCache();

  // ---- 2. HybridType::Value across method param & return (NO NVE_VAL_BYTES double-wrapping!) ----
  final minI64 = const Key64(0, 0x80000000).toSignedInt();
  final maxI64 = const Key64(0xFFFFFFFF, 0x7FFFFFFF).toSignedInt();
  final richMeta = DynamicValue.fromObject(<String, Object?>{
    'null': null,
    'flag': true,
    'min': minI64,
    'max': maxI64,
    'f': 2.5,
    'text': '\uFEFFhéllo 🚀',
    'raw': Uint8List.fromList(<int>[0x00, 0xAA, 0xFF]),
    'emptyArr': <Object?>[],
    'emptyMap': <String, Object?>{},
    'arr': <Object?>[
      1,
      'x',
      null,
      <String, Object?>{'nested': false},
    ],
  });
  final echoed = crypto.inspectDynamic(richMeta);
  expectEq(echoed.tag, BinTag.map, 'inspectDynamic preserves MAP tag (not BYTES)');
  expectEq(echoed, richMeta, 'inspectDynamic byte-identical round-trip');
  expectEq(crypto.inspectDynamic(DynamicValue.nullValue), DynamicValue.nullValue, 'inspectDynamic null');

  // ---- 3. Cursor shadowing + multi-Option + nested Vec<Vec<i32>> ----
  final cursor = Cursor(
    mark: 1,
    w: Uint8List.fromList(<int>[1, 2, 3]),
    r: 'ab',
    count: 4,
    tag: 42,
    grid: const <List<int>>[
      <int>[1, 2, 3],
      <int>[4, 5, 6],
    ],
  );
  final t = crypto.transpose(cursor);
  expectEq(t.mark, 2, 'cursor.mark');
  expectEq(t.count, 8, 'cursor.count');
  expectEq(t.r, 'AB', 'cursor.r');
  expectEq(t.tag, -42, 'cursor.tag');
  expect(sameBytes(t.w, <int>[3, 2, 1]), 'cursor.w');
  expectEq(t.grid.length, 3, 'cursor.grid rows');
  expectEq(t.grid[0][0], 1, 'cursor.grid[0][0]');
  expectEq(t.grid[0][1], 4, 'cursor.grid[0][1]');
  expectEq(t.grid[2][1], 6, 'cursor.grid[2][1]');

  // Invalid UTF-8 (lone surrogate) rejected BEFORE crossing FFI.
  var caughtUtf8 = false;
  try {
    crypto.batchSign(const <String>['\uD800'], HashAlgorithm.sha256);
  } on BinCodecException catch (e) {
    caughtUtf8 = e.kind == BinErrorKind.invalidUtf8;
  }
  expect(caughtUtf8, 'lone surrogate rejected before FFI');

  // ---- 4. VideoSurface view: setProps with lossless HybridType::Value (NO double-wrapping!) ----
  final video = VideoSurfaceHybridClient.resolve(lib);
  video.setProps(
    VideoSurfaceProps(
      sourceUrl: 'https://nve.dev/clip.mp4',
      muted: true,
      quality: VideoQuality.uhd4k,
    ),
  );
  video.seek(9876);
  final frame1 = video.captureFrame(const FramePoint(x: 5.0, y: 9.0));
  expect(sameBytes(frame1, <int>[5, 9, 24]), 'captureFrame baseline');

  // Now set present Option<String>, Option<FramePoint>, Bytes, Vec<i32>, AND rich DynamicValue metadata.
  video.setProps(
    VideoSurfaceProps(
      sourceUrl: 'https://nve.dev/clip.mp4',
      muted: true,
      quality: VideoQuality.uhd4k,
      posterUrl: 'p.png',
      focusPoint: const FramePoint(x: 7.0, y: 0.0),
      drmKey: Uint8List.fromList(<int>[0xAA, 0xBB]),
      chapters: const <int>[10, 20, 30],
      metadata: richMeta,
    ),
  );
  final frame2 = video.captureFrame(const FramePoint(x: 5.0, y: 9.0));
  expect(sameBytes(frame2, <int>[5, 9, 24, 7, 5, 0xAA, 0xBB, 10, 20, 30]), 'captureFrame with all props');
  final gotMeta = video.currentMetadata();
  expectEq(gotMeta.tag, BinTag.map, 'VideoSurface.metadata arrives as MAP, never BYTES');
  expectEq(gotMeta, richMeta, 'VideoSurface.metadata byte-identical round-trip');

  // Verify scalar & array DynamicValue props too (never double-wrapped).
  for (final sample in <DynamicValue>[
    DynamicValue.nullValue,
    DynamicValue.fromObject(false),
    DynamicValue.fromObject(-999),
    DynamicValue.fromObject(3.14),
    DynamicValue.fromObject('hello 🚀'),
    DynamicValue.fromObject(Uint8List.fromList(<int>[7, 8, 9])),
    DynamicValue.fromObject(<Object?>[1, 'two', true]),
  ]) {
    video.setProps(VideoSurfaceProps(sourceUrl: 'x', muted: true, metadata: sample));
    final cur = video.currentMetadata();
    expectEq(cur.tag, sample.tag, 'metadata prop tag ${sample.tag}');
    expectEq(cur, sample, 'metadata prop value ${sample.tag}');
  }

  // Error message with UTF-8 emoji round-trips cleanly.
  var caughtEmoji = false;
  try {
    video.captureFrame(const FramePoint(x: -1.0, y: 0.0));
  } on HybridException catch (e) {
    caughtEmoji = e.status == HybridStatus.err && e.message == 'bad \u{1F600}';
  }
  expect(caughtEmoji, 'captureFrame emoji error');

  // ---- 5. Multi-Option + heavy shadowing module & view ----
  final multi = MultiOptShadowHybridClient.resolve(lib);
  expectEq(multi.podOnly(6, 7), 42, 'podOnly');

  final recIn = ShadowRecord(
    w: 'inner',
    r: 'opt',
    mark: 3,
    count: Uint8List.fromList(<int>[9]),
    value: richMeta,
  );
  final rSome = multi.collide(
    const RawPod(raw: 11),
    'h',
    55,
    'ar',
    const RawPod(raw: 22),
    const <int>[1, 2, 3],
    recIn,
    richMeta,
  );
  expectEq(rSome.w, 'h:11|fn=55|arena=ar|builder=22|props=3|rec=inner', 'multi-opt all some');
  expectEq(rSome.r, 'ar', 'multi-opt r');
  expectEq(rSome.mark, 22, 'multi-opt mark');
  expect(sameBytes(rSome.count, <int>[1, 2, 3]), 'multi-opt count');
  expectEq(rSome.value, richMeta, 'multi-opt DynamicValue');

  final rNone = multi.collide(
    const RawPod(raw: 7),
    'z',
    null,
    null,
    null,
    null,
    null,
    DynamicValue.nullValue,
  );
  expectEq(rNone.w, 'z:7', 'multi-opt all none');
  expectEq(rNone.r, null, 'multi-opt none r');
  expectEq(rNone.mark, null, 'multi-opt none mark');
  expect(rNone.value.isNull, 'multi-opt none DynamicValue');

  final shadowView = ShadowViewHybridClient.resolve(lib);
  shadowView.setProps(
    ShadowViewProps(
      builder: 'my-builder',
      optPod: const RawPod(raw: 88),
      optRec: recIn,
      meta: richMeta,
    ),
  );
  expectEq(shadowView.summary(), 'my-builder|88|inner', 'ShadowView summary some');
  expectEq(shadowView.currentMeta(), richMeta, 'ShadowView meta lossless');
  shadowView.setProps(ShadowViewProps(builder: 'empty', meta: DynamicValue.nullValue));
  expectEq(shadowView.summary(), 'empty|-1|none', 'ShadowView summary none');
  expect(shadowView.currentMeta().isNull, 'ShadowView meta null');

  // ---- 6. Dispose & use-after-dispose ----
  crypto.dispose();
  crypto.dispose(); // idempotent
  var caughtDisposed = false;
  try {
    crypto.add(1, 2);
  } on HybridException catch (e) {
    caughtDisposed = e.status == HybridStatus.nullHandle;
  }
  expect(caughtDisposed, 'use-after-dispose throws HybridException(nullHandle)');

  video.dispose();
  multi.dispose();
  shadowView.dispose();
}
"#,
    )
    .unwrap();

    // First verify the test driver + generated files pass strict `dart analyze --fatal-infos`.
    let analyze = Command::new("dart")
        .args(["analyze", "--fatal-infos"])
        .arg(&dir)
        .output()
        .expect("dart analyze e2e package");
    assert!(
        analyze.status.success(),
        "Dart e2e package failed strict `dart analyze --fatal-infos`:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&analyze.stdout),
        String::from_utf8_lossy(&analyze.stderr)
    );

    // Now EXECUTE live Dart VM <-> Rust cdylib end-to-end!
    let pkg_config = dir.join(".dart_tool/package_config.json");
    let run = Command::new("dart")
        .arg("--enable-asserts")
        .arg(format!("--packages={}", pkg_config.display()))
        .arg(&main_dart)
        .arg(&cdylib_path)
        .current_dir(&dir)
        .output()
        .expect("run live Dart VM <-> Rust cdylib e2e");
    assert!(
        run.status.success(),
        "Live Dart VM <-> Rust cdylib e2e failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
