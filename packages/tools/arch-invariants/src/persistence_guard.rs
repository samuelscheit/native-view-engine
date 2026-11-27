//! Permanent `[INV:no-reloc-mmap-snapshot]` path and symbol scanners banning re-introduction of
//! retired `mmap_arena`, `reloc_scene`, `nve_snapshot`, `instant_boot`, `mmap_snapshot`, and
//! `/reloc/` persistence code across the repository.

use std::collections::BTreeSet;

use crate::{
    DOC_EXCLUDES, RepoIndex, Violation, is_binary_ext, is_self_invariant_file, rel_path,
    walk_files_no_symlinks,
};

/// Legacy persistence / mmap-reloc path tokens permanently retired in Track 1B-2 Slice 2A-2.
pub const BANNED_LEGACY_PERSISTENCE_PATH_TOKENS: &[&str] = &[
    "mmap_arena",
    "reloc_scene",
    "nve_snapshot",
    "instant_boot",
    "mmap_snapshot",
];

/// Legacy persistence / mmap-reloc type symbols permanently retired in Track 1B-2 Slice 2A-2.
pub const BANNED_LEGACY_PERSISTENCE_SYMBOLS: &[&str] = &[
    "MmapOffsetArena",
    "RelocSceneNode",
    "RelocSceneRoot",
    "RelocatableStateArena",
];

#[inline]
fn norm_path_byte(b: u8) -> u8 {
    match b {
        b'\\' => b'/',
        b'-' => b'_',
        other => other.to_ascii_lowercase(),
    }
}

fn path_contains_norm_token(rel: &str, token: &str) -> bool {
    let tb = token.as_bytes();
    let rb = rel.as_bytes();
    !tb.is_empty()
        && rb.windows(tb.len()).any(|w| {
            w.iter()
                .zip(tb.iter())
                .all(|(&a, &b)| norm_path_byte(a) == b)
        })
}

/// Returns the matched retired legacy persistence token if `rel` re-introduces a banned path
/// (zero-alloc case- and hyphen-normalized, including any `/reloc/` directory segment).
pub fn is_banned_legacy_persistence_path(rel: &str) -> Option<&'static str> {
    if is_self_invariant_file(rel) || DOC_EXCLUDES.contains(&rel) {
        return None;
    }
    if let Some(tok) = BANNED_LEGACY_PERSISTENCE_PATH_TOKENS
        .iter()
        .copied()
        .find(|&tok| path_contains_norm_token(rel, tok))
    {
        return Some(tok);
    }
    if rel.split(['/', '\\']).any(|seg| {
        seg.eq_ignore_ascii_case("reloc")
            || seg
                .split('.')
                .next()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("reloc"))
    }) {
        return Some("/reloc/");
    }
    None
}

fn contains_ident_word(line: &str, ident: &str) -> bool {
    let bytes = line.as_bytes();
    let needle = ident.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut pos = 0;
    while let Some(off) = line[pos..].find(ident) {
        let start = pos + off;
        let end = start + needle.len();
        let left_ok = start == 0 || !is_ident(bytes[start - 1]);
        let right_ok = end == bytes.len() || !is_ident(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        pos = start + 1;
    }
    false
}

/// Returns the matched banned legacy persistence symbol if `line` mentions one as an identifier.
pub fn detect_banned_legacy_persistence_symbol(line: &str) -> Option<&'static str> {
    BANNED_LEGACY_PERSISTENCE_SYMBOLS
        .iter()
        .copied()
        .find(|&sym| contains_ident_word(line, sym))
}

/// Scans tracked non-binary files (excluding historical changelog/audit docs & self-invariant code)
/// for re-introduced legacy reloc/mmap persistence symbols.
pub fn check_banned_legacy_persistence_symbols(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        for (i, line) in content.lines().enumerate() {
            if let Some(sym) = detect_banned_legacy_persistence_symbol(line) {
                violations.push(Violation::new(
                    rel,
                    i + 1,
                    line,
                    format!(
                        "Retired legacy persistence symbol `{sym}` is permanently banned ([INV:no-reloc-mmap-snapshot]); use Frame-0 `NVE2` (`Frame0File`) + atomic `StateFile`"
                    ),
                ));
            }
        }
    }
    violations
}

/// Bans re-introduction of deleted `mmap_arena`, `reloc_scene`, `nve_snapshot`, `instant_boot`,
/// `mmap_snapshot`, or `/reloc/` paths across tracked files and live workspace trees
/// (`packages/`, `hosts/`, `cxx/`, `examples/`).
pub fn check_banned_legacy_persistence_paths(idx: &RepoIndex) -> Vec<Violation> {
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
    assert!(
        !seen.is_empty(),
        "banned legacy persistence scanner must scan > 0 files"
    );

    let mut violations = Vec::new();
    for rel in seen {
        if let Some(tok) = is_banned_legacy_persistence_path(&rel) {
            violations.push(Violation::new(
                &rel,
                0,
                &rel,
                format!(
                    "Retired legacy persistence path token `{tok}` is permanently banned; use Frame-0 `NVE2` (`Frame0File`) + atomic `StateFile`"
                ),
            ));
        }
    }
    violations
}
