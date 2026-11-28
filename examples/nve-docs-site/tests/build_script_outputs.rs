//! `build.rs` outputs must be checkout-independent: a rustc wrapper that caches build-script runs
//! (kache, `docs/UPSTREAM.md`) replays them in other worktrees, so an absolute path in a generated
//! source breaks compilation there once the recording worktree is gone, and an absolute
//! `rerun-if-changed` path makes older kache replay a run recorded against another checkout.

use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const OUT_DIR: &str = env!("OUT_DIR");
const GENERATED_SOURCES: [&str; 2] = [
    "embedded_docs_content.generated.rs",
    "embedded_benchmarks.generated.rs",
];

/// Spellings of this checkout and of the build directory that must never appear in outputs.
fn checkout_specific_paths() -> Vec<String> {
    let workspace = Path::new(MANIFEST_DIR).join("../..");
    let mut paths = vec![
        MANIFEST_DIR.to_owned(),
        OUT_DIR.to_owned(),
        workspace
            .canonicalize()
            .expect("workspace root")
            .display()
            .to_string(),
        Path::new(OUT_DIR)
            .canonicalize()
            .expect("OUT_DIR")
            .display()
            .to_string(),
    ];
    paths.sort();
    paths.dedup();
    paths
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .map_err(|err| format!("{}: {err}", path.display()))
        .expect("readable build-script output")
}

/// The `cargo:` / `cargo::` directives the build script printed (Cargo stores them in `output`
/// next to `OUT_DIR`).
fn build_script_directives() -> String {
    read(
        &Path::new(OUT_DIR)
            .parent()
            .expect("OUT_DIR has a parent")
            .join("output"),
    )
}

fn rerun_if_changed_paths(directives: &str) -> Vec<&str> {
    directives
        .lines()
        .filter_map(|line| {
            line.strip_prefix("cargo:rerun-if-changed=")
                .or_else(|| line.strip_prefix("cargo::rerun-if-changed="))
        })
        .collect()
}

#[test]
fn generated_sources_contain_no_checkout_path() {
    let forbidden = checkout_specific_paths();
    for name in GENERATED_SOURCES {
        let source = read(&Path::new(OUT_DIR).join(name));
        for path in &forbidden {
            assert!(
                !source.contains(path.as_str()),
                "{name} embeds the checkout-specific path {path}"
            );
        }
        assert!(
            !source.contains("include_str!(\"/") && !source.contains("include_bytes!(\"/"),
            "{name} includes a file by absolute path"
        );
    }
}

#[test]
fn rerun_if_changed_paths_are_package_relative_and_exist() {
    let directives = build_script_directives();
    let paths = rerun_if_changed_paths(&directives);
    assert!(
        !paths.is_empty(),
        "build.rs declared no rerun-if-changed inputs"
    );
    for path in paths {
        assert!(
            Path::new(path).is_relative(),
            "rerun-if-changed={path} is absolute; declare it relative to the package"
        );
        let resolved: PathBuf = Path::new(MANIFEST_DIR).join(path);
        assert!(
            resolved.exists(),
            "rerun-if-changed={path} names no existing input"
        );
    }
}

#[test]
fn build_script_directives_contain_no_checkout_path() {
    let directives = build_script_directives();
    for path in checkout_specific_paths() {
        assert!(
            !directives.contains(path.as_str()),
            "build.rs printed the checkout-specific path {path}"
        );
    }
}

#[test]
fn rerun_if_changed_parser_accepts_both_directive_syntaxes() {
    let directives = "cargo:rerun-if-changed=a\ncargo::rerun-if-changed=b\ncargo:warning=c\n";
    assert_eq!(rerun_if_changed_paths(directives), ["a", "b"]);
}

#[test]
fn every_docs_content_file_is_embedded() {
    let embedded = read(&Path::new(OUT_DIR).join(GENERATED_SOURCES[0]));
    let content_root = Path::new(MANIFEST_DIR).join("../../docs/content");
    let mut pending = vec![content_root.clone()];
    let mut count = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read docs/content") {
            let path = entry.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if name == "_meta.toml" || name.ends_with(".md") || name.ends_with(".mdx") {
                let rel = path
                    .strip_prefix(&content_root)
                    .expect("under content root");
                let rel = rel.to_string_lossy().replace('\\', "/");
                assert!(
                    embedded.contains(&format!("\"/../../docs/content/{rel}\"")),
                    "docs/content/{rel} is not embedded"
                );
                count += 1;
            }
        }
    }
    assert!(count > 0, "docs/content has no embeddable files");
}
