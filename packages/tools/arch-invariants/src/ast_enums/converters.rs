//! Workspace-wide duplicate `pub enum` and internal NVE->NVE enum/discriminant converter checks.
//!
//! Hardened beyond `packages/core` + `packages/backends`:
//! - Scans all production Rust files across `packages/`, `hosts/`, and `examples/`.
//! - Detects both exact-name `pub enum` duplicates AND structural identical-variant-set (`>= 4` unit variants) duplicates.
//! - Detects `impl From`/`impl TryFrom` and body-matching enum/discriminant converters regardless of function name prefix.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{Item, Pat, Type, Visibility};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

use super::fingerprint::MIN_DUPLICATE_ARMS;
use super::patterns::EXTERNAL_SDK_ROOTS;

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

#[derive(Clone)]
struct EnumDefInfo {
    file: String,
    line: usize,
    name: String,
}

pub fn scan_duplicate_enums_and_converters(
    core_files: &[(String, &str, &syn::File)],
    other_files: &[(String, &str, &syn::File)],
    out: &mut Vec<Violation>,
) {
    let mut by_name: BTreeMap<String, Vec<EnumDefInfo>> = BTreeMap::new();
    let mut by_variants: BTreeMap<Vec<String>, Vec<EnumDefInfo>> = BTreeMap::new();
    let mut core_enum_names = BTreeSet::new();
    let mut internal_enum_names = BTreeSet::new();

    for (is_core, files) in [(true, core_files), (false, other_files)] {
        for (rel, _, ast) in files {
            if is_cfg_test_attr(&ast.attrs) {
                continue;
            }
            collect_pub_enums(
                rel,
                &ast.items,
                is_core,
                &mut by_name,
                &mut by_variants,
                &mut core_enum_names,
                &mut internal_enum_names,
            );
        }
    }

    let mut reported_enum_locs = BTreeSet::new();
    for (name, defs) in &by_name {
        if !core_enum_names.contains(name) {
            continue;
        }
        let distinct: BTreeSet<&str> = defs.iter().map(|d| d.file.as_str()).collect();
        if distinct.len() >= 2 {
            let summary = defs
                .iter()
                .map(|d| format!("{}:{}", d.file, d.line))
                .collect::<Vec<_>>()
                .join(", ");
            for d in defs {
                reported_enum_locs.insert((d.file.clone(), d.line));
                out.push(Violation::new(
                    &d.file,
                    d.line,
                    format!("pub enum {name} (duplicates: {summary})"),
                    format!(
                        "Duplicate internal `pub enum {name}` across {} workspace files; collapse into one canonical type",
                        distinct.len()
                    ),
                ));
            }
        }
    }

    for (variants, defs) in &by_variants {
        if variants.len() < MIN_DUPLICATE_ARMS {
            continue;
        }
        let distinct_names: BTreeSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        let distinct_files: BTreeSet<&str> = defs.iter().map(|d| d.file.as_str()).collect();
        if distinct_names.len() >= 2 && distinct_files.len() >= 2 {
            let summary = defs
                .iter()
                .map(|d| format!("{} ({}:{})", d.name, d.file, d.line))
                .collect::<Vec<_>>()
                .join(", ");
            for d in defs {
                if reported_enum_locs.contains(&(d.file.clone(), d.line)) {
                    continue;
                }
                out.push(Violation::new(
                    &d.file,
                    d.line,
                    format!("pub enum {} (structural duplicates: {summary})", d.name),
                    format!(
                        "Structurally identical `pub enum {}` ({} identical unit variants) duplicated across workspace; collapse into one canonical type",
                        d.name,
                        variants.len()
                    ),
                ));
            }
        }
    }

    for (rel, raw, ast) in core_files.iter().chain(other_files.iter()) {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        scan_converters_in_items(rel, raw, &ast.items, &internal_enum_names, out);
    }
}

fn collect_pub_enums(
    rel: &str,
    items: &[Item],
    is_core: bool,
    by_name: &mut BTreeMap<String, Vec<EnumDefInfo>>,
    by_variants: &mut BTreeMap<Vec<String>, Vec<EnumDefInfo>>,
    core_names: &mut BTreeSet<String>,
    internal_names: &mut BTreeSet<String>,
) {
    for item in items {
        match item {
            Item::Enum(en)
                if !is_cfg_test_attr(&en.attrs) && matches!(en.vis, Visibility::Public(_)) =>
            {
                let name = en.ident.to_string();
                let line = en.ident.span().start().line;
                let all_unit = en
                    .variants
                    .iter()
                    .all(|v| matches!(v.fields, syn::Fields::Unit));
                let unit_variants: BTreeSet<String> = if all_unit {
                    en.variants.iter().map(|v| v.ident.to_string()).collect()
                } else {
                    BTreeSet::new()
                };
                let info = EnumDefInfo {
                    file: rel.to_string(),
                    line,
                    name: name.clone(),
                };
                by_name.entry(name.clone()).or_default().push(info.clone());
                if unit_variants.len() >= MIN_DUPLICATE_ARMS && !name.ends_with("Error") {
                    let key: Vec<String> = unit_variants.into_iter().collect();
                    by_variants.entry(key).or_default().push(info);
                }
                if is_core {
                    core_names.insert(name.clone());
                }
                if !name.ends_with("Error") {
                    internal_names.insert(name);
                }
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_pub_enums(
                        rel,
                        sub,
                        is_core,
                        by_name,
                        by_variants,
                        core_names,
                        internal_names,
                    );
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
                let impl_ty = last_type_ident(&imp.self_ty);
                let mut flagged_from_impl = false;
                if let Some((_, trait_path, _)) = &imp.trait_
                    && let Some(seg) = trait_path.segments.last()
                    && (seg.ident == "From" || seg.ident == "TryFrom")
                    && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
                    && let Some(syn::GenericArgument::Type(src_ty)) = args.args.first()
                    && !type_has_external_sdk(src_ty)
                    && !type_has_external_sdk(&imp.self_ty)
                    && let (Some(s), Some(d)) = (last_type_ident(src_ty), impl_ty.clone())
                    && s != d
                    && internal_enums.contains(&s)
                    && internal_enums.contains(&d)
                {
                    flagged_from_impl = true;
                    let ln = imp.impl_token.span.start().line;
                    out.push(Violation::new(
                        rel,
                        ln,
                        line_at(raw, ln),
                        format!(
                            "Redundant internal NVE-to-NVE enum converter `impl {}<{s}> for {d}`; collapse into one canonical enum",
                            seg.ident
                        ),
                    ));
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                    {
                        if flagged_from_impl
                            && (f.sig.ident == "from" || f.sig.ident == "try_from")
                        {
                            continue;
                        }
                        check_converter_fn(
                            rel,
                            raw,
                            &f.sig,
                            &f.block,
                            impl_ty.as_deref(),
                            internal_enums,
                            out,
                        );
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                check_converter_fn(rel, raw, &f.sig, &f.block, None, internal_enums, out);
            }
            _ => {}
        }
    }
}

fn check_converter_fn(
    rel: &str,
    raw: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_ty: Option<&str>,
    internal_enums: &BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    let name = sig.ident.to_string();
    if EXTERNAL_BRIDGE_FN_PREFIXES.iter().any(|p| name.starts_with(p)) {
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

    let ok_ret = unwrap_option_or_result(ret_ty);
    if type_is_collection_or_ref(ok_ret) {
        return;
    }

    let mut param_enums = BTreeSet::new();
    if takes_self
        && let Some(st) = self_ty
        && internal_enums.contains(st)
    {
        param_enums.insert(st.to_string());
    }
    for ty in &param_tys {
        collect_internal_enums_in_type(ty, self_ty, internal_enums, &mut param_enums);
    }
    let mut ret_enums = BTreeSet::new();
    collect_internal_enums_in_type(ok_ret, self_ty, internal_enums, &mut ret_enums);

    let single_disc = param_tys.len() == 1 && is_discriminant_type(param_tys[0]);
    let any_disc = param_tys.iter().any(|ty| is_discriminant_type(ty));
    let is_named_converter = name.starts_with("decode_")
        || name.starts_with("map_")
        || name.starts_with("convert_")
        || name.starts_with("to_")
        || name.starts_with("from_")
        || matches!(name.as_str(), "from_u8" | "from_u32" | "to_u8");

    let is_self_disc_name = name.starts_with("from_u")
        || name.starts_with("from_i")
        || name.starts_with("from_word")
        || name.starts_with("from_code")
        || name.starts_with("decode_")
        || name.starts_with("map_")
        || name.starts_with("convert_");

    let is_redundant_disc = !takes_self
        && ((single_disc && type_mentions_self(ok_ret) && is_self_disc_name)
            || (!ret_enums.is_empty() && any_disc && is_named_converter)
            || (!ret_enums.is_empty() && single_disc && body_has_compact_int_match(block, 3)));

    let distinct_e2e = !param_enums.is_empty()
        && !ret_enums.is_empty()
        && param_enums != ret_enums
        && (is_named_converter
            || (param_tys.len() + usize::from(takes_self) == 1 && body_has_enum_match(block, 3)));

    if is_redundant_disc || distinct_e2e {
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

fn body_has_compact_int_match(block: &syn::Block, min_arms: usize) -> bool {
    struct Scan {
        min_arms: usize,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Scan {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            let mut vals = BTreeSet::new();
            for arm in &m.arms {
                if let Pat::Lit(l) = &arm.pat
                    && let syn::Lit::Int(i) = &l.lit
                    && let Ok(v) = i.base10_parse::<i128>()
                {
                    vals.insert(v);
                }
            }
            if vals.len() >= self.min_arms {
                let min = vals.first().copied().unwrap_or(0);
                let max = vals.last().copied().unwrap_or(0);
                if min >= 0 && max <= 255 && (max - min) <= (vals.len() as i128 * 2 + 4) {
                    self.found = true;
                }
            }
            if !self.found {
                visit::visit_expr_match(self, m);
            }
        }
    }
    let mut s = Scan {
        min_arms,
        found: false,
    };
    s.visit_block(block);
    s.found
}

fn body_has_enum_match(block: &syn::Block, min_arms: usize) -> bool {
    struct Scan {
        min_arms: usize,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Scan {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            let non_wild = m
                .arms
                .iter()
                .filter(|a| !matches!(a.pat, Pat::Wild(_)))
                .count();
            if non_wild >= self.min_arms {
                self.found = true;
            }
            if !self.found {
                visit::visit_expr_match(self, m);
            }
        }
    }
    let mut s = Scan {
        min_arms,
        found: false,
    };
    s.visit_block(block);
    s.found
}

fn is_discriminant_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.qself.is_none() && tp.path.segments.len() == 1
        && matches!(tp.path.segments[0].ident.to_string().as_str(), "u8" | "u16" | "u32" | "i32"))
}

fn unwrap_option_or_result(ty: &Type) -> &Type {
    if let Type::Path(tp) = ty
        && let Some(last) = tp.path.segments.last()
        && matches!(last.ident.to_string().as_str(), "Result" | "Option")
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return inner;
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
    self_ty: Option<&str>,
    internal_enums: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    struct EnumScan<'a>(Option<&'a str>, &'a BTreeSet<String>, &'a mut BTreeSet<String>);
    impl<'ast> Visit<'ast> for EnumScan<'_> {
        fn visit_path(&mut self, p: &'ast syn::Path) {
            if let Some(last) = p.segments.last() {
                let raw = last.ident.to_string();
                let name = if raw == "Self" {
                    self.0.unwrap_or(&raw).to_string()
                } else {
                    raw
                };
                if self.1.contains(&name) {
                    self.2.insert(name);
                }
            }
            visit::visit_path(self, p);
        }
    }
    EnumScan(self_ty, internal_enums, out).visit_type(ty);
}
