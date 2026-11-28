//! Permanent `[INV:single-scene-abi]` scanner: one scene ABI (`NveSceneView` published by
//! `libnve_runtime` straight from `NodeStore` slabs, read by one `SceneReader` per language) and
//! no C++ Fabric shadow-tree bridge. Bans the retired 64-byte per-node scene records, the legacy
//! readers / runtime-prefixed names, and every `cxx/nve-fabric-bridge` path or reference.

use crate::persistence_guard::{contains_ident_word, path_contains_norm_token};
use crate::{DOC_EXCLUDES, RepoIndex, Violation, is_binary_ext, is_self_invariant_file};

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
    "NVE_SCENE_SCHEMA_HASH",
    "NveFabricHostHandle",
    "NveRnComponentBridge",
];

/// Case- and `-`/`_`-normalized substrings naming the deleted C++ Fabric bridge (paths and text).
pub const RETIRED_BRIDGE_TOKENS: &[&str] = &["nve_fabric_bridge", "nve_fabric_host"];

/// Historical research notes describe the code they measured; they are not live references.
const HISTORY_PREFIX: &str = "docs/research/";

fn exempt(rel: &str) -> bool {
    is_self_invariant_file(rel) || DOC_EXCLUDES.contains(&rel) || rel.starts_with(HISTORY_PREFIX)
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
                .find(|&tok| path_contains_norm_token(line, tok))
        })
}

/// Scans every tracked file path and every tracked text file for retired scene / bridge references.
pub fn check_single_scene_abi(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for rel in &idx.tracked {
        if exempt(rel) {
            continue;
        }
        if let Some(tok) = RETIRED_BRIDGE_TOKENS
            .iter()
            .copied()
            .find(|&tok| path_contains_norm_token(rel, tok))
        {
            violations.push(Violation::new(
                rel,
                0,
                rel,
                format!("Path re-introduces the deleted C++ Fabric bridge (`{tok}`) [INV:single-scene-abi]"),
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
