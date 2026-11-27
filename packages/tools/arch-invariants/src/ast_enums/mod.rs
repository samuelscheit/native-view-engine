//! Cross-file Rust AST checks for:
//! 1. Structural same-file & cross-file duplicated enum match/assignment dispatch tables
//!    (same-direction AND mirrored, enforced unconditionally with a shrink-only ratchet baseline).
//! 2. Duplicate `pub enum` definitions (by name and by identical unit-variant set across workspace),
//!    redundant internal NVE->NVE enum/discriminant converters (by signature AND match body),
//!    compact discriminant matches, and core enum<->str keyword tables (`Stage::Post1B1D`).

pub mod baseline;
pub mod converters;
mod extract;
pub mod fingerprint;
pub mod legacy_tables;
mod normalize;
mod patterns;
#[cfg(test)]
mod tests;

use crate::{
    is_cfg_test_attr, is_self_invariant_file, is_test_or_bench_rs, ParsedRustFile, RepoIndex,
};

pub use baseline::{
    evaluate_duplicates_with_ratchet, AllowlistedDuplicate, DuplicateMatchReport,
    ALLOWLISTED_DUPLICATES,
};
pub use converters::scan_duplicate_enums_and_converters;
pub use extract::{extract_match_sites, MatchSite};
pub use fingerprint::{
    find_duplicate_match_sites, scan_duplicate_enum_match_arms, DuplicateKind, MatchDuplicate,
    MIN_DUPLICATE_ARMS,
};
pub use legacy_tables::scan_match_tables;
pub use patterns::EXTERNAL_SDK_ROOTS;

/// Collects and parses all production `.rs` files across `packages/`, `hosts/`, and `examples/`,
/// skipping tests, benches, generated files, and `arch-invariants` itself.
pub fn collect_workspace_prod_rs(idx: &RepoIndex) -> Vec<(String, ParsedRustFile)> {
    let mut paths = idx.rs_files_under("packages");
    paths.extend(idx.rs_files_under("hosts"));
    paths.extend(idx.rs_files_under("examples"));
    paths.sort();
    paths.dedup();

    let mut parsed = Vec::new();
    for rel in paths {
        if is_test_or_bench_rs(&rel)
            || is_self_invariant_file(&rel)
            || rel.contains(".generated.")
        {
            continue;
        }
        if let Some(pf) = idx.parse_rust(&rel)
            && !is_cfg_test_attr(&pf.ast.attrs)
        {
            parsed.push((rel, pf));
        }
    }
    parsed
}

/// Runs the workspace-wide duplicate enum match/assignment fingerprinter and evaluates against
/// `ALLOWLISTED_DUPLICATES` (ratcheted: new hits AND stale allowlist entries fail immediately).
pub fn check_duplicate_enum_match_arms(idx: &RepoIndex) -> DuplicateMatchReport {
    let owned = collect_workspace_prod_rs(idx);
    let refs: Vec<(&str, &str, &syn::File)> = owned
        .iter()
        .map(|(r, p)| (r.as_str(), p.raw.as_str(), &p.ast))
        .collect();
    let dups = scan_duplicate_enum_match_arms(&refs);
    evaluate_duplicates_with_ratchet(&dups, ALLOWLISTED_DUPLICATES)
}
