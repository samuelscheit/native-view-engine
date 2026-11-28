//! Permanent `[INV:single-scene-abi]` scanner: one scene ABI (`NveSceneView` published by
//! `libnve_runtime` straight from `NodeStore` slabs, read by one `SceneReader` per language) and
//! no C++ Fabric shadow-tree bridge. Bans the retired 64-byte per-node scene records, the legacy
//! readers / runtime-prefixed names, and every `cxx/nve-fabric-bridge` path or reference.

use std::collections::BTreeSet;

use crate::persistence_guard::{contains_ident_word, path_contains_norm_token};
use crate::{
    DOC_EXCLUDES, RepoIndex, Violation, is_binary_ext, is_self_invariant_file, rel_path,
    walk_files_no_symlinks,
};

/// Identifiers of the retired legacy scene ABI and bridge C API.
pub const RETIRED_SCENE_SYMBOLS: &[&str] = &[
    "NveRenderNode",
    "NveSceneTable",
    "NveSceneRef",
    "SceneNodeInput",
    "NveNodeMeta",
    "NveSceneBuffer",
    "NveRuntimeSceneView",
    "RuntimeSceneReader",
    "RuntimeSceneViewABI",
    "NveRuntimeScene",
    "NveTestRuntimeSceneView",
    "readRuntimeScene",
    "NVE_SCENE_SCHEMA_HASH",
    "NveFabricHostHandle",
    "NveRnComponentBridge",
    "NveFrameStats",
    "AbiCFunctionDescriptor",
    "SceneMirror",
    "publish_scene_view",
    "nve_web_scene_view_ptr",
];

/// Case- and separator-insensitive (`-`/`_`/CamelCase) substrings naming the deleted bridge.
pub const RETIRED_BRIDGE_TOKENS: &[&str] = &["nve_fabric_bridge", "nve_fabric_host"];

/// Case- and `-`/`_`-normalized snake_case retired scene tokens checked across paths and text.
pub const RETIRED_SCENE_TOKENS: &[&str] = &[
    "nve_render_node",
    "nve_scene_table",
    "nve_scene_buffer",
    "nve_node_meta",
];

/// Historical research notes describe the code they measured; they are not live references.
const HISTORY_PREFIX: &str = "docs/research/";

fn exempt(rel: &str) -> bool {
    is_self_invariant_file(rel) || DOC_EXCLUDES.contains(&rel) || rel.starts_with(HISTORY_PREFIX)
}

/// Case-insensitive substring match that ignores `_` and `-`, catching CamelCase (`NveFabricHost`)
/// alongside `nve-fabric-host` and `NVE_FABRIC_HOST`.
fn contains_squashed_token(haystack: &str, token: &str) -> bool {
    let needle: Vec<u8> = token
        .bytes()
        .filter(|&b| b != b'_' && b != b'-')
        .map(|b| b.to_ascii_lowercase())
        .collect();
    if needle.is_empty() {
        return false;
    }
    let hb = haystack.as_bytes();
    for start in 0..hb.len() {
        let first = hb[start];
        if first == b'_' || first == b'-' || first.to_ascii_lowercase() != needle[0] {
            continue;
        }
        let mut matched = 1usize;
        let mut idx = start + 1;
        while matched < needle.len() && idx < hb.len() {
            let b = hb[idx];
            idx += 1;
            if b == b'_' || b == b'-' {
                continue;
            }
            if b.to_ascii_lowercase() == needle[matched] {
                matched += 1;
            } else {
                break;
            }
        }
        if matched == needle.len() {
            return true;
        }
    }
    false
}

fn match_retired_path_token(rel: &str) -> Option<&'static str> {
    RETIRED_BRIDGE_TOKENS
        .iter()
        .copied()
        .find(|&tok| contains_squashed_token(rel, tok))
        .or_else(|| {
            RETIRED_SCENE_TOKENS
                .iter()
                .copied()
                .find(|&tok| path_contains_norm_token(rel, tok))
        })
}

/// Returns the retired symbol or bridge token `line` mentions, if any.
pub fn detect_retired_scene_reference(line: &str) -> Option<&'static str> {
    RETIRED_SCENE_SYMBOLS
        .iter()
        .copied()
        .find(|&sym| contains_ident_word(line, sym))
        .or_else(|| {
            RETIRED_BRIDGE_TOKENS
                .iter()
                .copied()
                .find(|&tok| contains_squashed_token(line, tok))
        })
        .or_else(|| {
            RETIRED_SCENE_TOKENS
                .iter()
                .copied()
                .find(|&tok| path_contains_norm_token(line, tok))
        })
}

/// Scans every tracked file and live workspace tree (`packages/`, `hosts/`, `cxx/`, `examples/`)
/// for retired scene / bridge paths and references.
pub fn check_single_scene_abi(idx: &RepoIndex) -> Vec<Violation> {
    let mut seen = BTreeSet::new();
    for rel in &idx.tracked {
        if idx.root.join(rel).exists() {
            seen.insert(rel.clone());
        }
    }
    for dir in ["packages", "hosts", "cxx", "examples"] {
        let root_dir = idx.root.join(dir);
        if root_dir.exists() {
            let mut files = Vec::new();
            walk_files_no_symlinks(&root_dir, &mut files);
            for path in files {
                seen.insert(rel_path(&idx.root, &path));
            }
        }
    }

    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for rel in &seen {
        if exempt(rel) {
            continue;
        }
        if let Some(tok) = match_retired_path_token(rel) {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                format!("Path re-introduces retired scene ABI or C++ Fabric bridge (`{tok}`) [INV:single-scene-abi]"),
            ));
        }
        if is_binary_ext(rel) || rel == "Cargo.lock" {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        scanned += 1;
        for (i, line) in content.lines().enumerate() {
            if let Some(hit) = detect_retired_scene_reference(line) {
                violations.push(Violation::new(
                    rel,
                    i + 1,
                    line,
                    format!(
                        "`{hit}` is retired [INV:single-scene-abi]: read the one `NveSceneView` through `SceneReader`; no C++ Fabric bridge"
                    ),
                ));
            }
        }
    }
    assert!(scanned > 0, "single-scene-abi scanner must scan > 0 files");
    violations
}
