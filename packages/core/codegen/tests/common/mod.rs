//! Shared compiler detection and C11 / C++20 verification helpers for integration tests.
//!
//! Uses `clang`/`clang++` when available (macOS, Linux, LLVM Windows), falls back to MSVC `cl`
//! on Windows hosts where `clang`/`clang++` are absent, and skips gracefully only if neither
//! toolchain is installed.

#![allow(dead_code)]

pub mod specs;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Writes freshly generated C11/C++20 Core ABI, Scene ABI, Runtime ABI, and Wire Codec headers
/// (`nve_abi.generated.h`, `nve_scene_abi.generated.h`, `nve_runtime_abi.generated.h`,
/// `nve_wire_codecs.generated.hpp`) directly into `dir` so compiler tests never depend on
/// generated files existing in the repo.
pub fn write_generated_c_abi_headers(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create temp header dir");
    for file in nve_codegen::generate_all_abi_files() {
        if let Some(fname) = file
            .relative_path
            .strip_prefix("cxx/nve-fabric-bridge/include/")
            .or_else(|| {
                file.relative_path
                    .strip_prefix("packages/core/host-cxx/include/nve/host/")
            })
        {
            std::fs::write(dir.join(fname), &file.content)
                .expect("write generated header");
        }
    }
}

/// Materializes a self-contained offline `nve_host` Dart package under `temp_dir/nve_host_pkg`:
/// the hand-written `packages/core/host-dart/lib` sources plus freshly generated `*.generated.dart`
/// bindings, so `dart analyze` succeeds on a fresh checkout with zero generated files on disk.
pub fn prepare_offline_nve_host_pkg(temp_dir: &Path, workspace_root: &Path) -> PathBuf {
    let pkg_root = temp_dir.join("nve_host_pkg");
    let dst_lib = pkg_root.join("lib");
    std::fs::create_dir_all(dst_lib.join("src")).expect("create temp nve_host_pkg/lib/src");
    std::fs::write(
        pkg_root.join("pubspec.yaml"),
        "name: nve_host\nenvironment:\n  sdk: '>=3.4.0 <4.0.0'\n",
    )
    .expect("write temp nve_host pubspec.yaml");

    let src_lib = workspace_root.join("packages/core/host-dart/lib");
    for dir in [PathBuf::new(), PathBuf::from("src")] {
        for entry in std::fs::read_dir(src_lib.join(&dir)).expect("read host-dart lib dir") {
            let path = entry.expect("host-dart lib entry").path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name.ends_with(".dart") && !name.contains(".generated.") {
                std::fs::copy(&path, dst_lib.join(&dir).join(name)).expect("copy host-dart source");
            }
        }
    }

    for file in nve_codegen::generate_all_abi_files() {
        if let Some(rel) = file.relative_path.strip_prefix("packages/core/host-dart/lib/") {
            let out_path = dst_lib.join(rel);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).expect("create dart parent dir");
            }
            std::fs::write(&out_path, &file.content)
                .expect("write generated dart file");
        }
    }

    pkg_root
}

/// Validates generated-file naming invariants (`.generated.` for non-Java, `generated/` package
/// for Java) and checks byte-for-byte parity for any generated file currently materialized on disk.
pub fn assert_generated_file_naming_and_optional_disk_parity(
    files: &[nve_codegen::GeneratedFile],
    repo_root: &Path,
    update_mode: bool,
) {
    for file in files {
        let fname = Path::new(&file.relative_path)
            .file_name()
            .and_then(|n| n.to_str())
            .expect("generated file name is valid UTF-8");
        if file.relative_path.ends_with(".java") {
            assert!(
                file.relative_path.contains("/generated/"),
                "generated Java file must live inside a `generated/` package directory: {}",
                file.relative_path
            );
            assert!(
                file.content.contains("package dev.nve.android.generated;"),
                "generated Java file must declare `package dev.nve.android.generated;`: {}",
                file.relative_path
            );
        } else {
            assert!(
                fname.contains(".generated."),
                "generated non-Java file must contain `.generated.` in filename: {}",
                file.relative_path
            );
        }

        let full_path = repo_root.join(&file.relative_path);
        if update_mode {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dirs");
            }
            std::fs::write(&full_path, &file.content).expect("write generated ABI file");
        }
        if full_path.is_file() {
            let on_disk = std::fs::read_to_string(&full_path)
                .expect("read generated ABI file on disk")
                .replace("\r\n", "\n");
            assert_eq!(
                on_disk, file.content,
                "ABI schema drift detected in {}! Re-run `cargo run --locked -p nve-cli -- codegen abi` to regenerate.",
                file.relative_path
            );
        }
    }
}

pub fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn msvc_cl_available() -> bool {
    cfg!(windows)
        && Command::new("cl")
            .arg("/?")
            .output()
            .is_ok_and(|o| o.status.success())
}

/// Syntax-checks a C11 source file (`clang -std=c11 -Wall -Wextra -Werror -fsyntax-only`, or
/// MSVC `cl /nologo /std:c11 /W4 /WX /Zs`). Returns `true` if a compiler ran, or `false` if
/// skipped because neither `clang` nor `cl` is installed.
pub fn check_c11_syntax(source: &Path, include_dirs: &[&Path], work_dir: &Path) -> bool {
    if tool_available("clang") {
        let mut cmd = Command::new("clang");
        cmd.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"]);
        for inc in include_dirs {
            cmd.arg(format!("-I{}", inc.display()));
        }
        cmd.arg(source);
        let out = cmd.output().expect("run clang -std=c11");
        assert!(
            out.status.success(),
            "C11 compile failed (clang):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return true;
    }

    if msvc_cl_available() {
        let mut cmd = Command::new("cl");
        cmd.current_dir(work_dir)
            .args(["/nologo", "/std:c11", "/W4", "/WX", "/Zs"]);
        for inc in include_dirs {
            cmd.arg(format!("/I{}", inc.display()));
        }
        cmd.arg(source);
        let out = cmd.output().expect("run cl /std:c11");
        assert!(
            out.status.success(),
            "C11 compile failed (MSVC cl):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return true;
    }

    eprintln!(
        "skipping C11 header check for {}: neither clang nor MSVC cl is installed",
        source.display()
    );
    false
}

/// Compiles and executes a C++20 test driver (`clang++ -std=c++20 -Wall -Wextra -Werror`, or
/// MSVC `cl /nologo /std:c++20 /EHsc /W4 /WX`). Returns `true` if compiled and executed, or
/// `false` if skipped because neither `clang++` nor `cl` is installed.
pub fn compile_and_run_cpp20(
    source: &Path,
    exe_stem: &str,
    include_dirs: &[&Path],
    work_dir: &Path,
) -> bool {
    let exe_name = if cfg!(windows) {
        format!("{exe_stem}.exe")
    } else {
        exe_stem.to_owned()
    };
    let cpp_exe = work_dir.join(exe_name);

    if tool_available("clang++") {
        let mut cmd = Command::new("clang++");
        cmd.args(["-std=c++20", "-Wall", "-Wextra", "-Werror"]);
        for inc in include_dirs {
            cmd.arg(format!("-I{}", inc.display()));
        }
        cmd.arg(source).arg("-o").arg(&cpp_exe);
        let out = cmd.output().expect("run clang++ -std=c++20");
        assert!(
            out.status.success(),
            "C++20 compile failed (clang++):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    } else if msvc_cl_available() {
        let mut cmd = Command::new("cl");
        cmd.current_dir(work_dir)
            .args(["/nologo", "/std:c++20", "/EHsc", "/W4", "/WX"]);
        for inc in include_dirs {
            cmd.arg(format!("/I{}", inc.display()));
        }
        cmd.arg(source).arg(format!("/Fe:{}", cpp_exe.display()));
        let out = cmd.output().expect("run cl /std:c++20");
        assert!(
            out.status.success(),
            "C++20 compile failed (MSVC cl):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    } else {
        eprintln!(
            "skipping C++20 compile-and-run for {}: neither clang++ nor MSVC cl is installed",
            source.display()
        );
        return false;
    }

    let run = Command::new(&cpp_exe)
        .output()
        .expect("execute compiled C++ test binary");
    assert!(
        run.status.success(),
        "C++20 execution failed ({}):\nstdout:\n{}\nstderr:\n{}",
        cpp_exe.display(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    true
}
