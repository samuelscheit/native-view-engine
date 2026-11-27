//! Generated hybrid Swift over the static `packages/core/host-swift` runtime: shape of the emitted
//! code, and a live Swift <-> Rust end-to-end run (generated Swift clients linked against `NveHost`
//! / `NveHostAbi` and a Rust staticlib built from the generated Rust vtables).

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::rustc::{add_dep_search_dirs, locate_nve_abi_and_dep_dirs};
use common::swift::{SWIFT_STRICT, build_host_swift, typecheck_generated};
use nve_codegen::{HybridTarget, generate_hybrid_target, parse_hybrid_rust_spec};

const REALISTIC_HYBRID_SPEC: &str = include_str!("fixtures/realistic_hybrid.nve.rs");
const EDGE_HYBRID_SPEC: &str = include_str!("fixtures/swift_edge_hybrid.nve.rs");
const PROVIDER_RS: &str = include_str!("fixtures/swift_e2e_provider.rs");
const MAIN_SWIFT: &str = include_str!("fixtures/swift_e2e_main.swift");

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nve_hybrid_swift_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn realistic_swift() -> String {
    let spec = parse_hybrid_rust_spec(REALISTIC_HYBRID_SPEC).expect("parse realistic spec");
    let out = generate_hybrid_target(&spec, HybridTarget::Swift).expect("generate Swift");
    out.files[0].content.clone()
}

#[test]
fn generated_swift_uses_the_static_host_runtime() {
    let src = realistic_swift();
    for import in [
        "#if canImport(CryptoHybridAbi)\nimport CryptoHybridAbi\n#endif",
        "import CNveAbi",
        "import NveHost",
        "import NveHostAbi",
    ] {
        assert!(src.contains(import), "missing `{import}`");
    }
    // Per-file constants are namespaced, so several generated files share one module.
    assert!(src.contains("public enum CryptoHybridFile {"));
    assert!(src.contains("expected: CryptoHybridFile.abiVersion"));
    assert!(!src.contains("public let "), "no module-level globals");
    // No codec, error type, dynamic value, or helper is copied into generated code any more.
    for copied in [
        "NveHybridBinWriter",
        "NveHybridBinReader",
        "NveHybridError",
        "NveHybridDynamicValue",
        "unwrapOrThrow",
        "extension Optional",
        "NveSwiftUI",
        "NveDirect",
    ] {
        assert!(!src.contains(copied), "generated Swift still contains `{copied}`");
    }
    assert!(src.contains("public enum HashAlgorithm: UInt32, HybridEnum {"));
    assert!(src.contains("public func encodeBin(into w: inout BinWriter) throws(BinError) {"));
    assert!(src.contains("func inspectDynamic(meta: DynamicValue) throws -> DynamicValue"));

    // `NveValue` props are a lossless tree; `Struct` / `Vec` props one `Bytes` payload; scalars,
    // strings, and bytes their own tags; absent options `Null`.
    assert!(src.contains("let _nveProp8 = try _nveTree.value(props.metadata)"));
    assert!(src.contains("let _nveProp5 = try props.focusPoint.map { _nveSome in"));
    assert!(src.contains("let _nveProp7 =\n            try _nveTree.encoded { _nveW in"));
    assert!(src.contains("let _nveProp6 = _nveTree.bytes(props.drmKey)"));
    assert!(src.contains(
        "let _nveProp4 = props.posterUrl.map { _nveSome in _nveTree.string(_nveSome) } ?? _nveTree.null()"
    ));
    assert!(src.contains("let _nveStatus = withExtendedLifetime(_nveTree) {"));
    // The `NveValue` argument is a tree kept alive across the call.
    assert!(src.contains("let _nveArg0 = try _nveTree.value(meta)"));
    assert!(src.contains("return try withExtendedLifetime(_nveTree) {"));
}

/// The specs linked into one Swift module, one Rust staticlib each crate: `(spec, stem, crate)`.
const E2E_SPECS: [(&str, &str, &str); 2] = [
    (REALISTIC_HYBRID_SPEC, "Crypto", "crypto_hybrid"),
    (EDGE_HYBRID_SPEC, "Edge", "edge_hybrid"),
];

/// Builds the generated Rust crates and `PROVIDER_RS` into a static library exporting
/// `nve_get_interface`.
fn build_rust_provider(dir: &Path) -> PathBuf {
    let _touch = nve_abi::NveValue::null(); // links `nve_abi` into this test binary's deps
    let (nve_abi_rlib, dep_dirs) = locate_nve_abi_and_dep_dirs();
    let rustc = |args: &[&str], extern_crates: &[(&str, PathBuf)], src: &Path, out: &Path| {
        let mut cmd = Command::new("rustc");
        cmd.args(["--edition=2024", "-D", "warnings"]).args(args);
        add_dep_search_dirs(&mut cmd, dir, &dep_dirs);
        for (name, path) in extern_crates {
            cmd.arg("--extern").arg(format!("{name}={}", path.display()));
        }
        let res = cmd.arg(src).arg("-o").arg(out).output().expect("run rustc");
        assert!(
            res.status.success(),
            "rustc {} failed:\n{}",
            src.display(),
            String::from_utf8_lossy(&res.stderr)
        );
    };
    let mut externs = vec![("nve_abi", nve_abi_rlib.clone())];
    for (_, _, crate_name) in E2E_SPECS {
        let rlib = dir.join(format!("lib{crate_name}.rlib"));
        rustc(
            &["--crate-type=lib", &format!("--crate-name={crate_name}")],
            &[("nve_abi", nve_abi_rlib.clone())],
            &dir.join(format!("{crate_name}.generated.rs")),
            &rlib,
        );
        externs.push((crate_name, rlib));
    }
    let provider = dir.join("provider.rs");
    std::fs::write(&provider, PROVIDER_RS).expect("write provider");
    let staticlib = dir.join("liblive_provider.a");
    rustc(&["--crate-type=staticlib"], &externs, &provider, &staticlib);
    staticlib
}

#[test]
fn generated_swift_calls_live_rust_provider_end_to_end() {
    if !common::require_tool("swiftc") {
        return;
    }
    let dir = temp_test_dir("e2e");
    // Optimized, so ARC may release a value tree right after its last use; AddressSanitizer then
    // catches any read through a pointer into a released `NveValueTree` or pinned buffer.
    let flags = ["-O", "-sanitize=address"];
    let host = build_host_swift(&dir, &flags);

    let mut swift_files = Vec::new();
    let mut bridge = String::new();
    for (src, stem, _) in E2E_SPECS {
        let spec = parse_hybrid_rust_spec(src).expect("parse hybrid spec");
        for target in [HybridTarget::Cpp, HybridTarget::Swift, HybridTarget::Rust] {
            generate_hybrid_target(&spec, target)
                .expect("generate hybrid target")
                .write_to_dir(&dir)
                .expect("write generated files");
        }
        let header = dir.join(format!("{stem}HybridAbi.generated.h"));
        let swift_file = dir.join(format!("{stem}HybridSpec.generated.swift"));
        typecheck_generated(&host, &header, &swift_file);
        bridge.push_str(&format!("#include \"{stem}HybridAbi.generated.h\"\n"));
        swift_files.push(swift_file);
    }
    let staticlib = build_rust_provider(&dir);
    bridge.push_str(
        "const void* nve_get_interface(uint32_t interface_id);\n\
         size_t nve_test_prop_shape(uint32_t index, uint8_t* out, size_t cap);\n",
    );
    let bridge_h = dir.join("bridge.h");
    std::fs::write(&bridge_h, bridge).expect("write bridging header");
    let main = dir.join("main.swift");
    std::fs::write(&main, MAIN_SWIFT).expect("write main.swift");

    // Inputs before the libraries they use (dependents first), as static linkers resolve in order.
    let exe = dir.join("swift_e2e");
    let mut cmd = Command::new("swiftc");
    cmd.args(SWIFT_STRICT)
        .args(flags)
        .arg("-import-objc-header")
        .arg(&bridge_h);
    host.search_args(&mut cmd);
    cmd.arg(&main).args(&swift_files).arg(&staticlib);
    host.lib_args(&mut cmd);
    let build = cmd
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("link Swift against the Rust staticlib");
    assert!(
        build.status.success(),
        "Swift <-> Rust link failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&exe).output().expect("run the Swift e2e binary");
    assert!(
        run.status.success(),
        "Swift <-> Rust run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "swift e2e ok");
    let _ = std::fs::remove_dir_all(&dir);
}
