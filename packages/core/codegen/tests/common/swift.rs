//! Builds the static Swift host runtime (`packages/core/host-swift`: `NveHost`, `NveHostAbi`) as
//! real Swift 6 modules for generated-code compiler tests, from a fresh copy plus freshly generated
//! ABI files, so no test depends on generated files existing in the repo.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{copy_tree, workspace_root, write_generated_c_abi_headers};

/// Search paths of a built host runtime: `include` holds the `CNveAbi` module (generated C ABI) and
/// `modules` the `NveHost` / `NveHostAbi` `.swiftmodule`s and static libraries.
pub struct SwiftHost {
    pub include: PathBuf,
    pub modules: PathBuf,
}

impl SwiftHost {
    /// `-I include -I modules`: enough to `import CNveAbi, NveHost, NveHostAbi`.
    pub fn search_args(&self, cmd: &mut Command) {
        cmd.arg("-I").arg(&self.include).arg("-I").arg(&self.modules);
    }

    /// The static runtime libraries, dependents first; pass them after the inputs that use them.
    pub fn lib_args(&self, cmd: &mut Command) {
        cmd.arg("-L")
            .arg(&self.modules)
            .args(["-lNveHostAbi", "-lNveHost"]);
    }
}

/// Strict flags every Swift compile in these tests uses.
pub const SWIFT_STRICT: [&str; 3] = ["-swift-version", "6", "-warnings-as-errors"];

/// Builds `NveHost` and `NveHostAbi` under `dir`; `extra` is passed to every `swiftc` (e.g.
/// `-sanitize=address`).
pub fn build_host_swift(dir: &Path, extra: &[&str]) -> SwiftHost {
    let include = dir.join("include");
    write_generated_c_abi_headers(&include);
    let host_cxx = workspace_root().join("packages/core/host-cxx/include/module.modulemap");
    std::fs::copy(host_cxx, include.join("module.modulemap")).expect("copy CNveAbi module map");

    let sources = dir.join("host-swift");
    let pkg = workspace_root().join("packages/core/host-swift/Sources");
    copy_tree(&pkg.join("NveHost"), &sources.join("NveHost"));
    copy_tree(&pkg.join("NveHostAbi"), &sources.join("NveHostAbi"));
    let generated = nve_codegen::generate_all_abi_files(nve_components::tables::CORE_TABLE_DESCS)
        .expect("generate host ABI files");
    for file in generated {
        if let Some(rel) = file
            .relative_path
            .strip_prefix("packages/core/host-swift/Sources/")
        {
            std::fs::write(sources.join(rel), &file.content).expect("write generated Swift");
        }
    }

    let modules = dir.join("modules");
    std::fs::create_dir_all(&modules).expect("create Swift module dir");
    let host = SwiftHost { include, modules };
    for module in ["NveHost", "NveHostAbi"] {
        let mut cmd = Command::new("swiftc");
        cmd.args(SWIFT_STRICT)
            .args(extra)
            .args(["-parse-as-library", "-wmo", "-static", "-emit-library", "-module-name"])
            .arg(module)
            .arg("-emit-module-path")
            .arg(host.modules.join(format!("{module}.swiftmodule")))
            .arg("-o")
            .arg(host.modules.join(format!("lib{module}.a")));
        host.search_args(&mut cmd);
        cmd.args(swift_files(&sources.join(module)));
        let out = cmd.output().expect("run swiftc on the Swift host runtime");
        assert!(
            out.status.success(),
            "building {module} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    host
}

fn swift_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read Swift sources")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "swift"))
        .collect();
    files.sort();
    files
}

/// Type-checks generated hybrid Swift against the built runtime, with `abi_header` imported the
/// SwiftPM way: as the C module `<Stem>HybridAbi` (the end-to-end run uses a bridging header).
pub fn typecheck_generated(host: &SwiftHost, abi_header: &Path, swift_file: &Path) {
    let mut cmd = Command::new("swiftc");
    cmd.args(SWIFT_STRICT)
        .args(["-typecheck", "-parse-as-library", "-I"])
        .arg(hybrid_abi_module_dir(host, abi_header));
    host.search_args(&mut cmd);
    let out = cmd
        .arg(swift_file)
        .output()
        .expect("run swiftc -typecheck on generated Swift");
    assert!(
        out.status.success(),
        "generated Swift {} failed swiftc -typecheck:\n{}",
        swift_file.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Writes a module map exposing `abi_header` (`<Stem>HybridAbi.generated.h`) as the C module
/// `<Stem>HybridAbi` and returns its directory, for `swiftc -I`.
pub fn hybrid_abi_module_dir(host: &SwiftHost, abi_header: &Path) -> PathBuf {
    let file_name = abi_header
        .file_name()
        .and_then(|n| n.to_str())
        .expect("UTF-8 header name");
    let module = file_name
        .strip_suffix(".generated.h")
        .expect("`<Stem>HybridAbi.generated.h`");
    let map_dir = host.modules.join("c").join(module);
    std::fs::create_dir_all(&map_dir).expect("create module map dir");
    std::fs::write(
        map_dir.join("module.modulemap"),
        format!(
            "module {module} {{\n    header \"{}\"\n    export *\n}}\n",
            abi_header.display()
        ),
    )
    .expect("write hybrid module map");
    map_dir
}
