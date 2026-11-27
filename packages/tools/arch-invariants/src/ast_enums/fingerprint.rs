//! Inverted-index same-direction & mirrored duplicate detector for `MatchSite` fingerprints.

use std::collections::{BTreeMap, BTreeSet};

use super::extract::{extract_match_sites_with_crate, MatchSite};
use super::normalize_helpers::is_noop_body;
use super::patterns::{MirrorAtom, MirrorKey};
use super::scope::EnumQualifier;

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
/// (set at 65% so 3/5 cross-language `Bool/Int/Float` zero-value coincidences drop while 2/3+ real duplicates match).
const MIN_SYNTACTIC_ARM_AGREEMENT_PCT: usize = 65;

/// Minimum percentage of shared domain entries that must mirror for an inverse pair.
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
    let owned: Vec<(&str, &str, &syn::File, Option<&str>)> = files
        .iter()
        .map(|&(rel, raw, ast)| (rel, raw, ast, None))
        .collect();
    scan_duplicate_enum_match_arms_with_crates(&owned)
}

pub fn scan_duplicate_enum_match_arms_with_crates(
    files: &[(&str, &str, &syn::File, Option<&str>)],
) -> Vec<MatchDuplicate> {
    let mut all_sites = Vec::new();
    let mut opaque_seed = 0usize;
    for &(rel, raw, ast, crate_name) in files {
        extract_match_sites_with_crate(rel, raw, ast, crate_name, &mut opaque_seed, &mut all_sites);
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

fn variant_head(pat_key: &str) -> &str {
    let without_guard = pat_key.split(" if ").next().unwrap_or(pat_key);
    let cut = without_guard
        .find(['(', '{'])
        .unwrap_or(without_guard.len());
    &without_guard[..cut]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QualifierMatch {
    Incompatible,
    ExplicitOrGlobMatched,
    UnqualifiedOnly,
}

fn qualifiers_compatible(a: &MatchSite, b: &MatchSite) -> QualifierMatch {
    if a.tuple_qualifier.is_some() || b.tuple_qualifier.is_some() {
        return match (&a.tuple_qualifier, &b.tuple_qualifier) {
            (Some(ta), Some(tb)) if ta == tb => QualifierMatch::ExplicitOrGlobMatched,
            _ => QualifierMatch::Incompatible,
        };
    }
    match (&a.enum_qualifier, &b.enum_qualifier) {
        (Some(qa), Some(qb)) => {
            if qa.compatible_with(qb) {
                QualifierMatch::ExplicitOrGlobMatched
            } else {
                QualifierMatch::Incompatible
            }
        }
        (None, None) => QualifierMatch::UnqualifiedOnly,
        (Some(q), None) => {
            if b.glob_enums.iter().any(|g| g.compatible_with(q)) {
                QualifierMatch::ExplicitOrGlobMatched
            } else if b.has_unscoped_glob {
                QualifierMatch::UnqualifiedOnly
            } else {
                QualifierMatch::Incompatible
            }
        }
        (None, Some(q)) => {
            if a.glob_enums.iter().any(|g| g.compatible_with(q)) {
                QualifierMatch::ExplicitOrGlobMatched
            } else if a.has_unscoped_glob {
                QualifierMatch::UnqualifiedOnly
            } else {
                QualifierMatch::Incompatible
            }
        }
    }
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
        if a.direction != b.direction {
            continue;
        }
        let qmatch = qualifiers_compatible(a, b);
        if qmatch == QualifierMatch::Incompatible {
            continue;
        }

        // Match `shared_pats` on variant head (ignoring guard/binding differences) so a guarded
        // disagreeing variant like `Struct(name) if is_pod => ...` still counts as a shared
        // disagreeing arm rather than vanishing from the denominator.
        let a_heads: BTreeSet<&str> = a.arms.iter().map(|arm| variant_head(&arm.pat_key)).collect();
        let b_heads: BTreeSet<&str> = b.arms.iter().map(|arm| variant_head(&arm.pat_key)).collect();
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

        let a_has_semantic_wild = a.wildcard_body.as_deref().is_some_and(|w| !is_noop_body(w));
        let b_has_semantic_wild = b.wildcard_body.as_deref().is_some_and(|w| !is_noop_body(w));

        let mut shared_pats = 0usize;
        let mut shared_arms = 0usize;
        let mut distinct_discriminating = BTreeSet::new();
        let mut syn_total_a: BTreeMap<usize, usize> = BTreeMap::new();
        let mut syn_matched_a: BTreeMap<usize, usize> = BTreeMap::new();
        let mut shared_syn_a = BTreeSet::new();
        for arm in &a.arms {
            *syn_total_a.entry(arm.arm_index).or_insert(0) += 1;
            if b_heads.contains(variant_head(&arm.pat_key)) {
                shared_pats += 1;
                shared_syn_a.insert(arm.arm_index);
            }
            if b_pairs.contains(&(arm.pat_key.as_str(), arm.body_key.as_str())) {
                shared_arms += 1;
                *syn_matched_a.entry(arm.arm_index).or_insert(0) += 1;
                if !arm.is_noop && !arm.is_bare_outer {
                    distinct_discriminating.insert(arm.body_key.as_str());
                }
            }
        }

        let mut syn_total_b: BTreeMap<usize, usize> = BTreeMap::new();
        let mut syn_matched_b: BTreeMap<usize, usize> = BTreeMap::new();
        let mut shared_syn_b = BTreeSet::new();
        for arm in &b.arms {
            *syn_total_b.entry(arm.arm_index).or_insert(0) += 1;
            if a_heads.contains(variant_head(&arm.pat_key)) {
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

        // When both sides have distinct non-noop wildcards, count the wildcard itself as a disagreeing syntactic arm.
        let both_distinct_wild = a_has_semantic_wild
            && b_has_semantic_wild
            && a.wildcard_body != b.wildcard_body;
        let wild_penalty = usize::from(both_distinct_wild);
        let denom_syn_a = shared_syn_a.len() + wild_penalty;
        let denom_syn_b = shared_syn_b.len() + wild_penalty;

        let min_arms = a.arms.len().min(b.arms.len());
        if shared_arms < MIN_DUPLICATE_ARMS
            || shared_pats == 0
            || shared_arms * 100 < shared_pats * MIN_SHARED_PAT_AGREEMENT_PCT
            || shared_arms * 100 < min_arms * MIN_SMALLER_MATCH_COVERAGE_PCT
            || denom_syn_a == 0
            || denom_syn_b == 0
            || matched_syn_a * 100 < denom_syn_a * MIN_SYNTACTIC_ARM_AGREEMENT_PCT
            || matched_syn_b * 100 < denom_syn_b * MIN_SYNTACTIC_ARM_AGREEMENT_PCT
        {
            continue;
        }

        let exact_whole_match = shared_arms == a.arms.len()
            && shared_arms == b.arms.len()
            && a.wildcard_body == b.wildcard_body
            && !distinct_discriminating.is_empty();

        // Only require exact whole-match agreement when neither side matched an explicit or glob qualifier.
        if qmatch == QualifierMatch::UnqualifiedOnly && !exact_whole_match {
            continue;
        }

        if distinct_discriminating.len() >= 2 || exact_whole_match {
            reported_pairs.insert((i, j));
            out.push(make_duplicate(
                DuplicateKind::SameDirection,
                a,
                b,
                shared_arms,
                distinct_discriminating.len(),
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

        // O(M log M) maps on `b` (replaces O(M^2) scan and handles EnumA <-> EnumB with distinct variant names).
        let mut b_by_pair: BTreeMap<(MirrorKey, MirrorKey), Vec<(&MirrorAtom, &MirrorAtom)>> =
            BTreeMap::new();
        let mut b_lhs_by_key: BTreeMap<MirrorKey, Vec<&MirrorAtom>> = BTreeMap::new();
        let mut b_rhs_by_key: BTreeMap<MirrorKey, Vec<&MirrorAtom>> = BTreeMap::new();
        for (bl, br) in &b.mirror_pairs {
            let lk = bl.key_ignoring_qualifier();
            let rk = br.key_ignoring_qualifier();
            b_by_pair.entry((lk.clone(), rk.clone())).or_default().push((bl, br));
            b_lhs_by_key.entry(lk).or_default().push(bl);
            b_rhs_by_key.entry(rk).or_default().push(br);
        }

        let mut overlap = 0usize;
        let mut shared_lhs_to_rhs = 0usize;
        let mut shared_rhs_to_lhs = 0usize;
        let mut seen_a_lhs = BTreeSet::new();
        let mut seen_a_rhs = BTreeSet::new();

        for (al, ar) in &a.mirror_pairs {
            let alk = al.key_ignoring_qualifier();
            let ark = ar.key_ignoring_qualifier();
            if let Some(rev_list) = b_by_pair.get(&(ark.clone(), alk.clone()))
                && rev_list.iter().any(|&(bl, br)| {
                    mirror_atoms_compatible(al, br, a, b) && mirror_atoms_compatible(ar, bl, a, b)
                })
            {
                overlap += 1;
            }
            if seen_a_lhs.insert(al)
                && let Some(br_list) = b_rhs_by_key.get(&alk)
                && br_list.iter().any(|br| mirror_atoms_compatible(al, br, a, b))
            {
                shared_lhs_to_rhs += 1;
            }
            if seen_a_rhs.insert(ar)
                && let Some(bl_list) = b_lhs_by_key.get(&ark)
                && bl_list.iter().any(|bl| mirror_atoms_compatible(ar, bl, a, b))
            {
                shared_rhs_to_lhs += 1;
            }
        }

        let shared_domain = shared_lhs_to_rhs.max(shared_rhs_to_lhs);
        if overlap >= MIN_DUPLICATE_ARMS
            && shared_domain > 0
            && overlap * 100 >= shared_domain * MIN_MIRROR_VARIANT_AGREEMENT_PCT
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

fn enum_qual_compatible(
    qa: &Option<EnumQualifier>,
    qb: &Option<EnumQualifier>,
    site_a: &MatchSite,
    site_b: &MatchSite,
) -> bool {
    match (qa, qb) {
        (Some(a), Some(b)) => a.compatible_with(b),
        (None, None) => true,
        (Some(q), None) => site_b.glob_enums.iter().any(|g| g.compatible_with(q)) || site_b.has_unscoped_glob,
        (None, Some(q)) => site_a.glob_enums.iter().any(|g| g.compatible_with(q)) || site_a.has_unscoped_glob,
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
        ) => va == vb && enum_qual_compatible(qa, qb, site_x, site_y),
        _ => false,
    }
}
