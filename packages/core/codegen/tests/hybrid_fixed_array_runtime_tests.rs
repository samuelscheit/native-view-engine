//! `HybridType::FixedArray` (`[T; N]`) in compiled C++20, Swift, and Dart against a live Rust
//! provider built from the generated Rust, and in TypeScript on Node: `NveBinValue` codecs
//! (lossless, every other element count rejected, inner rows too), POD C arrays (nested and
//! `N = 1` too) through `to_abi` / `from_abi`, `Option<[T; N]>` arguments and returns,
//! `Option<PodStruct>`, `Result<Option<[T; N]>, String>`, and `[T; N]` view props.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::rustc::Rustc;
use common::swift::{SWIFT_STRICT, build_host_swift, typecheck_generated};
use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const SPEC: &str = include_str!("fixtures/fixed_array/fixed_array_hybrid.nve.rs");
const PROVIDER_RS: &str = include_str!("fixtures/fixed_array/provider.rs");
const CPP_DRIVER: &str = include_str!("fixtures/fixed_array/driver.cpp");
const SWIFT_MAIN: &str = include_str!("fixtures/fixed_array/main.swift");
const DART_MAIN: &str = include_str!("fixtures/fixed_array/main.dart");
const TS_MAIN: &str = include_str!("fixtures/fixed_array/main.ts");
const EDGE_SPEC: &str = include_str!("fixtures/hybrid_rust/edge_hybrid.nve.rs");

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_fixed_array_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn generate(spec_src: &str, targets: &[HybridTarget], dir: &Path) {
    let spec = parse_hybrid_rust_spec(spec_src).expect("parse hybrid spec");
    for &target in targets {
        generate_hybrid_target(&spec, target)
            .expect("generate hybrid target")
            .write_to_dir(dir)
            .expect("write generated files");
    }
}

fn run_ok(cmd: &mut Command, what: &str) -> String {
    let out = cmd.output().expect("start the command");
    assert!(
        out.status.success(),
        "{what} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Generates Rust for `SPEC` into `dir` and builds the Rust provider as `crate_type` at `out`.
fn build_provider(dir: &Path, crate_type: &str, out: &Path) {
    generate(SPEC, &[HybridTarget::Rust], dir);
    let rustc = Rustc::locate();
    let src = dir.join("fixed_codec_hybrid.generated.rs");
    let rlib = rustc.build_lib(dir, "fixed_codec_hybrid", &src);
    let provider = dir.join("provider.rs");
    std::fs::write(&provider, PROVIDER_RS).expect("write provider");
    let externs = [("fixed_codec_hybrid", rlib.as_path())];
    rustc.build(dir, &[&format!("--crate-type={crate_type}")], &externs, &provider, out);
}

/// The provider as a staticlib, plus the C++ and Swift bindings, in `dir`.
fn build_static_provider(dir: &Path) -> PathBuf {
    generate(SPEC, &[HybridTarget::Cpp, HybridTarget::Swift], dir);
    let staticlib = dir.join("libfixed_provider.a");
    build_provider(dir, "staticlib", &staticlib);
    staticlib
}

#[test]
fn cpp20_fixed_arrays_cross_the_codec_c_abi_and_live_provider() {
    if !common::require_tool("clang++") {
        return;
    }
    let dir = temp_test_dir("cpp");
    common::write_generated_c_abi_headers(&dir);
    let staticlib = build_static_provider(&dir);
    let header = std::fs::read_to_string(dir.join("FixedCodecHybridSpec.generated.hpp"))
        .expect("read generated C++ header");
    assert!(header.contains("const std::array<uint32_t, 4>& coords"), "arrays pass by const&");
    assert!(header.contains("begin_array_exact(4u)"), "decode checks the exact length");
    assert!(!header.contains("memcpy"), "POD arrays convert element by element");
    let abi = std::fs::read_to_string(dir.join("FixedCodecHybridAbi.generated.h")).expect("read C header");
    assert!(abi.contains("uint32_t grid[3][2];") && abi.contains("bool flags[2][33];"));
    let point = abi.find("typedef struct NveHybridFixedPoint").expect("FixedPoint C struct");
    let pod = abi.find("typedef struct NveHybridFixedPod").expect("FixedPod C struct");
    assert!(point < pod, "element structs come first even when declared after their users");

    let driver = dir.join("driver.cpp");
    std::fs::write(&driver, CPP_DRIVER).expect("write C++ driver");
    let exe = dir.join(format!("driver{}", std::env::consts::EXE_SUFFIX));
    run_ok(
        Command::new("clang++")
            .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsanitize=address,undefined"])
            .arg(format!("-I{}", dir.display()))
            .arg(&driver)
            .arg(&staticlib)
            .arg("-o")
            .arg(&exe),
        "clang++ fixed-array driver",
    );
    assert_eq!(run_ok(&mut Command::new(&exe), "C++ fixed-array driver"), "cpp fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swift_fixed_arrays_cross_the_codec_c_abi_and_live_provider() {
    if !common::require_tool("swiftc") {
        return;
    }
    let dir = temp_test_dir("swift");
    let flags = ["-O", "-sanitize=address"];
    let host = build_host_swift(&dir, &flags);
    let staticlib = build_static_provider(&dir);
    let abi_header = dir.join("FixedCodecHybridAbi.generated.h");
    let swift_file = dir.join("FixedCodecHybridSpec.generated.swift");
    typecheck_generated(&host, &abi_header, &swift_file);

    let bridge = dir.join("bridge.h");
    std::fs::write(
        &bridge,
        "#include \"FixedCodecHybridAbi.generated.h\"\nconst void* nve_get_interface(uint32_t interface_id);\n",
    )
    .expect("write bridging header");
    let main = dir.join("main.swift");
    std::fs::write(&main, SWIFT_MAIN).expect("write Swift driver");
    let exe = dir.join(format!("fixed_array{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new("swiftc");
    cmd.args(SWIFT_STRICT).args(flags).arg("-import-objc-header").arg(&bridge);
    host.search_args(&mut cmd);
    cmd.arg(&main).arg(&swift_file).arg(&staticlib);
    host.lib_args(&mut cmd);
    run_ok(cmd.arg("-o").arg(&exe), "swiftc fixed-array driver");
    assert_eq!(run_ok(&mut Command::new(&exe), "Swift fixed-array driver"), "swift fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dart_fixed_arrays_cross_the_codec_ffi_arrays_and_live_provider() {
    if !common::require_tool("dart") {
        return;
    }
    let dir = temp_test_dir("dart");
    let cdylib = dir.join(format!(
        "{}fixed_provider{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    build_provider(&dir, "cdylib", &cdylib);
    generate(SPEC, &[HybridTarget::Dart], &dir.join("lib"));
    let dart = std::fs::read_to_string(dir.join("lib/fixed_codec_hybrid.generated.dart"))
        .expect("read generated Dart");
    assert!(dart.contains("@ffi.Array.multi([3, 2])\n  external ffi.Array<ffi.Array<ffi.Uint32>> grid;"));
    assert!(dart.contains("@ffi.Array.multi([2, 1])\n  external ffi.Array<ffi.Array<NveHybridFixedPoint>> cells;"));
    assert!(dart.contains("@ffi.Array(1)\n  external ffi.Array<ffi.Uint32> one;"));
    common::dart::setup_offline_dart_pkg(&dir, "nve_fixed_array");
    let driver = dir.join("bin/main.dart");
    std::fs::create_dir_all(dir.join("bin")).expect("create bin dir");
    std::fs::write(&driver, DART_MAIN).expect("write Dart driver");
    common::dart::analyze(&dir);
    assert_eq!(common::dart::run(&dir, &driver, &[&cdylib]), "dart fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ts_fixed_arrays_type_check_and_round_trip_on_node() {
    if !common::require_tool("node") {
        return;
    }
    let dir = temp_test_dir("ts");
    generate(SPEC, &[HybridTarget::TypeScript], &dir);
    let hook = common::ts::prepare_offline_host_ts_pkg(&dir);
    let driver = dir.join("main.ts");
    std::fs::write(&driver, TS_MAIN).expect("write TS driver");
    let tsconfig = dir.join("tsconfig.json");
    std::fs::write(&tsconfig, common::ts::STRICT_TSCONFIG).expect("write tsconfig.json");
    std::fs::write(dir.join("node_ambient.d.ts"), NODE_ASSERT_DTS).expect("write node_ambient.d.ts");
    std::fs::write(dir.join("package.json"), r#"{"type": "module"}"#).expect("write package.json");
    common::ts::run_tsc_no_emit(&tsconfig);
    assert_eq!(common::ts::run_node(&hook, &driver), "ts fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `node:assert/strict` as the drivers use it (`types: []` keeps `@types/node` out).
const NODE_ASSERT_DTS: &str = r#"
declare module 'node:assert/strict' {
  interface Assert {
    (value: unknown, message?: string): asserts value;
    deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
    throws(fn: () => unknown, expected?: ((err: unknown) => boolean) | (abstract new (...args: never[]) => Error), message?: string): void;
  }
  const assert: Assert;
  export default assert;
}
"#;

/// The Rust edge spec (`[[bool; 33]; 2]` and `[Phase; 2]` in a POD struct, among every other codec
/// shape) compiles as C++20, type-checks as Swift 6, and passes strict `dart analyze` and `tsc`.
#[test]
fn edge_spec_with_nested_pod_arrays_compiles_in_cpp20_swift_dart_and_ts() {
    let dir = temp_test_dir("edge");
    generate(EDGE_SPEC, &[HybridTarget::Cpp, HybridTarget::Swift, HybridTarget::TypeScript], &dir);
    analyze_dart(EDGE_SPEC, &dir.join("dart"), "nve_edge");
    tsc_check(&dir);
    let abi = std::fs::read_to_string(dir.join("ShadowHybridAbi.generated.h")).expect("read C header");
    assert!(abi.contains("bool grid[2][33];"), "nested C array dimensions, outermost first");
    if common::require_tool("clang++") {
        common::write_generated_c_abi_headers(&dir);
        let check = dir.join("check.cpp");
        std::fs::write(&check, "#include \"ShadowHybridSpec.generated.hpp\"\nint main() { return 0; }\n")
            .expect("write C++ check");
        run_ok(
            Command::new("clang++")
                .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
                .arg(format!("-I{}", dir.display()))
                .arg(&check),
            "clang++ edge spec",
        );
    }
    if common::require_tool("swiftc") {
        let host = build_host_swift(&dir.join("swift"), &[]);
        typecheck_generated(
            &host,
            &dir.join("ShadowHybridAbi.generated.h"),
            &dir.join("ShadowHybridSpec.generated.swift"),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A struct field may hold an opaque `*const [T; N]`: no C form, so the struct stays C++-only, and
/// static C functions take it as an opaque handle (a file-scope incomplete typedef). The C header
/// passes C11 and C++20 `-Werror`, and the Dart and TypeScript bindings pass their strict checkers.
#[test]
fn opaque_pointers_to_fixed_arrays_keep_their_struct_out_of_the_c_abi() {
    let src = "#[nve_hybrid_struct]\npub struct Holder { pub values: *const [u32; 4], pub n: u32 }\n\
               #[nve_hybrid_c_fn]\npub extern \"C\" fn nve_holder_len(holder: *const Holder) -> u32;\n\
               #[nve_hybrid_c_fn]\npub extern \"C\" fn nve_holder_take(holder: *mut Holder) -> *mut Holder;\n\
               #[nve_hybrid_c_fn]\npub extern \"C\" fn nve_holder_pick(index: Option<u32>) -> u32;\n";
    let dir = temp_test_dir("array_ptr");
    let targets = [HybridTarget::Cpp, HybridTarget::Swift, HybridTarget::TypeScript, HybridTarget::Rust];
    generate(src, &targets, &dir);
    let abi = std::fs::read_to_string(dir.join("HolderHybridAbi.generated.h")).expect("read C header");
    assert!(abi.contains("typedef struct NveHybridHolder NveHybridHolder;"), "an opaque handle, not a C struct");
    assert!(!abi.contains("typedef struct NveHybridHolder {"), "no C struct for a field without a C form");
    assert!(abi.contains("NVE_API NveHybridHolder* nve_holder_take(NveHybridHolder* holder);"));
    assert!(abi.contains("typedef struct NveHybridOptU32 {"), "C function options are C types");
    common::write_generated_c_abi_headers(&dir);
    let c_check = dir.join("check.c");
    std::fs::write(&c_check, "#include \"HolderHybridAbi.generated.h\"\nint main(void) { return 0; }\n")
        .expect("write C check");
    common::check_c11_syntax(&c_check, &[&dir], &dir);
    if common::require_tool("clang++") {
        let check = dir.join("check.cpp");
        std::fs::write(&check, "#include \"HolderHybridSpec.generated.hpp\"\nint main() { return 0; }\n")
            .expect("write C++ check");
        run_ok(
            Command::new("clang++")
                .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
                .arg(format!("-I{}", dir.display()))
                .arg(&check),
            "clang++ opaque array pointer",
        );
    }
    let spec = parse_hybrid_rust_spec(src).expect("parse opaque-pointer spec");
    for target in [HybridTarget::Dart, HybridTarget::Rust] {
        let out = generate_hybrid_target(&spec, target).expect("generate");
        assert!(
            out.files.iter().all(|f| !f.content.contains("NveHybridOptU32")),
            "{target:?} binds no C function, so it declares none of their options"
        );
    }
    analyze_dart(src, &dir.join("dart"), "nve_holder");
    tsc_check(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Generates `spec_src` as Dart into the package `pkg` and runs strict `dart analyze` on it.
fn analyze_dart(spec_src: &str, pkg: &Path, name: &str) {
    if common::require_tool("dart") {
        generate(spec_src, &[HybridTarget::Dart], &pkg.join("lib"));
        common::dart::setup_offline_dart_pkg(pkg, name);
        common::dart::analyze(pkg);
    }
}

/// Strict `tsc --noEmit` over the generated `.ts` files in `dir`.
fn tsc_check(dir: &Path) {
    if !common::require_tool("node") {
        return;
    }
    common::ts::prepare_offline_host_ts_pkg(dir);
    std::fs::write(dir.join("package.json"), r#"{"type": "module"}"#).expect("write package.json");
    let tsconfig = dir.join("tsconfig.json");
    std::fs::write(&tsconfig, common::ts::STRICT_TSCONFIG).expect("write tsconfig.json");
    common::ts::run_tsc_no_emit(&tsconfig);
}

/// Kotlin / JNI has no fixed-array form: the target rejects `[T; N]` instead of emitting a lossy codec.
#[test]
fn kotlin_rejects_fixed_arrays() {
    let spec = parse_hybrid_rust_spec(SPEC).expect("parse fixed-array spec");
    let err = generate_hybrid_target(&spec, HybridTarget::Kotlin)
        .expect_err("Kotlin must reject `[T; N]`")
        .to_string();
    assert!(err.contains("Kotlin / JNI cannot carry"), "unexpected Kotlin error: {err}");
}

/// `[T; 0]` and arrays of more than 4,096 elements (all dimensions multiplied) fail validation on
/// every target.
#[test]
fn fixed_arrays_hold_1_to_4096_elements() {
    for (ty, ok) in [
        ("[u8; 0]", false),
        ("[u8; 1]", true),
        ("[u8; 4096]", true),
        ("[u8; 4097]", false),
        ("[[u8; 64]; 64]", true),
        ("[[u8; 65]; 65]", false),
        ("[[u64; 65536]; 65536]", false),
        ("[u8; 4294967296]", false),
    ] {
        let src = format!(
            "#[nve_hybrid_module(name = \"Edge\")]\npub trait EdgeSpec {{\n    fn take(&mut self, a: {ty});\n}}\n"
        );
        let result = parse_hybrid_rust_spec(&src).and_then(|spec| spec.validate());
        match result {
            Ok(()) => assert!(ok, "{ty} must be rejected"),
            Err(err) => {
                let err = err.to_string();
                assert!(!ok, "{ty}: {err}");
                assert!(err.contains("at least one element") || err.contains("more than 4096 elements"), "{ty}: {err}");
            }
        }
    }
}
