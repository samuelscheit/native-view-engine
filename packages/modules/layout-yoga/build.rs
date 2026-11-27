use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_cpp_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            collect_cpp_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("cpp") {
            out.push(path);
        }
    }
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(nve_has_rn_yoga)");
    println!("cargo:rerun-if-env-changed=RN_REACT_COMMON_DIR");
    println!("cargo:rerun-if-env-changed=RN_COMMON_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=c_api/yoga_shim.cpp");

    let rn_common = env::var_os("RN_REACT_COMMON_DIR")
        .or_else(|| env::var_os("RN_COMMON_DIR"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());

    let Some(rn_common_dir) = rn_common else {
        return;
    };

    let yoga_root = rn_common_dir.join("yoga");
    let yoga_src = yoga_root.join("yoga");
    if !yoga_src.is_dir() {
        println!(
            "cargo:warning=nve-layout-yoga: RN_REACT_COMMON_DIR is set to '{}' but '{}/yoga/yoga' was not found; building without RN Yoga",
            rn_common_dir.display(),
            rn_common_dir.display()
        );
        return;
    }

    let mut cpp_files = Vec::new();
    collect_cpp_files(&yoga_src, &mut cpp_files);
    if cpp_files.is_empty() {
        println!(
            "cargo:warning=nve-layout-yoga: no Yoga .cpp sources found under '{}'; building without RN Yoga",
            yoga_src.display()
        );
        return;
    }

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++20")
        .include(&yoga_root)
        .file("c_api/yoga_shim.cpp")
        .files(&cpp_files)
        .warnings(false);

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "msvc" {
        build.flag("/EHsc").flag("/GR");
    } else {
        build.flag("-fexceptions").flag("-frtti");
    }

    build.compile("nve_rn_yoga");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "android" {
        println!("cargo:rustc-link-lib=log");
    }

    println!("cargo:rustc-cfg=nve_has_rn_yoga");
}
