//! Shared compiler detection and C11 / C++20 verification helpers for integration tests.
//!
//! Uses `clang`/`clang++` when available (macOS, Linux, LLVM Windows), falls back to MSVC `cl`
//! on Windows hosts where `clang`/`clang++` are absent, and skips gracefully only if neither
//! toolchain is installed.

#![allow(dead_code)]

pub mod rustc;
pub mod specs;
pub mod swift;
pub mod dart;
pub mod ts;

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create copy target dir");
    for entry in std::fs::read_dir(from)
        .expect("read copy source dir")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else if !path.to_string_lossy().contains(".generated.") {
            std::fs::copy(&path, to.join(entry.file_name())).expect("copy file");
        }
    }
}

/// Recreates the `packages/core/host-cxx/include` tree under `dir`: the static `nve/host/*.h`
/// headers plus freshly generated `nve/generated/nve_{abi,tables}.generated.h` (17 core tables)
/// and `hash_vectors.generated.h`, so compiler tests (`-I dir`) never depend on generated files
/// existing in the repo.
pub fn write_generated_c_abi_headers(dir: &Path) {
    copy_tree(
        &workspace_root().join("packages/core/host-cxx/include/nve/host"),
        &dir.join("nve/host"),
    );
    let files = nve_codegen::generate_all_abi_files(nve_tables::tables::CORE_TABLE_DESCS)
        .expect("generate host ABI files");
    for file in files {
        let rel = file.relative_path.as_str();
        if let Some(rel) = rel
            .strip_prefix("packages/core/host-cxx/include/")
            .or_else(|| rel.strip_prefix("packages/core/host-cxx/tests/"))
        {
            let out = dir.join(rel);
            std::fs::create_dir_all(out.parent().expect("header has a parent dir"))
                .expect("create header dir");
            std::fs::write(out, &file.content).expect("write generated header");
        }
    }
}

/// Materializes the `nve_host` Dart package (static `lib/` plus freshly generated Dart) under
/// `temp_dir/nve_host_pkg`, returning its root.
pub fn prepare_offline_nve_host_pkg(temp_dir: &Path) -> PathBuf {
    let pkg_root = temp_dir.join("nve_host_pkg");
    let src = workspace_root().join("packages/core/host-dart");
    copy_tree(&src.join("lib"), &pkg_root.join("lib"));
    std::fs::copy(src.join("pubspec.yaml"), pkg_root.join("pubspec.yaml"))
        .expect("copy nve_host pubspec");
    let files = nve_codegen::generate_all_abi_files(nve_tables::tables::CORE_TABLE_DESCS)
        .expect("generate host ABI files");
    for file in files {
        if let Some(rel) = file
            .relative_path
            .strip_prefix("packages/core/host-dart/lib/")
        {
            let out = pkg_root.join("lib").join(rel);
            std::fs::create_dir_all(out.parent().expect("dart file has a parent dir"))
                .expect("create dart dir");
            std::fs::write(out, &file.content).expect("write generated dart file");
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
                file.content.contains("package dev.nve.host.generated;"),
                "generated Java file must declare `package dev.nve.host.generated;`: {}",
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

/// Whether `tool` runs. A missing tool returns `false` (the caller skips), unless the test run
/// requires it: `NVE_REQUIRE_<TOOL>` (`swiftc` -> `NVE_REQUIRE_SWIFTC`), and for `clang` /
/// `clang++` also `NVE_REQUIRE_CLANG` or `CI` (every CI image has clang). Then the test fails.
pub fn require_tool(tool: &str) -> bool {
    if tool_available(tool) {
        return true;
    }
    let var = format!(
        "NVE_REQUIRE_{}",
        tool.to_uppercase().replace('+', "X").replace('-', "_")
    );
    let clang = tool.starts_with("clang")
        && (std::env::var_os("NVE_REQUIRE_CLANG").is_some() || std::env::var_os("CI").is_some());
    assert!(
        std::env::var_os(&var).is_none() && !clang,
        "`{tool}` is required ({var} / CI) but was not found"
    );
    eprintln!("SKIP: `{tool}` not found");
    false
}

/// Whether `tool` answers its version probe (`kotlinc` only knows `-version`; `--version` exits 1).
pub fn tool_available(tool: &str) -> bool {
    let probe = if tool == "kotlinc" { "-version" } else { "--version" };
    Command::new(tool)
        .arg(probe)
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

    assert!(
        !require_tool("clang"),
        "unreachable: clang probe already returned false"
    );
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
