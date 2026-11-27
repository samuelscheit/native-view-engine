//! Real `rustc` builds of generated Rust against this workspace's `nve_abi` rlib.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Locates `libnve_abi-*.rlib` and all Cargo dependency search directories across both classic
/// `target/<profile>/deps/` and Cargo 1.100+ per-unit `target/<profile>/build/<pkg>/<hash>/out/`
/// layouts.
pub fn locate_nve_abi_and_dep_dirs() -> (PathBuf, Vec<PathBuf>) {
    let current_exe = std::env::current_exe().expect("current_exe");
    let exe_dir = current_exe.parent().expect("test executable parent dir");

    let find_nve_abi_in = |dir: &Path| -> Option<PathBuf> {
        std::fs::read_dir(dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libnve_abi-") && n.ends_with(".rlib"))
            })
    };

    let rlib_has_wire_feature = |path: &Path| -> bool {
        // `nve_hybrid_free_bytes` is a `#[cfg(feature = "wire")]` `extern "C"` symbol in
        // `nve_abi::hybrid`, absent from `--no-default-features` and pre-1B-1e rlibs.
        const NEEDLE: &[u8] = b"nve_hybrid_free_bytes";
        std::fs::read(path)
            .map(|bytes| bytes.windows(NEEDLE.len()).any(|w| w == NEEDLE))
            .unwrap_or(false)
    };
    let find_best_nve_abi_in = |dir: &Path| -> Option<PathBuf> {
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
        matches
            .pop()
            .and_then(|(has_wire, _, path)| has_wire.then_some(path))
    };

    // 1. Classic Cargo layout (`target/<profile>/deps/`).
    if let Some(rlib) = find_best_nve_abi_in(exe_dir) {
        return (rlib, vec![exe_dir.to_path_buf()]);
    }

    // 2. Cargo 1.100+ per-unit build layout (`target/<profile>/build/<pkg>/<hash>/out/`).
    let build_root = current_exe
        .ancestors()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("build"))
        .expect("expected Cargo deps/ or build/ ancestor directory for test executable");

    let mut nve_abi_candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
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
                if is_nve_abi && let Some(rlib) = find_nve_abi_in(&out_dir) {
                    let mtime = std::fs::metadata(&rlib)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    nve_abi_candidates.push((mtime, rlib));
                }
                let has_rlib = std::fs::read_dir(&out_dir).ok().is_some_and(|it| {
                    it.filter_map(|e| e.ok()).any(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| matches!(ext, "rlib" | "dll" | "so" | "dylib"))
                    })
                });
                if has_rlib {
                    dep_dirs.push(out_dir);
                }
            }
        }
    }

    nve_abi_candidates.sort_by_key(|(mtime, path)| (rlib_has_wire_feature(path), *mtime));
    let nve_abi_rlib = nve_abi_candidates
        .pop()
        .map(|(_, path)| path)
        .filter(|path| rlib_has_wire_feature(path))
        .expect("wire-enabled libnve_abi-*.rlib must exist in target deps or build dir");
    (nve_abi_rlib, dep_dirs)
}

pub fn add_dep_search_dirs(cmd: &mut Command, temp_dir: &Path, dep_dirs: &[PathBuf]) {
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
