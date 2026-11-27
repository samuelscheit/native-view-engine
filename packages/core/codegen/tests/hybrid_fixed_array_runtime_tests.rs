//! `HybridType::FixedArray` (`[T; N]`) codec parity in compiled C++20 and Swift: the generated
//! `NveBinValue` codecs round-trip `[u32; 4]`, `[PodStruct; 2]`, and `Option<[u8; 3]>` inside a
//! non-POD struct losslessly, and reject every other element count (Swift also on encode), like
//! the Rust, Dart, and TypeScript targets.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::swift::{SWIFT_STRICT, build_host_swift, hybrid_abi_module_dir, typecheck_generated};
use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const SPEC: &str = include_str!("fixtures/fixed_array/fixed_array_hybrid.nve.rs");
const CPP_DRIVER: &str = include_str!("fixtures/fixed_array/driver.cpp");
const SWIFT_MAIN: &str = include_str!("fixtures/fixed_array/main.swift");

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_fixed_array_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn generate(target: HybridTarget, dir: &std::path::Path) {
    let spec = parse_hybrid_rust_spec(SPEC).expect("parse fixed-array spec");
    generate_hybrid_target(&spec, target)
        .expect("generate hybrid target")
        .write_to_dir(dir)
        .expect("write generated files");
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

#[test]
fn cpp20_fixed_arrays_round_trip_and_reject_wrong_lengths() {
    if !common::require_tool("clang++") {
        return;
    }
    let dir = temp_test_dir("cpp");
    common::write_generated_c_abi_headers(&dir);
    generate(HybridTarget::Cpp, &dir);
    let header = std::fs::read_to_string(dir.join("FixedCodecHybridSpec.generated.hpp"))
        .expect("read generated C++ header");
    assert!(header.contains("const std::array<uint32_t, 4>& coords"), "arrays pass by const&");
    assert!(header.contains("begin_array_exact(4u)"), "decode checks the exact length");

    let driver = dir.join("driver.cpp");
    std::fs::write(&driver, CPP_DRIVER).expect("write C++ driver");
    let exe = dir.join(format!("driver{}", std::env::consts::EXE_SUFFIX));
    run_ok(
        Command::new("clang++")
            .args(["-std=c++20", "-Wall", "-Wextra", "-Werror", "-fsanitize=address,undefined"])
            .arg(format!("-I{}", dir.display()))
            .arg(&driver)
            .arg("-o")
            .arg(&exe),
        "clang++ fixed-array driver",
    );
    assert_eq!(run_ok(&mut Command::new(&exe), "C++ fixed-array driver"), "cpp fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn swift_fixed_arrays_round_trip_and_reject_wrong_lengths() {
    if !common::require_tool("swiftc") {
        return;
    }
    let dir = temp_test_dir("swift");
    let host = build_host_swift(&dir, &[]);
    generate(HybridTarget::Cpp, &dir);
    generate(HybridTarget::Swift, &dir);
    let abi_header = dir.join("FixedCodecHybridAbi.generated.h");
    let swift_file = dir.join("FixedCodecHybridSpec.generated.swift");
    typecheck_generated(&host, &abi_header, &swift_file);

    let main = dir.join("main.swift");
    std::fs::write(&main, SWIFT_MAIN).expect("write Swift driver");
    let exe = dir.join(format!("fixed_array{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new("swiftc");
    cmd.args(SWIFT_STRICT).arg("-I").arg(hybrid_abi_module_dir(&host, &abi_header));
    host.search_args(&mut cmd);
    cmd.arg(&main).arg(&swift_file);
    host.lib_args(&mut cmd);
    run_ok(cmd.arg("-o").arg(&exe), "swiftc fixed-array driver");
    assert_eq!(run_ok(&mut Command::new(&exe), "Swift fixed-array driver"), "swift fixed array ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kotlin / JNI has no fixed-array form: the target rejects `[T; N]` instead of emitting a lossy codec.
#[test]
fn kotlin_rejects_fixed_arrays() {
    let spec = parse_hybrid_rust_spec(SPEC).expect("parse fixed-array spec");
    let err = generate_hybrid_target(&spec, HybridTarget::Kotlin)
        .expect_err("Kotlin must reject `[T; N]`")
        .to_string();
    assert!(
        err.contains("Kotlin / JNI cannot carry") && err.contains("[u32;4]"),
        "unexpected Kotlin error: {err}"
    );
}

/// The wire count is a `u32`: `[T; 0]` and `[T; > u32::MAX]` fail validation on every target.
#[test]
fn fixed_array_lengths_outside_u32_wire_range_are_rejected() {
    for (len, ok) in [("0", false), ("1", true), ("4294967295", true), ("4294967296", false)] {
        let src = format!(
            "#[nve_hybrid_module(name = \"Edge\")]\npub trait EdgeSpec {{\n    fn take(&mut self, a: [u8; {len}]);\n}}\n"
        );
        let result = parse_hybrid_rust_spec(&src).and_then(|spec| spec.validate());
        match (ok, result) {
            (true, Ok(())) => {}
            (false, Err(err)) => assert!(
                err.to_string().contains("1..=u32::MAX"),
                "[u8; {len}]: unexpected error {err}"
            ),
            (_, result) => panic!("[u8; {len}]: expected ok={ok}, got {result:?}"),
        }
    }
}
