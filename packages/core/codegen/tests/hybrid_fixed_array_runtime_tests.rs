//! `HybridType::FixedArray` (`[T; N]`) in compiled C++20 and Swift against a live Rust provider
//! built from the generated Rust: `NveBinValue` codecs (lossless, every other element count
//! rejected; Swift also on encode), POD C arrays (nested too) through `to_abi` / `from_abi`,
//! `Option<[T; N]>` and `Result<Option<[T; N]>, String>` returns, and `[T; N]` view props.

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

/// Generates C++ / Swift / Rust for `SPEC` into `dir` and builds the Rust provider staticlib.
fn build_provider(dir: &Path) -> PathBuf {
    generate(SPEC, &[HybridTarget::Cpp, HybridTarget::Swift, HybridTarget::Rust], dir);
    let rustc = Rustc::locate();
    let src = dir.join("fixed_codec_hybrid.generated.rs");
    let rlib = rustc.build_lib(dir, "fixed_codec_hybrid", &src);
    let provider = dir.join("provider.rs");
    std::fs::write(&provider, PROVIDER_RS).expect("write provider");
    let staticlib = dir.join("libfixed_provider.a");
    let externs = [("fixed_codec_hybrid", rlib.as_path())];
    rustc.build(dir, &["--crate-type=staticlib"], &externs, &provider, &staticlib);
    staticlib
}

#[test]
fn cpp20_fixed_arrays_cross_the_codec_c_abi_and_live_provider() {
    if !common::require_tool("clang++") {
        return;
    }
    let dir = temp_test_dir("cpp");
    common::write_generated_c_abi_headers(&dir);
    let staticlib = build_provider(&dir);
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
    let flags = ["-sanitize=address"];
    let host = build_host_swift(&dir, &flags);
    let staticlib = build_provider(&dir);
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

/// The Rust edge spec (`[[bool; 33]; 2]` and `[Phase; 2]` in a POD struct, among every other codec
/// shape) compiles as C++20 and type-checks as Swift 6.
#[test]
fn edge_spec_with_nested_pod_arrays_compiles_in_cpp20_and_swift() {
    let dir = temp_test_dir("edge");
    generate(EDGE_SPEC, &[HybridTarget::Cpp, HybridTarget::Swift], &dir);
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

/// Kotlin / JNI has no fixed-array form: the target rejects `[T; N]` instead of emitting a lossy codec.
#[test]
fn kotlin_rejects_fixed_arrays() {
    let spec = parse_hybrid_rust_spec(SPEC).expect("parse fixed-array spec");
    let err = generate_hybrid_target(&spec, HybridTarget::Kotlin)
        .expect_err("Kotlin must reject `[T; N]`")
        .to_string();
    assert!(err.contains("Kotlin / JNI cannot carry"), "unexpected Kotlin error: {err}");
}

/// `[T; 0]` and `[T; N > MAX_FIXED_ARRAY_LEN]` fail validation on every target.
#[test]
fn fixed_array_lengths_outside_1_to_65536_are_rejected() {
    for (len, ok) in [("0", false), ("1", true), ("65536", true), ("65537", false), ("4294967296", false)] {
        let src = format!(
            "#[nve_hybrid_module(name = \"Edge\")]\npub trait EdgeSpec {{\n    fn take(&mut self, a: [u8; {len}]);\n}}\n"
        );
        let result = parse_hybrid_rust_spec(&src).and_then(|spec| spec.validate());
        match (ok, result) {
            (true, Ok(())) => {}
            (false, Err(err)) => assert!(
                err.to_string().contains("1..=65536"),
                "[u8; {len}]: unexpected error {err}"
            ),
            (_, result) => panic!("[u8; {len}]: expected ok={ok}, got {result:?}"),
        }
    }
}
