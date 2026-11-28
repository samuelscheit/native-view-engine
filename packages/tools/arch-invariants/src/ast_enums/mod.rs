//! Cross-file Rust AST checks for:
//! 1. Structural same-file & cross-file duplicated enum match/assignment dispatch tables
//!    (same-direction AND mirrored, enforced unconditionally with a shrink-only ratchet baseline).
//! 2. Duplicate `pub enum` definitions (by name and by identical unit-variant set across workspace),
//!    redundant internal NVE->NVE enum/discriminant converters (by signature AND match body),
//!    compact discriminant matches, and core enum<->str keyword tables (`Stage::Post1B1D`).

mod baseline;
mod converters;
mod extract;
mod extract_arms;
mod fingerprint;
mod free_vars;
mod legacy_tables;
mod normalize;
mod normalize_helpers;
mod patterns;
mod repr_enum_body;
mod repr_enum_guard;
mod scope;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_pass3;
#[cfg(test)]
mod tests_regressions;

use std::sync::OnceLock;

use crate::{
    is_cfg_test_attr, is_self_invariant_file, is_test_or_bench_rs, ParsedRustFile, RepoIndex,
};

pub use baseline::{
    evaluate_duplicates_with_ratchet, AllowlistedDuplicate, DuplicateMatchReport,
    ALLOWLISTED_DUPLICATES,
};
pub use converters::scan_duplicate_enums_and_converters;
pub use fingerprint::{
    scan_duplicate_enum_match_arms, scan_duplicate_enum_match_arms_with_crates, DuplicateKind,
    MatchDuplicate, MIN_DUPLICATE_ARMS,
};
pub use legacy_tables::scan_match_tables;
pub use repr_enum_guard::{
    scan_manual_repr_enum_converters, scan_manual_repr_enum_converters_with_crates,
};

use extract::MatchSite;
use fingerprint::find_duplicate_match_sites;

pub struct WorkspaceProdFile {
    pub rel: String,
    pub crate_name: Option<String>,
    pub parsed: ParsedRustFile,
}

static WORKSPACE_PROD_INDEX: OnceLock<Vec<(String, Option<String>)>> = OnceLock::new();
static WORKSPACE_MATCH_SITES: OnceLock<Vec<MatchSite>> = OnceLock::new();

fn workspace_prod_paths(idx: &RepoIndex) -> &'static [(String, Option<String>)] {
    WORKSPACE_PROD_INDEX.get_or_init(|| {
        let mut pkg_prefixes: Vec<(String, String)> = idx
            .graph
            .workspace()
            .iter()
            .filter_map(|pkg| {
                let dir = pkg.source().workspace_path()?.as_str().replace('\\', "/");
                let prefix = format!("{}/", dir.trim_end_matches('/'));
                let crate_ident = pkg.name().replace('-', "_");
                Some((prefix, crate_ident))
            })
            .collect();
        // Longest directory prefix first so nested crates (e.g. `packages/modules/tv/web/`) win over parents.
        pkg_prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));

        let mut paths = idx.rs_files_under("packages");
        paths.extend(idx.rs_files_under("hosts"));
        paths.extend(idx.rs_files_under("examples"));
        paths.sort();
        paths.dedup();

        paths
            .into_iter()
            .filter(|rel| {
                !is_test_or_bench_rs(rel)
                    && !is_self_invariant_file(rel)
                    && !rel.contains(".generated.")
            })
            .map(|rel| {
                let crate_name = pkg_prefixes
                    .iter()
                    .find(|(prefix, _)| rel.starts_with(prefix.as_str()))
                    .map(|(_, name)| name.clone());
                (rel, crate_name)
            })
            .collect()
    })
}

/// Parses all production `.rs` files across `packages/`, `hosts/`, and `examples/`
/// (skipping tests, benches, generated files, `arch-invariants`, and `#![cfg(test)]` files),
/// reusing `RepoIndex::text_cache` for zero duplicate disk I/O.
pub fn collect_workspace_prod_rs(idx: &RepoIndex) -> Vec<WorkspaceProdFile> {
    let mut parsed = Vec::new();
    for (rel, crate_name) in workspace_prod_paths(idx) {
        if let Some(pf) = idx.parse_rust(rel)
            && !is_cfg_test_attr(&pf.ast.attrs)
        {
            parsed.push(WorkspaceProdFile {
                rel: rel.clone(),
                crate_name: crate_name.clone(),
                parsed: pf,
            });
        }
    }
    parsed
}

fn workspace_match_sites(idx: &RepoIndex) -> &'static [MatchSite] {
    WORKSPACE_MATCH_SITES.get_or_init(|| {
        let mut sites = Vec::new();
        let mut opaque_seed = 0usize;
        for (rel, crate_name) in workspace_prod_paths(idx) {
            let Some(raw) = idx.read_text(rel) else {
                continue;
            };
            if !raw.contains("match") && !raw.contains("if") {
                continue;
            }
            let Some(pf) = idx.parse_rust(rel) else {
                continue;
            };
            if is_cfg_test_attr(&pf.ast.attrs) {
                continue;
            }
            extract::extract_match_sites_with_crate(
                rel,
                &pf.raw,
                &pf.ast,
                crate_name.as_deref(),
                &mut opaque_seed,
                &mut sites,
            );
        }
        sites
    })
}

/// Runs the workspace-wide duplicate enum match/assignment fingerprinter and evaluates against
/// `ALLOWLISTED_DUPLICATES` (ratcheted: new hits AND stale allowlist entries fail immediately).
pub fn check_duplicate_enum_match_arms(idx: &RepoIndex) -> DuplicateMatchReport {
    let sites = workspace_match_sites(idx);
    let dups = find_duplicate_match_sites(sites);
    evaluate_duplicates_with_ratchet(&dups, ALLOWLISTED_DUPLICATES)
}
