//! Cached diff baselines: each watched file keeps its source, compact tokens and lazily built
//! [`RsxIndex`], so a save only lexes the new text.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::DiffOutcome;
use super::lexer::{Lex, Tokens, tokenize};
use super::matcher::classify;
use super::parser::RsxIndex;

/// The last accepted source of one file with its cached tokens and patch index.
#[derive(Debug)]
pub struct Baseline {
    src: String,
    tokens: Option<Tokens>,
    index: Option<RsxIndex>,
}

impl Baseline {
    pub fn new(src: String) -> Self {
        Self {
            src,
            tokens: None,
            index: None,
        }
    }

    /// Classifies `new_src` against this baseline without advancing it (caches fill lazily).
    /// Also returns the new source's tokens so an accepted save can adopt them.
    pub fn classify(
        &mut self,
        template_id: &str,
        new_src: &str,
        generation: u64,
    ) -> (DiffOutcome, Option<Tokens>) {
        if self.src == new_src {
            return (DiffOutcome::NoOp, None);
        }
        if self.tokens.is_none() {
            match tokenize(&self.src) {
                Ok(tokens) => self.tokens = Some(tokens),
                Err(e) => {
                    let reason = format!("baseline lex error: {}", e.as_str());
                    return (DiffOutcome::RequireTier2Rebuild { reason }, None);
                }
            }
        }
        let new_tokens = match tokenize(new_src) {
            Ok(tokens) => tokens,
            Err(e) => {
                let reason = format!("syntax error: {}", e.as_str());
                return (DiffOutcome::RequireTier2Rebuild { reason }, None);
            }
        };
        let Some(old_tokens) = self.tokens.as_ref() else {
            return (DiffOutcome::NoOp, None);
        };
        let outcome = classify(
            template_id,
            Lex::new(&self.src, old_tokens),
            &mut self.index,
            Lex::new(new_src, &new_tokens),
            generation,
        );
        (outcome, Some(new_tokens))
    }

    /// Classifies `new_src` and, on [`DiffOutcome::NoOp`] / [`DiffOutcome::Tier1Patch`], makes it
    /// the new baseline (reusing its tokens). [`DiffOutcome::RequireTier2Rebuild`] keeps the old
    /// baseline so broken intermediate syntax never poisons it.
    pub fn diff(&mut self, template_id: &str, new_src: &str, generation: u64) -> DiffOutcome {
        let (outcome, new_tokens) = self.classify(template_id, new_src, generation);
        if matches!(outcome, DiffOutcome::NoOp | DiffOutcome::Tier1Patch { .. })
            && self.src != new_src
        {
            // Token-identical saves keep the index valid (it stores token indices only).
            let same_tokens = match (&self.tokens, &new_tokens) {
                (Some(old), Some(new)) => {
                    let (old, new) = (Lex::new(&self.src, old), Lex::new(new_src, new));
                    old.len() == new.len() && old.same_range(0, old.len(), new, 0)
                }
                _ => false,
            };
            if !same_tokens {
                self.index = None;
            }
            self.src = new_src.to_owned();
            self.tokens = new_tokens;
        }
        outcome
    }
}

/// Baselines of every watched `.rs` file.
#[derive(Debug, Default)]
pub struct RsxWatchCache {
    files: HashMap<PathBuf, Baseline>,
}

impl RsxWatchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Recursively loads all `.rs` files under `dir` as Tier-1 diff baselines (lexed lazily).
    pub fn seed_dir(&mut self, dir: &Path) {
        fn walk(dir: &Path, out: &mut HashMap<PathBuf, Baseline>) {
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
                    out.insert(normalize_cache_key(&path), Baseline::new(src));
                }
            }
        }
        walk(dir, &mut self.files);
    }

    /// Records or replaces the baseline source text for a single file.
    #[cfg(test)]
    pub fn update_file(&mut self, path: impl AsRef<Path>, source: String) {
        self.files
            .insert(normalize_cache_key(path.as_ref()), Baseline::new(source));
    }

    /// Returns the cached baseline source for `path`, if present.
    #[cfg(test)]
    pub fn cached_source(&self, path: &Path) -> Option<&str> {
        self.files
            .get(&normalize_cache_key(path))
            .map(|b| b.src.as_str())
    }

    /// Number of `.rs` files currently cached.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Diffs a saved file against its cached baseline (see [`Baseline::diff`]); callers update
    /// the baseline after a successful Tier-2 build by re-seeding.
    pub fn diff_file(&mut self, path: &Path, new_source: &str, generation: u64) -> DiffOutcome {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("non-Rust file changed: {}", path.display()),
            };
        }
        let Some(baseline) = self.files.get_mut(&normalize_cache_key(path)) else {
            return DiffOutcome::RequireTier2Rebuild {
                reason: format!("new uncached Rust file: {}", path.display()),
            };
        };
        baseline.diff(&path.to_string_lossy(), new_source, generation)
    }
}

fn normalize_cache_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
