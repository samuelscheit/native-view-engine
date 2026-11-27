//! Pattern extraction, structured `variant_head` construction, canonical path/literal formatting,
//! and shared placeholder / discriminant helpers.
//!
//! Scope note: `Pat::Slice`, `Pat::Range`, and `Pat::Const` are intentionally excluded from
//! enum-variant arm expansion because they match slice/range/const patterns rather than enum
//! variant heads (though `Pat::Slice`/`Pat::Range` are still normalized inside `if let` / nested
//! `match` bodies via `normalize_helpers::norm_binding_pat`).

use std::collections::BTreeSet;

use syn::{Lit, Pat};

use super::scope::{
    is_screaming_snake_const, is_type_ident, is_unqualified_const_ident, EnumQualifier,
    ImportScope,
};

const PRELUDE_NON_DOMAIN_ENUMS: &[&str] = &[
    "Option",
    "Result",
    "Ordering",
    "ControlFlow",
    "Poll",
    "Cow",
];

const MAX_TUPLE_OR_EXPANSION: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MirrorAtom {
    Str(String),
    Int(i128),
    Char(char),
    Bool(bool),
    Variant {
        qualifier: Option<EnumQualifier>,
        variant: String,
    },
}

impl MirrorAtom {
    pub(super) fn is_variant(&self) -> bool {
        matches!(self, Self::Variant { .. })
    }

    pub(super) fn key_ignoring_qualifier(&self) -> MirrorKey {
        match self {
            Self::Str(s) => MirrorKey::Str(s.clone()),
            Self::Int(n) => MirrorKey::Int(*n),
            Self::Char(c) => MirrorKey::Char(*c),
            Self::Bool(b) => MirrorKey::Bool(*b),
            Self::Variant { variant, .. } => MirrorKey::Variant(variant.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MirrorKey {
    Str(String),
    Int(i128),
    Char(char),
    Bool(bool),
    Variant(String),
}

#[derive(Debug, Clone)]
pub(super) struct ExtractedPat {
    pub enum_qualifier: Option<EnumQualifier>,
    pub tuple_qualifiers: Option<Vec<Option<EnumQualifier>>>,
    pub is_enum_variant: bool,
    /// Structured head ignoring payload bindings & guards (e.g. `"Struct"`, `"lit:\"foo\""`,
    /// `"(HybridDefaultValue::Bool,_)"`), computed at extraction time so literals or tuples never collapse.
    pub variant_head: String,
    pub pat_key: String,
    pub mirror_atom: Option<MirrorAtom>,
    pub bindings: Vec<String>,
}

pub(super) enum PatternExpansion {
    Ok(Vec<ExtractedPat>),
    Overflow,
}

pub(super) fn is_catchall_pat(pat: &Pat) -> bool {
    match pat {
        Pat::Wild(_) => true,
        Pat::Ident(pi) => {
            pi.subpat.is_none()
                && (pi.by_ref.is_some()
                    || pi.mutability.is_some()
                    || !pi
                        .ident
                        .to_string()
                        .starts_with(|c: char| c.is_ascii_uppercase()))
        }
        Pat::Paren(p) => is_catchall_pat(&p.pat),
        Pat::Reference(r) => is_catchall_pat(&r.pat),
        _ => false,
    }
}

pub(super) fn extract_arm_patterns(
    pat: &Pat,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Vec<ExtractedPat> {
    match extract_arm_patterns_checked(pat, scope, self_ty) {
        PatternExpansion::Ok(v) => v,
        PatternExpansion::Overflow => Vec::new(),
    }
}

pub(super) fn extract_arm_patterns_checked(
    pat: &Pat,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> PatternExpansion {
    match pat {
        Pat::Or(or) => {
            let mut out = Vec::new();
            for c in &or.cases {
                match extract_arm_patterns_checked(c, scope, self_ty) {
                    PatternExpansion::Ok(v) => out.extend(v),
                    PatternExpansion::Overflow => return PatternExpansion::Overflow,
                }
            }
            PatternExpansion::Ok(out)
        }
        Pat::Paren(p) => extract_arm_patterns_checked(&p.pat, scope, self_ty),
        Pat::Reference(r) => extract_arm_patterns_checked(&r.pat, scope, self_ty),
        Pat::Type(t) => extract_arm_patterns_checked(&t.pat, scope, self_ty),
        Pat::Lit(lit) => PatternExpansion::Ok(extract_lit_pat(&lit.lit).into_iter().collect()),
        Pat::Path(p) => PatternExpansion::Ok(
            extract_path_variant(&p.path, scope, self_ty)
                .into_iter()
                .collect(),
        ),
        Pat::Ident(pi) => extract_ident_pat(pi, scope, self_ty),
        Pat::TupleStruct(ts) => extract_tuple_struct_pat(ts, scope, self_ty),
        Pat::Struct(ps) => PatternExpansion::Ok(
            extract_struct_pat(ps, scope, self_ty).into_iter().collect(),
        ),
        Pat::Tuple(tup) => extract_tuple_pat(tup, scope, self_ty),
        _ => PatternExpansion::Ok(Vec::new()),
    }
}

fn extract_lit_pat(lit: &Lit) -> Option<ExtractedPat> {
    let (key, atom) = lit_to_key_and_atom(lit, false)?;
    let full = format!("lit:{key}");
    Some(ExtractedPat {
        enum_qualifier: None,
        tuple_qualifiers: None,
        is_enum_variant: false,
        variant_head: full.clone(),
        pat_key: full,
        mirror_atom: Some(atom),
        bindings: Vec::new(),
    })
}

fn extract_ident_pat(
    pi: &syn::PatIdent,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> PatternExpansion {
    if let Some((_, sub)) = &pi.subpat {
        let outer_name = pi.ident.to_string();
        let mut inner = match extract_arm_patterns_checked(sub, scope, self_ty) {
            PatternExpansion::Ok(v) => v,
            PatternExpansion::Overflow => return PatternExpansion::Overflow,
        };
        for item in &mut inner {
            item.pat_key = shift_pat_placeholders(&item.pat_key, 1);
            item.bindings.insert(0, outer_name.clone());
        }
        return PatternExpansion::Ok(inner);
    }
    if pi.by_ref.is_some() || pi.mutability.is_some() {
        return PatternExpansion::Ok(Vec::new());
    }
    let name = pi.ident.to_string();
    let Some((qual, variant, _)) = scope.resolve_unqualified_variant(&name) else {
        return PatternExpansion::Ok(Vec::new());
    };
    PatternExpansion::Ok(vec![ExtractedPat {
        enum_qualifier: qual.clone(),
        tuple_qualifiers: None,
        is_enum_variant: true,
        variant_head: variant.clone(),
        pat_key: variant.clone(),
        mirror_atom: Some(MirrorAtom::Variant {
            qualifier: qual,
            variant,
        }),
        bindings: Vec::new(),
    }])
}

fn extract_path_variant(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<ExtractedPat> {
    let (qual, variant) = split_enum_variant_path(path, scope, self_ty)?;
    Some(ExtractedPat {
        enum_qualifier: qual.clone(),
        tuple_qualifiers: None,
        is_enum_variant: true,
        variant_head: variant.clone(),
        pat_key: variant.clone(),
        mirror_atom: Some(MirrorAtom::Variant {
            qualifier: qual,
            variant,
        }),
        bindings: Vec::new(),
    })
}

fn extract_tuple_struct_pat(
    ts: &syn::PatTupleStruct,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> PatternExpansion {
    if let Some(last) = ts.path.segments.last()
        && matches!(last.ident.to_string().as_str(), "Some" | "Ok")
        && ts.elems.len() == 1
        && let Some(inner) = ts.elems.first()
    {
        return extract_arm_patterns_checked(inner, scope, self_ty);
    }
    let Some((qual, variant)) = split_enum_variant_path(&ts.path, scope, self_ty) else {
        return PatternExpansion::Ok(Vec::new());
    };
    let mut bindings = Vec::new();
    let parts: Vec<String> = ts
        .elems
        .iter()
        .map(|e| norm_subpat(e, scope, self_ty, &mut bindings))
        .collect();
    PatternExpansion::Ok(vec![ExtractedPat {
        enum_qualifier: qual,
        tuple_qualifiers: None,
        is_enum_variant: true,
        variant_head: variant.clone(),
        pat_key: format!("{variant}({})", parts.join(",")),
        mirror_atom: None,
        bindings,
    }])
}

fn extract_struct_pat(
    ps: &syn::PatStruct,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<ExtractedPat> {
    let (qual, variant) = split_enum_variant_path(&ps.path, scope, self_ty)?;
    let mut fields: Vec<(String, &Pat)> = ps
        .fields
        .iter()
        .map(|fp| (member_to_string(&fp.member), &*fp.pat))
        .collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));

    let mut bindings = Vec::new();
    let mut parts = Vec::new();
    for (fname, fpat) in fields {
        let shape = norm_subpat(fpat, scope, self_ty, &mut bindings);
        parts.push(format!("{fname}:{shape}"));
    }
    if ps.rest.is_some() {
        parts.push("..".to_string());
    }
    Some(ExtractedPat {
        enum_qualifier: qual,
        tuple_qualifiers: None,
        is_enum_variant: true,
        variant_head: variant.clone(),
        pat_key: format!("{variant}{{{}}}", parts.join(",")),
        mirror_atom: None,
        bindings,
    })
}

fn extract_tuple_pat(
    tup: &syn::PatTuple,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> PatternExpansion {
    #[derive(Clone)]
    struct ElemChoice {
        is_enum: bool,
        qual: Option<EnumQualifier>,
        head: String,
        key: String,
        bindings: Vec<String>,
    }

    let mut per_elem: Vec<Vec<ElemChoice>> = Vec::with_capacity(tup.elems.len());
    for elem in &tup.elems {
        if matches!(elem, Pat::Wild(_)) {
            per_elem.push(vec![ElemChoice {
                is_enum: false,
                qual: None,
                head: "_".to_string(),
                key: "_".to_string(),
                bindings: Vec::new(),
            }]);
            continue;
        }
        let sub = match extract_arm_patterns_checked(elem, scope, self_ty) {
            PatternExpansion::Ok(v) => v,
            PatternExpansion::Overflow => return PatternExpansion::Overflow,
        };
        if !sub.is_empty() {
            per_elem.push(
                sub.into_iter()
                    .map(|s| {
                        let head = match &s.enum_qualifier {
                            Some(q) => format!("{}::{}", q.display_name(), s.variant_head),
                            None => s.variant_head,
                        };
                        ElemChoice {
                            is_enum: s.is_enum_variant,
                            qual: s.enum_qualifier,
                            head,
                            key: s.pat_key,
                            bindings: s.bindings,
                        }
                    })
                    .collect(),
            );
        } else {
            let mut b = Vec::new();
            let k = norm_subpat(elem, scope, self_ty, &mut b);
            let head = if b.is_empty() {
                k.clone()
            } else {
                "_".to_string()
            };
            per_elem.push(vec![ElemChoice {
                is_enum: false,
                qual: None,
                head,
                key: k,
                bindings: b,
            }]);
        }
    }

    // Bounded Cartesian expansion of `Pat::Or` inside tuple elements; overflow skips the site.
    let mut combos: Vec<Vec<ElemChoice>> = vec![Vec::new()];
    for choices in per_elem {
        if combos.len().saturating_mul(choices.len()) > MAX_TUPLE_OR_EXPANSION {
            return PatternExpansion::Overflow;
        }
        let mut next = Vec::with_capacity(combos.len() * choices.len());
        for existing in &combos {
            for choice in &choices {
                let mut row = existing.clone();
                row.push(choice.clone());
                next.push(row);
            }
        }
        combos = next;
        if combos.is_empty() {
            return PatternExpansion::Ok(Vec::new());
        }
    }

    let mut out = Vec::new();
    for combo in combos {
        if !combo.iter().any(|c| c.is_enum) {
            continue;
        }
        let mut quals = Vec::with_capacity(combo.len());
        let mut heads = Vec::with_capacity(combo.len());
        let mut keys = Vec::with_capacity(combo.len());
        let mut bindings = Vec::new();
        for c in combo {
            quals.push(c.qual);
            heads.push(c.head);
            let offset = bindings.len();
            keys.push(shift_pat_placeholders(&c.key, offset));
            bindings.extend(c.bindings);
        }
        out.push(ExtractedPat {
            enum_qualifier: None,
            tuple_qualifiers: Some(quals),
            is_enum_variant: true,
            variant_head: format!("({})", heads.join(",")),
            pat_key: format!("({})", keys.join(",")),
            mirror_atom: None,
            bindings,
        });
    }
    PatternExpansion::Ok(out)
}

/// Shared `$p<N>` placeholder rewriter used by both `shift_pat_placeholders` and `NormCtx::finalize_pat_key`.
pub(super) fn rewrite_pat_placeholders(s: &str, mut on_idx: impl FnMut(usize, &mut String)) -> String {
    if !s.contains("$p") {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 2 < bytes.len()
            && bytes[i] == b'$'
            && bytes[i + 1] == b'p'
            && bytes[i + 2].is_ascii_digit()
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if let Ok(idx) = s[i + 2..j].parse::<usize>() {
                on_idx(idx, &mut out);
                i = j;
                continue;
            }
        }
        if let Some(ch) = s[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

pub(super) fn shift_pat_placeholders(s: &str, offset: usize) -> String {
    if offset == 0 {
        return s.to_string();
    }
    rewrite_pat_placeholders(s, |idx, out| {
        out.push_str(&format!("$p{}", idx + offset));
    })
}

fn norm_subpat(
    pat: &Pat,
    scope: &ImportScope,
    self_ty: Option<&str>,
    bindings: &mut Vec<String>,
) -> String {
    match pat {
        Pat::Wild(_) => "_".to_string(),
        Pat::Rest(_) => "..".to_string(),
        Pat::Paren(p) => norm_subpat(&p.pat, scope, self_ty, bindings),
        Pat::Reference(r) => norm_subpat(&r.pat, scope, self_ty, bindings),
        Pat::Lit(l) => lit_to_key_and_atom(&l.lit, false)
            .map(|(k, _)| k)
            .unwrap_or_else(|| "lit".to_string()),
        Pat::Ident(pi) => {
            let name = pi.ident.to_string();
            if pi.by_ref.is_none()
                && pi.mutability.is_none()
                && pi.subpat.is_none()
                && is_type_ident(&name)
                && !is_unqualified_const_ident(&name)
            {
                return name;
            }
            let idx = bindings.len();
            bindings.push(name);
            if let Some((_, sub)) = &pi.subpat {
                format!("$p{idx}@{}", norm_subpat(sub, scope, self_ty, bindings))
            } else {
                format!("$p{idx}")
            }
        }
        Pat::Path(p) => canonical_path(&p.path, scope, self_ty),
        Pat::Tuple(t) => {
            let inner: Vec<String> = t
                .elems
                .iter()
                .map(|e| norm_subpat(e, scope, self_ty, bindings))
                .collect();
            format!("({})", inner.join(","))
        }
        Pat::TupleStruct(ts) => {
            let name = canonical_path(&ts.path, scope, self_ty);
            let inner: Vec<String> = ts
                .elems
                .iter()
                .map(|e| norm_subpat(e, scope, self_ty, bindings))
                .collect();
            format!("{name}({})", inner.join(","))
        }
        Pat::Struct(ps) => {
            let name = canonical_path(&ps.path, scope, self_ty);
            let mut fields: Vec<(String, &Pat)> = ps
                .fields
                .iter()
                .map(|fp| (member_to_string(&fp.member), &*fp.pat))
                .collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            let mut parts: Vec<String> = fields
                .into_iter()
                .map(|(f, p)| format!("{f}:{}", norm_subpat(p, scope, self_ty, bindings)))
                .collect();
            if ps.rest.is_some() {
                parts.push("..".to_string());
            }
            format!("{name}{{{}}}", parts.join(","))
        }
        _ => "_".to_string(),
    }
}

pub(super) fn split_enum_variant_path(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<(Option<EnumQualifier>, String)> {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs.len() == 1 {
        let (q, v, _) = scope.resolve_unqualified_variant(&segs[0])?;
        return Some((q, v));
    }
    let leaf = segs.last()?.clone();
    if !is_type_ident(&leaf) || is_screaming_snake_const(&leaf) {
        return None;
    }
    let type_segs = &segs[..segs.len() - 1];
    let qual = scope.resolve_type_segments(type_segs, self_ty)?;
    if PRELUDE_NON_DOMAIN_ENUMS.contains(&qual.type_name.as_str()) {
        return None;
    }
    Some((Some(qual), leaf))
}

pub(super) fn lit_to_key_and_atom(lit: &Lit, neg: bool) -> Option<(String, MirrorAtom)> {
    match lit {
        Lit::Str(s) if !neg => {
            let v = s.value();
            Some((format!("{v:?}"), MirrorAtom::Str(v)))
        }
        Lit::Int(i) => {
            let v = i.base10_parse::<i128>().ok()?;
            let signed = if neg { -v } else { v };
            Some((signed.to_string(), MirrorAtom::Int(signed)))
        }
        Lit::Char(c) if !neg => {
            let v = c.value();
            Some((format!("{v:?}"), MirrorAtom::Char(v)))
        }
        Lit::Float(f) => {
            let v = f.base10_parse::<f64>().ok()?;
            let signed = if neg { -v } else { v };
            Some((format!("f:{signed}"), MirrorAtom::Str(format!("f:{signed}"))))
        }
        Lit::Bool(b) if !neg => {
            let s = if b.value { "true" } else { "false" };
            Some((s.to_string(), MirrorAtom::Bool(b.value)))
        }
        Lit::Byte(b) if !neg => {
            let v = i128::from(b.value());
            Some((format!("b:{v}"), MirrorAtom::Int(v)))
        }
        Lit::ByteStr(bs) if !neg => {
            let s = format!("b:{:?}", bs.value());
            Some((s.clone(), MirrorAtom::Str(s)))
        }
        _ => None,
    }
}

pub(super) fn canonical_path(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> String {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.is_empty() {
        return String::new();
    }
    if segs.len() == 1 {
        let single = &segs[0];
        if single == "Self"
            && let Some(ty) = self_ty
        {
            return scope.resolve_single_type_ident(ty).canonical_key();
        }
        if let Some((Some(q), v, _)) = scope.resolve_unqualified_variant(single) {
            return format!("{}::{v}", q.canonical_key());
        }
        if let Some(q) = scope.type_aliases.get(single) {
            return q.canonical_key();
        }
        return single.clone();
    }
    let last = &segs[segs.len() - 1];
    let parent_segs = &segs[..segs.len() - 1];
    if let Some(q) = scope.resolve_type_segments(parent_segs, self_ty) {
        return format!("{}::{last}", q.canonical_key());
    }
    if let Some(q) = scope.resolve_type_segments(&segs, self_ty) {
        return q.canonical_key();
    }
    let mut trimmed = segs;
    while trimmed
        .first()
        .is_some_and(|s| matches!(s.as_str(), "crate" | "self" | "super"))
    {
        trimmed.remove(0);
    }
    if trimmed.len() >= 2 {
        format!("{}::{}", trimmed[trimmed.len() - 2], trimmed[trimmed.len() - 1])
    } else {
        trimmed.into_iter().next().unwrap_or_default()
    }
}

pub(super) fn member_to_string(m: &syn::Member) -> String {
    match m {
        syn::Member::Named(id) => id.to_string(),
        syn::Member::Unnamed(idx) => idx.index.to_string(),
    }
}

/// Shared compact `0..N` integer-discriminant check used by both `converters.rs` and `legacy_tables.rs`.
pub(super) fn is_compact_discriminant_set(vals: &[i128], min_arms: usize) -> bool {
    let unique: BTreeSet<i128> = vals.iter().copied().collect();
    if unique.len() < min_arms {
        return false;
    }
    let min = *unique.first().unwrap_or(&0);
    let max = *unique.last().unwrap_or(&0);
    min >= 0 && max <= 255 && (max - min) <= (unique.len() as i128 * 2 + 4)
}
