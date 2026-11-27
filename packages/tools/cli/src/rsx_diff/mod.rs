//! Zero-dependency Rust lexer and span-based typed `rsx! { ... }` differ for Tier-1 (`< 10ms`)
//! hot-patching.
//!
//! Compares the cached baseline `.rs` source against the newly saved source:
//! - Ignores comments and whitespace (comment/whitespace-only edits return [`DiffOutcome::NoOp`]).
//! - Indexes every patchable span of the baseline: static text children and native-element style
//!   attribute values ([`attrs`]) of any token count, with template coordinates.
//! - Requires every token outside those spans to match token for token; values inside them may
//!   change token count and are evaluated by the const evaluator ([`eval`]) into typed values.
//! - Emits [`DiffOutcome::Tier1Patch`] with the [`TypedRsxPatch`] and its lowering to
//!   `HotPatchMsg::TemplatePatch` messages (one for text, one per changed element).
//! - Falls back to [`DiffOutcome::RequireTier2Rebuild`] for any Rust logic, control flow, event
//!   handler, component prop, interpolated or non-constant value, identity (`key`/`test_id`),
//!   structural or syntax change.

mod attrs;
mod eval;
mod lexer;
mod matcher;
mod parser;
mod patch;

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use nve_engine::HotPatchMsg;

pub use matcher::diff_rsx_source;
pub use patch::TypedRsxPatch;

/// Classification of a watched `.rs` file save for 2-tier hot reload.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffOutcome {
    /// Comment-only, whitespace-only, or semantically identical save (no patch or rebuild needed).
    NoOp,
    /// Constant text/style value edits inside `rsx! { ... }`: the typed patch plus its
    /// `TemplatePatch` lowering, broadcast via the Tier-1 WebSocket.
    Tier1Patch {
        patch: TypedRsxPatch,
        messages: Vec<HotPatchMsg>,
    },
    /// Any logic, structural, interpolation, non-constant value or syntax change: Tier-2 build.
    RequireTier2Rebuild { reason: String },
}

/// In-memory cache of the last-known baseline source text of every watched `.rs` file.
#[derive(Debug, Default)]
pub struct RsxWatchCache {
    files: HashMap<PathBuf, String>,
}

impl RsxWatchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recursively loads all `.rs` files under `dir` into the cache as the Tier-1 diff baseline.
    pub fn seed_dir(&mut self, dir: &Path) {
        fn walk(dir: &Path, out: &mut HashMap<PathBuf, String>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str())
                        && (name == "target" || name.starts_with('.'))
                    {
                        continue;
                    }
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                    && let Ok(src) = std::fs::read_to_string(&path)
                {
                    out.insert(normalize_cache_key(&path), src);
                }
            }
        }
        walk(dir, &mut self.files);
    }

    /// Records or updates the baseline source text for a single file.
    #[cfg(test)]
    pub fn update_file(&mut self, path: impl AsRef<Path>, source: String) {
        self.files
            .insert(normalize_cache_key(path.as_ref()), source);
    }

    /// Returns the cached baseline source for `path`, if present.
    #[cfg(test)]
    pub fn cached_source(&self, path: &Path) -> Option<&str> {
        self.files
            .get(&normalize_cache_key(path))
            .map(String::as_str)
    }

    /// Number of `.rs` files currently cached.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Diffs a saved file against its cached baseline.
    ///
    /// Updates the cache on [`DiffOutcome::NoOp`] and [`DiffOutcome::Tier1Patch`] so consecutive
    /// Tier-1 saves chain cleanly. Leaves the cache unchanged on
    /// [`DiffOutcome::RequireTier2Rebuild`] so callers update it once the Tier-2 build succeeds
    /// (preventing broken intermediate syntax from poisoning the baseline).
    pub fn diff_file(&mut self, path: &Path, new_source: &str, generation: u64) -> DiffOutcome {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("non-Rust file changed: {}", path.display()),
            };
        }

        let key = normalize_cache_key(path);
        let Some(old_source) = self.files.get(&key) else {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("new uncached Rust file: {}", path.display()),
            };
        };

        let template_id = path.to_string_lossy().into_owned();
        let outcome = diff_rsx_source(&template_id, old_source, new_source, generation);
        if matches!(outcome, DiffOutcome::NoOp | DiffOutcome::Tier1Patch { .. }) {
            self.files.insert(key, new_source.to_owned());
        }
        outcome
    }
}

fn normalize_cache_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
