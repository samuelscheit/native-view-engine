//! Cross-file Rust AST checks for duplicate `pub enum`s, redundant NVE->NVE enum/discriminant
//! converters, mirrored `<->` match tables, compact discriminant matches, and core enum<->str tables.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{Expr, Item, Lit, Pat, Type, Visibility};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

const EXTERNAL_SDK_ROOTS: &[&str] = &[
    "taffy",
    "skia_safe",
    "web_sys",
    "accesskit",
    "winit",
    "parley",
    "cosmic_text",
    "vk",
    "ash",
    "windows",
    "objc2",
    "gtk4",
];

const EXTERNAL_BRIDGE_FN_PREFIXES: &[&str] = &[
    "to_taffy_",
    "from_taffy_",
    "to_skia_",
    "from_skia_",
    "to_web_",
    "to_accesskit_",
    "to_winit_",
    "to_parley_",
    "to_cosmic_",
];

pub fn scan_duplicate_enums_and_converters(
    core_files: &[(String, &str, &syn::File)],
    backend_files: &[(String, &str, &syn::File)],
    out: &mut Vec<Violation>,
) {
    let mut all_enum_defs: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    let mut core_enum_names = BTreeSet::new();
    let mut internal_enum_names = BTreeSet::new();

    for (is_core, files) in [(true, core_files), (false, backend_files)] {
        for (rel, _, ast) in files {
            collect_pub_enums(
                rel,
                &ast.items,
                is_core,
                &mut all_enum_defs,
                &mut core_enum_names,
                &mut internal_enum_names,
            );
        }
    }

    for (name, locs) in &all_enum_defs {
        if !core_enum_names.contains(name) {
            continue;
        }
        let distinct: BTreeSet<&str> = locs.iter().map(|(f, _)| f.as_str()).collect();
        if distinct.len() >= 2 {
            let summary = locs
                .iter()
                .map(|(f, l)| format!("{f}:{l}"))
                .collect::<Vec<_>>()
                .join(", ");
            for (f, l) in locs {
                out.push(Violation::new(
                    f,
                    *l,
                    format!("pub enum {name} (duplicates: {summary})"),
                    format!(
                        "Duplicate internal `pub enum {name}` across {} core/backend files; collapse into one canonical type",
                        distinct.len()
                    ),
                ));
            }
        }
    }

    for (rel, raw, ast) in core_files.iter().chain(backend_files.iter()) {
        scan_converters_in_items(rel, raw, &ast.items, &internal_enum_names, out);
    }
}

fn collect_pub_enums(
    rel: &str,
    items: &[Item],
    is_core: bool,
    all_defs: &mut BTreeMap<String, Vec<(String, usize)>>,
    core_names: &mut BTreeSet<String>,
    internal_names: &mut BTreeSet<String>,
) {
    for item in items {
        match item {
            Item::Enum(en) if !is_cfg_test_attr(&en.attrs) && matches!(en.vis, Visibility::Public(_)) => {
                let name = en.ident.to_string();
                let line = en.ident.span().start().line;
                all_defs.entry(name.clone()).or_default().push((rel.to_string(), line));
                if is_core {
                    core_names.insert(name.clone());
                }
                if !name.ends_with("Error") {
                    internal_names.insert(name);
                }
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_pub_enums(rel, sub, is_core, all_defs, core_names, internal_names);
                }
            }
            _ => {}
        }
    }
}

fn scan_converters_in_items(
    rel: &str,
    raw: &str,
    items: &[Item],
    internal_enums: &BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    scan_converters_in_items(rel, raw, sub, internal_enums, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                if let Some((_, trait_path, _)) = &imp.trait_
                    && let Some(seg) = trait_path.segments.last()
                    && seg.ident == "From"
                    && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
                    && let Some(syn::GenericArgument::Type(src_ty)) = args.args.first()
                    && !type_has_external_sdk(src_ty)
                    && !type_has_external_sdk(&imp.self_ty)
                    && let (Some(s), Some(d)) = (last_type_ident(src_ty), last_type_ident(&imp.self_ty))
                    && s != d
                    && internal_enums.contains(&s)
                    && internal_enums.contains(&d)
                {
                    let ln = imp.impl_token.span.start().line;
                    out.push(Violation::new(
                        rel,
                        ln,
                        line_at(raw, ln),
                        format!(
                            "Redundant internal NVE-to-NVE enum converter `impl From<{s}> for {d}`; collapse into one canonical enum"
                        ),
                    ));
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                    {
                        check_converter_sig(rel, raw, &f.sig, internal_enums, out);
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                check_converter_sig(rel, raw, &f.sig, internal_enums, out);
            }
            _ => {}
        }
    }
}

fn check_converter_sig(
    rel: &str,
    raw: &str,
    sig: &syn::Signature,
    internal_enums: &BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    let name = sig.ident.to_string();
    let is_converter = name.starts_with("decode_")
        || name.starts_with("map_")
        || name.starts_with("convert_")
        || name.starts_with("to_")
        || name.starts_with("from_")
        || matches!(name.as_str(), "from_u8" | "from_u32" | "to_u8");
    if !is_converter || EXTERNAL_BRIDGE_FN_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return;
    }
    let syn::ReturnType::Type(_, ret_ty) = &sig.output else {
        return;
    };
    if type_has_external_sdk(ret_ty) {
        return;
    }

    let mut takes_self = false;
    let mut param_tys = Vec::new();
    for input in &sig.inputs {
        match input {
            syn::FnArg::Receiver(_) => takes_self = true,
            syn::FnArg::Typed(pat_ty) => {
                if type_has_external_sdk(&pat_ty.ty) {
                    return;
                }
                param_tys.push(&*pat_ty.ty);
            }
        }
    }

    let ok_ret = unwrap_result_ok(ret_ty);
    if type_is_collection_or_ref(ok_ret) {
        return;
    }

    let mut param_enums = BTreeSet::new();
    for ty in &param_tys {
        collect_internal_enums_in_type(ty, internal_enums, &mut param_enums);
    }
    let mut ret_enums = BTreeSet::new();
    collect_internal_enums_in_type(ok_ret, internal_enums, &mut ret_enums);

    let single_disc = param_tys.len() == 1 && is_discriminant_type(param_tys[0]);
    let any_disc = param_tys.iter().any(|ty| is_discriminant_type(ty));
    let is_self_disc_name = name.starts_with("from_u")
        || name.starts_with("from_i")
        || name.starts_with("from_word")
        || name.starts_with("from_code")
        || name.starts_with("decode_")
        || name.starts_with("map_")
        || name.starts_with("convert_");

    let is_redundant_disc = !takes_self
        && ((single_disc && type_mentions_self(ok_ret) && is_self_disc_name)
            || (!ret_enums.is_empty() && any_disc));
    let is_redundant_e2e = !param_enums.is_empty() && !ret_enums.is_empty() && param_enums != ret_enums;

    if is_redundant_disc || is_redundant_e2e {
        let ln = sig.ident.span().start().line;
        out.push(Violation::new(
            rel,
            ln,
            line_at(raw, ln),
            format!(
                "Manual internal enum/discriminant converter `fn {name}`; use canonical `#[derive(WordEnum)]` type directly"
            ),
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ArmAtom {
    Str(String),
    Int(i128),
    Variant(String),
}

struct FnMatchProfile {
    name: String,
    line: usize,
    pairs: BTreeSet<(ArmAtom, ArmAtom)>,
}

pub fn scan_match_tables(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    let mut fns = Vec::new();
    collect_fn_profiles(&ast.items, &mut fns);

    let mut reported_mirror = BTreeSet::new();
    for a in 0..fns.len() {
        if fns[a].pairs.len() < 3 {
            continue;
        }
        for b in (a + 1)..fns.len() {
            if fns[b].pairs.len() < 3 {
                continue;
            }
            let overlap = fns[a]
                .pairs
                .iter()
                .filter(|(l, r)| l != r && fns[b].pairs.contains(&(r.clone(), l.clone())))
                .count();
            if overlap >= 3 {
                reported_mirror.insert(a);
                reported_mirror.insert(b);
                out.push(Violation::new(
                    rel,
                    fns[a].line,
                    line_at(raw, fns[a].line),
                    format!(
                        "Duplicated bidirectional mapping between `fn {}` (line {}) and `fn {}` (line {}): {overlap} mirrored `match` arms — generate from one source of truth",
                        fns[a].name, fns[a].line, fns[b].name, fns[b].line
                    ),
                ));
            }
        }
    }

    if !rel.starts_with("packages/core/codegen/") {
        for (idx, f) in fns.iter().enumerate() {
            if reported_mirror.contains(&idx) {
                continue;
            }
            let i2v: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Int(n), ArmAtom::Variant(_)) => Some(*n),
                    _ => None,
                })
                .collect();
            let v2i: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Variant(_), ArmAtom::Int(n)) => Some(*n),
                    _ => None,
                })
                .collect();
            let count = if is_compact_discriminant_set(&i2v) {
                i2v.len()
            } else if is_compact_discriminant_set(&v2i) {
                v2i.len()
            } else {
                0
            };
            if count >= 3 {
                out.push(Violation::new(
                    rel,
                    f.line,
                    line_at(raw, f.line),
                    format!(
                        "Manual integer-discriminant `match` in `fn {}` ({count} arms); derive via `#[derive(WordEnum)]` / `#[derive(NveCodec)]`",
                        f.name
                    ),
                ));
            }
        }
    }

    if rel.starts_with("packages/core/tables/") || rel.starts_with("packages/core/components/") {
        for f in &fns {
            let v2s = f
                .pairs
                .iter()
                .filter(|(l, r)| matches!((l, r), (ArmAtom::Variant(_), ArmAtom::Str(_))))
                .count();
            let s2v = f
                .pairs
                .iter()
                .filter(|(l, r)| matches!((l, r), (ArmAtom::Str(_), ArmAtom::Variant(_))))
                .count();
            if v2s >= 3 || s2v >= 3 {
                let count = v2s.max(s2v);
                out.push(Violation::new(
                    rel,
                    f.line,
                    line_at(raw, f.line),
                    format!(
                        "Core function `{}` maps {count} enum variants <-> string literals; move CSS/HTML/ARIA keyword tables to `nve-backend-web`",
                        f.name
                    ),
                ));
            }
        }
    }
}

fn collect_fn_profiles(items: &[Item], out: &mut Vec<FnMatchProfile>) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_fn_profiles(sub, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let skip_trait = imp.trait_.as_ref().is_some_and(|(_, p, _)| {
                    p.segments
                        .last()
                        .is_some_and(|s| s.ident == "Display" || s.ident == "Debug")
                });
                let skip_err = last_type_ident(&imp.self_ty).is_some_and(|n| n.ends_with("Error"));
                if skip_trait || skip_err {
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                        && f.sig.ident != "fmt"
                    {
                        record_fn_profile(&f.sig, &f.block, out);
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) && f.sig.ident != "fmt" => {
                record_fn_profile(&f.sig, &f.block, out);
            }
            _ => {}
        }
    }
}

fn record_fn_profile(sig: &syn::Signature, block: &syn::Block, out: &mut Vec<FnMatchProfile>) {
    struct MatchScan(BTreeSet<(ArmAtom, ArmAtom)>);
    impl<'ast> Visit<'ast> for MatchScan {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            for arm in &m.arms {
                if arm.guard.is_none()
                    && let Some(rhs) = expr_atom(&arm.body)
                {
                    collect_pat_atoms(&arm.pat, &rhs, &mut self.0);
                }
            }
            visit::visit_expr_match(self, m);
        }
    }
    let mut scan = MatchScan(BTreeSet::new());
    scan.visit_block(block);
    if !scan.0.is_empty() {
        out.push(FnMatchProfile {
            name: sig.ident.to_string(),
            line: sig.ident.span().start().line,
            pairs: scan.0,
        });
    }
}

fn collect_pat_atoms(pat: &Pat, rhs: &ArmAtom, out: &mut BTreeSet<(ArmAtom, ArmAtom)>) {
    match pat {
        Pat::Or(or) => or.cases.iter().for_each(|c| collect_pat_atoms(c, rhs, out)),
        _ => {
            if let Some(lhs) = pat_atom(pat) {
                out.insert((lhs, rhs.clone()));
            }
        }
    }
}

fn pat_atom(pat: &Pat) -> Option<ArmAtom> {
    match pat {
        Pat::Lit(lit) => lit_atom(&lit.lit, false),
        Pat::Path(p) => path_variant_atom(&p.path),
        Pat::TupleStruct(ts) => {
            let last = ts.path.segments.last()?.ident.to_string();
            ((last == "Some" || last == "Ok") && ts.elems.len() == 1)
                .then(|| pat_atom(ts.elems.first()?))
                .flatten()
        }
        Pat::Paren(p) => pat_atom(&p.pat),
        _ => None,
    }
}

fn expr_atom(expr: &Expr) -> Option<ArmAtom> {
    match expr {
        Expr::Lit(l) => lit_atom(&l.lit, false),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => match &*u.expr {
            Expr::Lit(l) => lit_atom(&l.lit, true),
            _ => None,
        },
        Expr::Path(p) => path_variant_atom(&p.path),
        Expr::Call(c) => {
            if let Expr::Path(p) = &*c.func
                && let Some(last) = p.path.segments.last()
                && (last.ident == "Some" || last.ident == "Ok")
                && c.args.len() == 1
            {
                return expr_atom(c.args.first()?);
            }
            None
        }
        Expr::Paren(p) => expr_atom(&p.expr),
        Expr::Group(g) => expr_atom(&g.expr),
        _ => None,
    }
}

fn lit_atom(lit: &Lit, neg: bool) -> Option<ArmAtom> {
    match lit {
        Lit::Str(s) if !neg => Some(ArmAtom::Str(s.value())),
        Lit::Int(i) => i
            .base10_parse::<i128>()
            .ok()
            .map(|v| ArmAtom::Int(if neg { -v } else { v })),
        _ => None,
    }
}

fn path_variant_atom(path: &syn::Path) -> Option<ArmAtom> {
    if path.segments.len() < 2
        || path
            .segments
            .iter()
            .any(|s| EXTERNAL_SDK_ROOTS.contains(&s.ident.to_string().as_str()))
    {
        return None;
    }
    let leaf = path.segments.last()?.ident.to_string();
    leaf.starts_with(|c: char| c.is_ascii_uppercase())
        .then_some(ArmAtom::Variant(leaf))
}

fn is_compact_discriminant_set(vals: &[i128]) -> bool {
    let unique: BTreeSet<i128> = vals.iter().copied().collect();
    if unique.len() < 3 {
        return false;
    }
    let min = *unique.first().unwrap_or(&0);
    let max = *unique.last().unwrap_or(&0);
    min >= 0 && max <= 255 && (max - min) <= (unique.len() as i128 * 2 + 4)
}

fn is_discriminant_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.qself.is_none() && tp.path.segments.len() == 1
        && matches!(tp.path.segments[0].ident.to_string().as_str(), "u8" | "u16" | "u32" | "i32"))
}

fn unwrap_result_ok(ty: &Type) -> &Type {
    if let Type::Path(tp) = ty
        && let Some(last) = tp.path.segments.last()
        && last.ident == "Result"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(ok_ty)) = args.args.first()
    {
        return ok_ty;
    }
    ty
}

fn type_is_collection_or_ref(ty: &Type) -> bool {
    match ty {
        Type::Reference(_) | Type::Slice(_) | Type::Array(_) => true,
        Type::Path(tp) => tp.path.segments.last().is_some_and(|s| s.ident == "Vec"),
        _ => false,
    }
}

fn type_has_external_sdk(ty: &Type) -> bool {
    struct ExtScan(bool);
    impl<'ast> Visit<'ast> for ExtScan {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if p.segments.len() >= 2
                && p.segments
                    .iter()
                    .any(|s| EXTERNAL_SDK_ROOTS.contains(&s.ident.to_string().as_str()))
            {
                self.0 = true;
            }
            visit::visit_path(self, p);
        }
    }
    let mut s = ExtScan(false);
    s.visit_type(ty);
    s.0
}

fn type_mentions_self(ty: &Type) -> bool {
    struct SelfScan(bool);
    impl<'ast> Visit<'ast> for SelfScan {
        fn visit_ident(&mut self, i: &'ast syn::Ident) {
            if i == "Self" {
                self.0 = true;
            }
        }
    }
    let mut s = SelfScan(false);
    s.visit_type(ty);
    s.0
}

fn collect_internal_enums_in_type(
    ty: &Type,
    internal_enums: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    struct EnumScan<'a>(&'a BTreeSet<String>, &'a mut BTreeSet<String>);
    impl<'ast> Visit<'ast> for EnumScan<'_> {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if let Some(last) = p.segments.last() {
                let name = last.ident.to_string();
                if self.0.contains(&name) {
                    self.1.insert(name);
                }
            }
            visit::visit_path(self, p);
        }
    }
    EnumScan(internal_enums, out).visit_type(ty);
}
