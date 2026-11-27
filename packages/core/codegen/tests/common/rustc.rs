//! Real `rustc` builds of generated Rust against this workspace's `nve_abi` rlib.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// The `nve_abi` rlib and dependency search directories of the running test binary's profile.
pub struct Rustc {
    nve_abi_rlib: PathBuf,
    dep_dirs: Vec<PathBuf>,
}

impl Rustc {
    /// Locates `libnve_abi-*.rlib` across both the classic `target/<profile>/deps/` and the Cargo
    /// 1.100+ per-unit `target/<profile>/build/<pkg>/<hash>/out/` layouts.
    pub fn locate() -> Self {
        // Linking `nve_abi` into the test binary makes Cargo build its rlib.
        std::hint::black_box(nve_abi::NveValue::null());
        let (nve_abi_rlib, dep_dirs) = locate_nve_abi_and_dep_dirs();
        Self {
            nve_abi_rlib,
            dep_dirs,
        }
    }

    /// `rustc --edition=2024 -D warnings <args> --extern nve_abi=… <externs> <src> -o <out>`,
    /// failing the test with rustc's diagnostics.
    pub fn build(
        &self,
        work_dir: &Path,
        args: &[&str],
        externs: &[(&str, &Path)],
        src: &Path,
        out: &Path,
    ) {
        let mut cmd = Command::new("rustc");
        cmd.args(["--edition=2024", "-D", "warnings"]).args(args);
        add_dep_search_dirs(&mut cmd, work_dir, &self.dep_dirs);
        cmd.arg("--extern")
            .arg(format!("nve_abi={}", self.nve_abi_rlib.display()));
        for (name, rlib) in externs {
            cmd.arg("--extern")
                .arg(format!("{name}={}", rlib.display()));
        }
        let output = cmd.arg(src).arg("-o").arg(out).output().expect("run rustc");
        assert!(
            output.status.success(),
            "rustc {} failed:\nstdout:\n{}\nstderr:\n{}",
            src.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Builds `src` as the rlib `lib<crate_name>.rlib` in `work_dir` and returns its path.
    pub fn build_lib(&self, work_dir: &Path, crate_name: &str, src: &Path) -> PathBuf {
        let rlib = work_dir.join(format!("lib{crate_name}.rlib"));
        let name_arg = format!("--crate-name={crate_name}");
        self.build(work_dir, &["--crate-type=lib", &name_arg], &[], src, &rlib);
        rlib
    }
}

/// One `libnve_abi-*.rlib`: whether it was built with the `wire` feature, its mtime, its path.
type NveAbiRlib = (bool, SystemTime, PathBuf);

/// Every `libnve_abi-*.rlib` in `dir`, each read once to detect the `wire` feature.
fn nve_abi_rlibs_in(dir: &Path) -> Vec<NveAbiRlib> {
    // `nve_hybrid_free_bytes` is a `#[cfg(feature = "wire")]` `extern "C"` symbol in
    // `nve_abi::hybrid`, absent from `--no-default-features` and pre-1B-1e rlibs.
    const NEEDLE: &[u8] = b"nve_hybrid_free_bytes";
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
        })
        .map(|p| {
            let has_wire = std::fs::read(&p)
                .is_ok_and(|bytes| bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE));
            let mtime = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (has_wire, mtime, p)
        })
        .collect()
}

/// The newest wire-enabled rlib among `candidates`, found under `searched`.
fn newest_wire_rlib(candidates: Vec<NveAbiRlib>, searched: &Path) -> PathBuf {
    let newest = candidates
        .into_iter()
        .filter(|(has_wire, _, _)| *has_wire)
        .max_by_key(|(_, mtime, _)| *mtime)
        .map(|(_, _, path)| path);
    assert!(
        newest.is_some(),
        "wire-enabled libnve_abi-*.rlib must exist in {}",
        searched.display()
    );
    newest.expect("asserted above")
}

/// Locates `libnve_abi-*.rlib` and all Cargo dependency search directories across both classic
/// `target/<profile>/deps/` and Cargo 1.100+ per-unit `target/<profile>/build/<pkg>/<hash>/out/`
/// layouts.
fn locate_nve_abi_and_dep_dirs() -> (PathBuf, Vec<PathBuf>) {
    let current_exe = std::env::current_exe().expect("current_exe");
    let exe_dir = current_exe.parent().expect("test executable parent dir");

    // 1. Classic Cargo layout (`target/<profile>/deps/`): any `nve_abi` rlib here decides.
    let classic = nve_abi_rlibs_in(exe_dir);
    if !classic.is_empty() {
        return (newest_wire_rlib(classic, exe_dir), vec![exe_dir.to_path_buf()]);
    }

    // 2. Cargo 1.100+ per-unit build layout (`target/<profile>/build/<pkg>/<hash>/out/`).
    let build_root = current_exe
        .ancestors()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("build"))
        .expect("expected Cargo deps/ or build/ ancestor directory for test executable");

    let mut candidates = Vec::new();
    let mut dep_dirs = Vec::new();
    for pkg_entry in std::fs::read_dir(build_root).into_iter().flatten().flatten() {
        let pkg_path = pkg_entry.path();
        let is_nve_abi = pkg_entry.file_name() == "nve-abi";
        for hash_entry in std::fs::read_dir(&pkg_path).into_iter().flatten().flatten() {
            let out_dir = hash_entry.path().join("out");
            if !out_dir.is_dir() {
                continue;
            }
            if is_nve_abi {
                candidates.extend(nve_abi_rlibs_in(&out_dir));
            }
            let has_lib = std::fs::read_dir(&out_dir).into_iter().flatten().flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "rlib" | "dll" | "so" | "dylib"))
            });
            if has_lib {
                dep_dirs.push(out_dir);
            }
        }
    }
    (newest_wire_rlib(candidates, build_root), dep_dirs)
}

fn add_dep_search_dirs(cmd: &mut Command, temp_dir: &Path, dep_dirs: &[PathBuf]) {
    if dep_dirs.len() <= 8 {
        for dir in dep_dirs {
            cmd.arg("-L").arg(format!("dependency={}", dir.display()));
        }
        return;
    }
    // Use a `rustc` `@file` response file so Cargo 1.100+'s per-unit `build/<pkg>/<hash>/out`
    // directories never exceed Windows' 32 KiB `CreateProcessW` command-line limit (OS error 206).
    let rsp_path = temp_dir.join("rustc_dep_dirs.rsp");
    let mut contents = String::new();
    for dir in dep_dirs {
        contents.push_str("-L\ndependency=");
        contents.push_str(&dir.to_string_lossy());
        contents.push('\n');
    }
    std::fs::write(&rsp_path, contents).expect("write rustc dep dirs response file");
    cmd.arg(format!("@{}", rsp_path.display()));
}
