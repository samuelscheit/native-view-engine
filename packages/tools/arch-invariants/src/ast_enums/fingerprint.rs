//! Inverted-index same-direction & mirrored duplicate detector for `MatchSite` fingerprints.

use std::collections::{BTreeMap, BTreeSet};

use super::extract::{extract_match_sites, MatchSite};
use super::patterns::{MirrorAtom, MirrorKey};

pub const MIN_DUPLICATE_ARMS: usize = 4;

/// Minimum percentage of shared enum variants that must have identical normalized bodies
/// (prevents cross-language coincidences like C vs Dart sharing `"bool"`/`"double"`).
const MIN_SHARED_PAT_AGREEMENT_PCT: usize = 70;

/// Minimum percentage of the smaller match's arms covered by identical `(pat, body)` pairs.
const MIN_SMALLER_MATCH_COVERAGE_PCT: usize = 60;

/// Minimum percentage of patterns within a single syntactic `Pat::Or` arm that must match
/// for that syntactic arm to count as matched (prevents `0 => Zero` from counting `0|7|8|9 => Zero`
/// as a full syntactic match).
const MIN_ARM_INTERNAL_MATCH_PCT: usize = 75;

/// Minimum percentage of shared syntactic arms (pre-`Pat::Or` expansion) that must match
/// (prevents one giant `U8 | U16 | ... | U64` `Pat::Or` arm from inflating overlap when
/// only 2 out of 6 syntactic arms agree).
const MIN_SYNTACTIC_ARM_AGREEMENT_PCT: usize = 60;

/// Minimum percentage of shared enum variants that must mirror for an inverse pair.
const MIN_MIRROR_VARIANT_AGREEMENT_PCT: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DuplicateKind {
    SameDirection,
    Mirrored,
}

impl DuplicateKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SameDirection => "same-direction",
            Self::Mirrored => "mirrored",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchDuplicate {
    pub kind: DuplicateKind,
    pub file_a: String,
    pub symbol_a: String,
    pub line_a: usize,
    pub snippet_a: String,
    pub file_b: String,
    pub symbol_b: String,
    pub line_b: usize,
    pub enum_name: String,
    pub overlap_arms: usize,
    pub distinct_bodies: usize,
}

pub fn scan_duplicate_enum_match_arms(files: &[(&str, &str, &syn::File)]) -> Vec<MatchDuplicate> {
    let mut all_sites = Vec::new();
    for &(rel, raw, ast) in files {
        extract_match_sites(rel, raw, ast, &mut all_sites);
    }
    find_duplicate_match_sites(&all_sites)
}

pub fn find_duplicate_match_sites(sites: &[MatchSite]) -> Vec<MatchDuplicate> {
    let mut out = Vec::new();
    let mut reported_pairs = BTreeSet::new();

    detect_same_direction(sites, &mut reported_pairs, &mut out);
    detect_mirrored(sites, &reported_pairs, &mut out);

    out.sort_by(|x, y| {
        (
            x.kind,
            &x.file_a,
            &x.symbol_a,
            x.line_a,
            &x.file_b,
            &x.symbol_b,
            x.line_b,
        )
            .cmp(&(
                y.kind,
                &y.file_a,
                &y.symbol_a,
                y.line_a,
                &y.file_b,
                &y.symbol_b,
                y.line_b,
            ))
    });
    out
}

fn detect_same_direction(
    sites: &[MatchSite],
    reported_pairs: &mut BTreeSet<(usize, usize)>,
    out: &mut Vec<MatchDuplicate>,
) {
    let mut arm_index: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    for (idx, site) in sites.iter().enumerate() {
        if site.direction.is_none() {
            continue;
        }
        for arm in &site.arms {
            arm_index
                .entry((arm.pat_key.as_str(), arm.body_key.as_str()))
                .or_default()
                .push(idx);
        }
    }

    let mut candidates: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for postings in arm_index.values() {
        if postings.len() < 2 {
            continue;
        }
        for a in 0..postings.len() {
            for b in (a + 1)..postings.len() {
                let (i, j) = (postings[a], postings[b]);
                if i != j {
                    let pair = if i < j { (i, j) } else { (j, i) };
                    *candidates.entry(pair).or_insert(0) += 1;
                }
            }
        }
    }

    for ((i, j), rough) in candidates {
        if rough < MIN_DUPLICATE_ARMS {
            continue;
        }
        let (a, b) = (&sites[i], &sites[j]);
        if a.direction != b.direction || !qualifiers_compatible(a, b) {
            continue;
        }
        let a_pats: BTreeSet<&str> = a.arms.iter().map(|arm| arm.pat_key.as_str()).collect();
        let b_pats: BTreeSet<&str> = b.arms.iter().map(|arm| arm.pat_key.as_str()).collect();
        let a_pairs: BTreeSet<(&str, &str)> = a
            .arms
            .iter()
            .map(|arm| (arm.pat_key.as_str(), arm.body_key.as_str()))
            .collect();
        let b_pairs: BTreeSet<(&str, &str)> = b
            .arms
            .iter()
            .map(|arm| (arm.pat_key.as_str(), arm.body_key.as_str()))
            .collect();

        let mut shared_pats = 0usize;
        let mut shared_arms = 0usize;
        let mut distinct_non_noop = BTreeSet::new();
        let mut syn_total_a: BTreeMap<usize, usize> = BTreeMap::new();
        let mut syn_matched_a: BTreeMap<usize, usize> = BTreeMap::new();
        let mut shared_syn_a = BTreeSet::new();
        for arm in &a.arms {
            *syn_total_a.entry(arm.arm_index).or_insert(0) += 1;
            if b_pats.contains(arm.pat_key.as_str()) {
                shared_pats += 1;
                shared_syn_a.insert(arm.arm_index);
            }
            if b_pairs.contains(&(arm.pat_key.as_str(), arm.body_key.as_str())) {
                shared_arms += 1;
                *syn_matched_a.entry(arm.arm_index).or_insert(0) += 1;
                if !arm.is_noop {
                    distinct_non_noop.insert(arm.body_key.as_str());
                }
            }
        }

        let mut syn_total_b: BTreeMap<usize, usize> = BTreeMap::new();
        let mut syn_matched_b: BTreeMap<usize, usize> = BTreeMap::new();
        let mut shared_syn_b = BTreeSet::new();
        for arm in &b.arms {
            *syn_total_b.entry(arm.arm_index).or_insert(0) += 1;
            if a_pats.contains(arm.pat_key.as_str()) {
                shared_syn_b.insert(arm.arm_index);
            }
            if a_pairs.contains(&(arm.pat_key.as_str(), arm.body_key.as_str())) {
                *syn_matched_b.entry(arm.arm_index).or_insert(0) += 1;
            }
        }

        let matched_syn_a = shared_syn_a
            .iter()
            .filter(|idx| {
                let total = syn_total_a.get(idx).copied().unwrap_or(0);
                let matched = syn_matched_a.get(idx).copied().unwrap_or(0);
                total > 0 && matched * 100 >= total * MIN_ARM_INTERNAL_MATCH_PCT
            })
            .count();
        let matched_syn_b = shared_syn_b
            .iter()
            .filter(|idx| {
                let total = syn_total_b.get(idx).copied().unwrap_or(0);
                let matched = syn_matched_b.get(idx).copied().unwrap_or(0);
                total > 0 && matched * 100 >= total * MIN_ARM_INTERNAL_MATCH_PCT
            })
            .count();

        let min_arms = a.arms.len().min(b.arms.len());
        if shared_arms < MIN_DUPLICATE_ARMS
            || shared_pats == 0
            || shared_arms * 100 < shared_pats * MIN_SHARED_PAT_AGREEMENT_PCT
            || shared_arms * 100 < min_arms * MIN_SMALLER_MATCH_COVERAGE_PCT
            || shared_syn_a.is_empty()
            || shared_syn_b.is_empty()
            || matched_syn_a * 100 < shared_syn_a.len() * MIN_SYNTACTIC_ARM_AGREEMENT_PCT
            || matched_syn_b * 100 < shared_syn_b.len() * MIN_SYNTACTIC_ARM_AGREEMENT_PCT
        {
            continue;
        }

        let exact_whole_match = shared_arms == a.arms.len()
            && shared_arms == b.arms.len()
            && a.wildcard_body == b.wildcard_body
            && !distinct_non_noop.is_empty();

        if (a.qualifier.is_none() || b.qualifier.is_none()) && !exact_whole_match {
            continue;
        }

        if distinct_non_noop.len() >= 2 || exact_whole_match {
            reported_pairs.insert((i, j));
            out.push(make_duplicate(
                DuplicateKind::SameDirection,
                a,
                b,
                shared_arms,
                distinct_non_noop.len(),
            ));
        }
    }
}

fn detect_mirrored(
    sites: &[MatchSite],
    reported_pairs: &BTreeSet<(usize, usize)>,
    out: &mut Vec<MatchDuplicate>,
) {
    let mut mirror_index: BTreeMap<(MirrorKey, MirrorKey), Vec<usize>> = BTreeMap::new();
    for (idx, site) in sites.iter().enumerate() {
        for (l, r) in &site.mirror_pairs {
            mirror_index
                .entry((l.key_ignoring_qualifier(), r.key_ignoring_qualifier()))
                .or_default()
                .push(idx);
        }
    }

    let mut candidates: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (i, site) in sites.iter().enumerate() {
        for (l, r) in &site.mirror_pairs {
            let rev = (r.key_ignoring_qualifier(), l.key_ignoring_qualifier());
            if let Some(matches) = mirror_index.get(&rev) {
                for &j in matches {
                    if i < j && !reported_pairs.contains(&(i, j)) {
                        *candidates.entry((i, j)).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    for ((i, j), rough) in candidates {
        if rough < MIN_DUPLICATE_ARMS {
            continue;
        }
        let (a, b) = (&sites[i], &sites[j]);
        let mut overlap = 0usize;
        for (al, ar) in &a.mirror_pairs {
            if b.mirror_pairs.iter().any(|(bl, br)| {
                mirror_atoms_compatible(al, br, a, b) && mirror_atoms_compatible(ar, bl, a, b)
            }) {
                overlap += 1;
            }
        }
        let a_variants: BTreeSet<&str> = a
            .mirror_pairs
            .iter()
            .filter_map(|(l, r)| l.variant_key().or_else(|| r.variant_key()))
            .collect();
        let b_variants: BTreeSet<&str> = b
            .mirror_pairs
            .iter()
            .filter_map(|(l, r)| l.variant_key().or_else(|| r.variant_key()))
            .collect();
        let shared_variants = a_variants.intersection(&b_variants).count();

        if overlap >= MIN_DUPLICATE_ARMS
            && shared_variants > 0
            && overlap * 100 >= shared_variants * MIN_MIRROR_VARIANT_AGREEMENT_PCT
        {
            out.push(make_duplicate(DuplicateKind::Mirrored, a, b, overlap, overlap));
        }
    }
}

fn make_duplicate(
    kind: DuplicateKind,
    a: &MatchSite,
    b: &MatchSite,
    overlap_arms: usize,
    distinct_bodies: usize,
) -> MatchDuplicate {
    let (first, second) = if (&a.file, &a.symbol, a.line) <= (&b.file, &b.symbol, b.line) {
        (a, b)
    } else {
        (b, a)
    };
    let enum_name = if first.enum_name != "<unqualified>" {
        first.enum_name.clone()
    } else {
        second.enum_name.clone()
    };
    MatchDuplicate {
        kind,
        file_a: first.file.clone(),
        symbol_a: first.symbol.clone(),
        line_a: first.line,
        snippet_a: first.snippet.clone(),
        file_b: second.file.clone(),
        symbol_b: second.symbol.clone(),
        line_b: second.line,
        enum_name,
        overlap_arms,
        distinct_bodies,
    }
}

fn qualifiers_compatible(a: &MatchSite, b: &MatchSite) -> bool {
    match (&a.qualifier, &b.qualifier) {
        (Some(qa), Some(qb)) => qa == qb,
        (None, None) => true,
        (Some(q), None) => b.glob_enums.contains(q) || b.has_unscoped_glob,
        (None, Some(q)) => a.glob_enums.contains(q) || a.has_unscoped_glob,
    }
}

fn mirror_atoms_compatible(
    x: &MirrorAtom,
    y: &MirrorAtom,
    site_x: &MatchSite,
    site_y: &MatchSite,
) -> bool {
    match (x, y) {
        (MirrorAtom::Str(a), MirrorAtom::Str(b)) => a == b,
        (MirrorAtom::Int(a), MirrorAtom::Int(b)) => a == b,
        (MirrorAtom::Char(a), MirrorAtom::Char(b)) => a == b,
        (
            MirrorAtom::Variant {
                qualifier: qa,
                variant: va,
            },
            MirrorAtom::Variant {
                qualifier: qb,
                variant: vb,
            },
        ) => {
            if va != vb {
                return false;
            }
            match (qa, qb) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                (Some(q), None) => site_y.glob_enums.contains(q) || site_y.has_unscoped_glob,
                (None, Some(q)) => site_x.glob_enums.contains(q) || site_x.has_unscoped_glob,
            }
        }
        _ => false,
    }
}
