//! Pattern extraction, `use` import resolution, and canonical path/literal helpers for `ast_enums`.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Item, Lit, Pat, UseTree};

use crate::is_cfg_test_attr;

pub const EXTERNAL_SDK_ROOTS: &[&str] = &[
    "taffy",
    "skia_safe",
    "web_sys",
    "accesskit",
    "winit",
    "parley",
    "cosmic_text",
    "vk",
    "ash",
    "ash_vk",
    "windows",
    "objc2",
    "gtk4",
];

const PRELUDE_NON_DOMAIN_ENUMS: &[&str] = &[
    "Option",
    "Result",
    "Ordering",
    "ControlFlow",
    "Poll",
    "Cow",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MirrorAtom {
    Str(String),
    Int(i128),
    Char(char),
    Variant {
        qualifier: Option<String>,
        variant: String,
    },
}

impl MirrorAtom {
    pub(super) fn is_variant(&self) -> bool {
        matches!(self, Self::Variant { .. })
    }

    pub(super) fn variant_key(&self) -> Option<&str> {
        match self {
            Self::Variant { variant, .. } => Some(variant.as_str()),
            _ => None,
        }
    }

    pub(super) fn key_ignoring_qualifier(&self) -> MirrorKey {
        match self {
            Self::Str(s) => MirrorKey::Str(s.clone()),
            Self::Int(n) => MirrorKey::Int(*n),
            Self::Char(c) => MirrorKey::Char(*c),
            Self::Variant { variant, .. } => MirrorKey::Variant(variant.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MirrorKey {
    Str(String),
    Int(i128),
    Char(char),
    Variant(String),
}

#[derive(Debug, Clone, Default)]
pub(super) struct ImportScope {
    /// Imported type name -> canonical SDK-prefixed name (e.g. `AlignItems` -> `taffy::AlignItems`).
    pub sdk_types: BTreeMap<String, String>,
    /// Explicitly imported enum variant -> `(EnumType, VariantName)`.
    pub variant_imports: BTreeMap<String, (String, String)>,
    /// `use path::Enum::*;` -> `Enum`.
    pub glob_enums: BTreeSet<String>,
    /// True if scope has a wildcard import from a module (`use super::*;`, `use crate::foo::*;`).
    pub has_unscoped_glob: bool,
}

impl ImportScope {
    pub(super) fn ingest_items(&mut self, items: &[Item]) {
        for item in items {
            if let Item::Use(u) = item
                && !is_cfg_test_attr(&u.attrs)
            {
                let mut prefix = Vec::new();
                self.ingest_tree(&u.tree, &mut prefix);
            }
        }
    }

    pub(super) fn ingest_tree(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.ingest_tree(&p.tree, prefix);
                prefix.pop();
            }
            UseTree::Group(g) => {
                for item in &g.items {
                    self.ingest_tree(item, prefix);
                }
            }
            UseTree::Name(n) => {
                let ident = n.ident.to_string();
                self.record_import(prefix, &ident, &ident);
            }
            UseTree::Rename(r) => {
                let orig = r.ident.to_string();
                let alias = r.rename.to_string();
                self.record_import(prefix, &orig, &alias);
            }
            UseTree::Glob(_) => {
                if let Some(last) = prefix.last()
                    && is_type_ident(last)
                {
                    let qual = canonical_qualifier_from_segments(prefix);
                    self.glob_enums.insert(qual);
                } else {
                    self.has_unscoped_glob = true;
                }
            }
        }
    }

    fn record_import(&mut self, prefix: &[String], orig: &str, alias: &str) {
        if orig == "self" || !is_type_ident(orig) {
            return;
        }
        let sdk_root = prefix
            .iter()
            .find(|s| EXTERNAL_SDK_ROOTS.contains(&s.as_str()))
            .cloned();
        if let Some(parent) = prefix.last()
            && is_type_ident(parent)
            && !is_screaming_const(orig)
        {
            let enum_qual = match sdk_root {
                Some(sdk) => format!("{sdk}::{parent}"),
                None => parent.clone(),
            };
            self.variant_imports
                .insert(alias.to_string(), (enum_qual, orig.to_string()));
        } else if let Some(sdk) = sdk_root {
            self.sdk_types
                .insert(alias.to_string(), format!("{sdk}::{orig}"));
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ExtractedPat {
    pub enum_qualifier: Option<String>,
    pub is_enum_variant: bool,
    pub pat_key: String,
    pub mirror_atom: Option<MirrorAtom>,
    pub bindings: Vec<String>,
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
    match pat {
        Pat::Or(or) => or
            .cases
            .iter()
            .flat_map(|c| extract_arm_patterns(c, scope, self_ty))
            .collect(),
        Pat::Paren(p) => extract_arm_patterns(&p.pat, scope, self_ty),
        Pat::Reference(r) => extract_arm_patterns(&r.pat, scope, self_ty),
        Pat::Type(t) => extract_arm_patterns(&t.pat, scope, self_ty),
        Pat::Lit(lit) => extract_lit_pat(&lit.lit, false).into_iter().collect(),
        Pat::Path(p) => extract_path_variant(&p.path, scope, self_ty)
            .into_iter()
            .collect(),
        Pat::Ident(pi) => extract_ident_variant(pi, scope).into_iter().collect(),
        Pat::TupleStruct(ts) => extract_tuple_struct_pat(ts, scope, self_ty),
        Pat::Struct(ps) => extract_struct_pat(ps, scope, self_ty).into_iter().collect(),
        Pat::Tuple(tup) => extract_tuple_pat(tup, scope, self_ty).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn extract_lit_pat(lit: &Lit, neg: bool) -> Option<ExtractedPat> {
    let (key, atom) = lit_to_key_and_atom(lit, neg)?;
    Some(ExtractedPat {
        enum_qualifier: None,
        is_enum_variant: false,
        pat_key: format!("lit:{key}"),
        mirror_atom: Some(atom),
        bindings: Vec::new(),
    })
}

fn extract_ident_variant(pi: &syn::PatIdent, scope: &ImportScope) -> Option<ExtractedPat> {
    if pi.by_ref.is_some() || pi.mutability.is_some() || pi.subpat.is_some() {
        return None;
    }
    let name = pi.ident.to_string();
    if matches!(name.as_str(), "None" | "Some" | "Ok" | "Err" | "true" | "false") {
        return None;
    }
    if let Some((enum_qual, orig_var)) = scope.variant_imports.get(&name) {
        return Some(ExtractedPat {
            enum_qualifier: Some(enum_qual.clone()),
            is_enum_variant: true,
            pat_key: orig_var.clone(),
            mirror_atom: Some(MirrorAtom::Variant {
                qualifier: Some(enum_qual.clone()),
                variant: orig_var.clone(),
            }),
            bindings: Vec::new(),
        });
    }
    if !is_type_ident(&name) || is_screaming_const(&name) {
        return None;
    }
    let qual = (scope.glob_enums.len() == 1)
        .then(|| scope.glob_enums.iter().next().cloned())
        .flatten();
    Some(ExtractedPat {
        enum_qualifier: qual.clone(),
        is_enum_variant: true,
        pat_key: name.clone(),
        mirror_atom: Some(MirrorAtom::Variant {
            qualifier: qual,
            variant: name,
        }),
        bindings: Vec::new(),
    })
}

fn extract_path_variant(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<ExtractedPat> {
    let (qual, variant) = split_enum_variant_path(path, scope, self_ty)?;
    Some(ExtractedPat {
        enum_qualifier: qual.clone(),
        is_enum_variant: true,
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
) -> Vec<ExtractedPat> {
    if let Some(last) = ts.path.segments.last()
        && matches!(last.ident.to_string().as_str(), "Some" | "Ok")
        && ts.elems.len() == 1
        && let Some(inner) = ts.elems.first()
    {
        return extract_arm_patterns(inner, scope, self_ty);
    }
    let Some((qual, variant)) = split_enum_variant_path(&ts.path, scope, self_ty) else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    let parts: Vec<String> = ts
        .elems
        .iter()
        .map(|e| norm_subpat(e, scope, self_ty, &mut bindings))
        .collect();
    vec![ExtractedPat {
        enum_qualifier: qual,
        is_enum_variant: true,
        pat_key: format!("{variant}({})", parts.join(",")),
        mirror_atom: None,
        bindings,
    }]
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
        is_enum_variant: true,
        pat_key: format!("{variant}{{{}}}", parts.join(",")),
        mirror_atom: None,
        bindings,
    })
}

fn extract_tuple_pat(
    tup: &syn::PatTuple,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<ExtractedPat> {
    let mut any_enum = false;
    let mut quals = Vec::new();
    let mut keys = Vec::new();
    let mut bindings = Vec::new();
    for elem in &tup.elems {
        if matches!(elem, Pat::Wild(_)) {
            quals.push("_".to_string());
            keys.push("_".to_string());
            continue;
        }
        let sub = extract_arm_patterns(elem, scope, self_ty);
        if sub.len() == 1 {
            let s = &sub[0];
            if s.is_enum_variant {
                any_enum = true;
            }
            quals.push(s.enum_qualifier.clone().unwrap_or_else(|| "_".to_string()));
            let offset = bindings.len();
            let shifted = shift_pat_placeholders(&s.pat_key, offset);
            bindings.extend(s.bindings.iter().cloned());
            keys.push(shifted);
        } else {
            quals.push("_".to_string());
            keys.push(norm_subpat(elem, scope, self_ty, &mut bindings));
        }
    }
    if !any_enum {
        return None;
    }
    Some(ExtractedPat {
        enum_qualifier: Some(format!("({})", quals.join(","))),
        is_enum_variant: true,
        pat_key: format!("({})", keys.join(",")),
        mirror_atom: None,
        bindings,
    })
}

fn shift_pat_placeholders(s: &str, offset: usize) -> String {
    if offset == 0 || !s.contains("$p") {
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
                out.push_str(&format!("$p{}", idx + offset));
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
                && !is_screaming_const(&name)
            {
                return name;
            }
            let idx = bindings.len();
            bindings.push(name);
            format!("$p{idx}")
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
        _ => "_".to_string(),
    }
}

pub(super) fn split_enum_variant_path(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<(Option<String>, String)> {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs.len() == 1 {
        let name = &segs[0];
        if matches!(name.as_str(), "Some" | "None" | "Ok" | "Err") {
            return None;
        }
        if let Some((q, v)) = scope.variant_imports.get(name) {
            return Some((Some(q.clone()), v.clone()));
        }
        if is_type_ident(name) && !is_screaming_const(name) {
            let q = (scope.glob_enums.len() == 1)
                .then(|| scope.glob_enums.iter().next().cloned())
                .flatten();
            return Some((q, name.clone()));
        }
        return None;
    }
    let leaf = segs.last()?.clone();
    if !is_type_ident(&leaf) || is_screaming_const(&leaf) {
        return None;
    }
    let raw_parent = &segs[segs.len() - 2];
    let parent = if raw_parent == "Self" {
        self_ty?.to_string()
    } else if is_type_ident(raw_parent) {
        raw_parent.clone()
    } else {
        return None;
    };
    if PRELUDE_NON_DOMAIN_ENUMS.contains(&parent.as_str()) {
        return None;
    }
    let sdk = segs
        .iter()
        .find(|s| EXTERNAL_SDK_ROOTS.contains(&s.as_str()))
        .cloned()
        .or_else(|| {
            scope
                .sdk_types
                .get(&parent)
                .and_then(|s| s.split("::").next().map(ToString::to_string))
        });
    let qual = match sdk {
        Some(s) => format!("{s}::{parent}"),
        None => parent,
    };
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
            Some((s.to_string(), MirrorAtom::Str(s.to_string())))
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

fn canonical_qualifier_from_segments(segs: &[String]) -> String {
    let last = segs.last().cloned().unwrap_or_default();
    if let Some(sdk) = segs.iter().find(|s| EXTERNAL_SDK_ROOTS.contains(&s.as_str())) {
        format!("{sdk}::{last}")
    } else {
        last
    }
}

pub(super) fn canonical_path(
    path: &syn::Path,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> String {
    let mut segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.first().is_some_and(|s| s == "Self")
        && let Some(ty) = self_ty
    {
        segs[0] = ty.to_string();
    }
    while segs
        .first()
        .is_some_and(|s| matches!(s.as_str(), "crate" | "self" | "super"))
    {
        segs.remove(0);
    }
    if segs.is_empty() {
        return String::new();
    }
    if segs.len() == 1 {
        if let Some(mapped) = scope.sdk_types.get(&segs[0]) {
            return mapped.clone();
        }
        return segs[0].clone();
    }
    let sdk = segs
        .iter()
        .find(|s| EXTERNAL_SDK_ROOTS.contains(&s.as_str()))
        .cloned()
        .or_else(|| {
            scope
                .sdk_types
                .get(&segs[segs.len() - 2])
                .and_then(|s| s.split("::").next().map(ToString::to_string))
        });
    let tail = format!("{}::{}", segs[segs.len() - 2], segs[segs.len() - 1]);
    match sdk {
        Some(s) if !tail.starts_with(&format!("{s}::")) => format!("{s}::{tail}"),
        _ => tail,
    }
}

pub(super) fn member_to_string(m: &syn::Member) -> String {
    match m {
        syn::Member::Named(id) => id.to_string(),
        syn::Member::Unnamed(idx) => idx.index.to_string(),
    }
}

pub(super) fn is_type_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase())
}

pub(super) fn is_screaming_const(s: &str) -> bool {
    s.len() >= 2
        && s.contains('_')
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
