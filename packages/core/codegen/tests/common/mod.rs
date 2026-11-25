//! Shared compiler detection and C11 / C++20 verification helpers for integration tests.
//!
//! Uses `clang`/`clang++` when available (macOS, Linux, LLVM Windows), falls back to MSVC `cl`
//! on Windows hosts where `clang`/`clang++` are absent, and skips gracefully only if neither
//! toolchain is installed.

#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

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
        .unwrap_or_else(|e| panic!("execute {}: {e}", cpp_exe.display()));
    assert!(
        run.status.success(),
        "C++20 execution failed ({}):\nstdout:\n{}\nstderr:\n{}",
        cpp_exe.display(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    true
}
