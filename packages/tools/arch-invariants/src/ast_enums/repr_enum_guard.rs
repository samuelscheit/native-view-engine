//! Zero-tolerance AST guard enforcing `[INV:no-manual-repr-enum-converters]`.
//!
//! Scans production `.rs` files (`packages/`, `hosts/`, `examples/`) for any hand-written
//! integer-to-enum converter targeting a `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]` unit-only enum
//! (public, crate-visible, or private). Covers inherent methods, `impl From<Int>` /
//! `impl TryFrom<Int>`, free functions, 2-arg fallback converters, `match` tables, and
//! `if` / `else if` discriminant chains while avoiding semantic-domain and same-name collisions.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Item, Pat, Type, Visibility};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

use super::repr_enum_body::{body_matches_repr_discriminants, extract_repr_unit_enum};
use super::scope::{EnumQualifier, ImportScope};

pub(super) const REPR_INT_TYPES: &[&str] = &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"];

#[derive(Clone)]
pub(super) struct ReprEnumInfo {
    pub repr: String,
    pub variants: BTreeSet<String>,
    /// Exact discriminants when every variant uses implicit or integer-literal discriminants.
    pub discriminants: Option<BTreeSet<i128>>,
}

struct PubEnumEntry {
    crate_key: String,
    info: ReprEnumInfo,
}

#[derive(Default)]
struct EnumCatalog {
    known_crates: BTreeSet<String>,
    pub_enums: BTreeMap<String, Vec<PubEnumEntry>>,
    crate_enums: BTreeMap<(String, String), ReprEnumInfo>,
    crate_non_repr_types: BTreeSet<(String, String)>,
}

#[derive(Clone, Default)]
struct LocalOverlay {
    repr_enums: BTreeMap<String, ReprEnumInfo>,
    non_repr_types: BTreeSet<String>,
}

impl EnumCatalog {
    fn resolve<'a>(
        &'a self,
        ident: &str,
        origin_crate: Option<&str>,
        current_crate: &str,
        local: &'a LocalOverlay,
    ) -> Option<&'a ReprEnumInfo> {
        if let Some(orig) = origin_crate
            && orig != current_crate
        {
            if self
                .crate_non_repr_types
                .contains(&(orig.to_owned(), ident.to_owned()))
            {
                return None;
            }
            if let Some(entries) = self.pub_enums.get(ident) {
                if let Some(exact) = entries.iter().find(|e| e.crate_key == orig) {
                    return Some(&exact.info);
                }
                if self.known_crates.contains(orig) && entries.len() == 1 {
                    return Some(&entries[0].info);
                }
            }
            return None;
        }
        if let Some(info) = local.repr_enums.get(ident) {
            return Some(info);
        }
        if local.non_repr_types.contains(ident) {
            return None;
        }
        let same_key = (current_crate.to_owned(), ident.to_owned());
        if let Some(info) = self.crate_enums.get(&same_key) {
            return Some(info);
        }
        if let Some(entries) = self.pub_enums.get(ident)
            && let Some(same) = entries.iter().find(|e| e.crate_key == current_crate)
        {
            return Some(&same.info);
        }
        if self.crate_non_repr_types.contains(&same_key) {
            return None;
        }
        self.pub_enums
            .get(ident)
            .and_then(|entries| entries.first().map(|e| &e.info))
    }
}

/// Scans all given production files for hand-written converters of `#[repr(<int>)]` unit enums,
/// inferring crate boundaries from workspace-relative file paths.
pub fn scan_manual_repr_enum_converters(
    files: &[(String, &str, &syn::File)],
    out: &mut Vec<Violation>,
) {
    let with_crates: Vec<(String, String, &str, &syn::File)> = files
        .iter()
        .map(|(rel, raw, ast)| (rel.clone(), infer_crate_key(rel), *raw, *ast))
        .collect();
    scan_manual_repr_enum_converters_with_crates(&with_crates, out);
}

/// Crate-aware entry point using exact `guppy` crate names when scanning the live workspace.
pub fn scan_manual_repr_enum_converters_with_crates(
    files: &[(String, String, &str, &syn::File)],
    out: &mut Vec<Violation>,
) {
    let mut catalog = EnumCatalog::default();
    for (_, crate_key, _, ast) in files {
        catalog.known_crates.insert(crate_key.clone());
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        collect_catalog_items(&ast.items, crate_key, &mut catalog);
    }

    for (rel, crate_key, raw, ast) in files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        let mut local = LocalOverlay::default();
        collect_local_types(&ast.items, &mut local);
        let mut scope = ImportScope::with_crate(Some(crate_key));
        scope.ingest_items(&ast.items);
        scan_items(rel, crate_key, raw, &ast.items, &scope, &catalog, &local, out);
    }
}

fn infer_crate_key(rel: &str) -> String {
    let base = if let Some((prefix, _)) = rel.split_once("/src/") {
        prefix.rsplit('/').next().unwrap_or(prefix)
    } else {
        rel.rsplit_once('/').map_or(rel, |(dir, _)| dir)
    };
    base.replace('-', "_")
}

fn collect_catalog_items(items: &[Item], crate_key: &str, catalog: &mut EnumCatalog) {
    for item in items {
        match item {
            Item::Enum(en) if !is_cfg_test_attr(&en.attrs) => {
                let name = en.ident.to_string();
                if let Some(info) = extract_repr_unit_enum(en) {
                    match &en.vis {
                        Visibility::Public(_) => {
                            catalog
                                .pub_enums
                                .entry(name)
                                .or_default()
                                .push(PubEnumEntry {
                                    crate_key: crate_key.to_owned(),
                                    info,
                                });
                        }
                        Visibility::Restricted(_) => {
                            catalog
                                .crate_enums
                                .insert((crate_key.to_owned(), name), info);
                        }
                        Visibility::Inherited => {}
                    }
                } else if !matches!(en.vis, Visibility::Inherited) {
                    catalog
                        .crate_non_repr_types
                        .insert((crate_key.to_owned(), name));
                }
            }
            Item::Struct(s)
                if !is_cfg_test_attr(&s.attrs) && !matches!(s.vis, Visibility::Inherited) =>
            {
                catalog
                    .crate_non_repr_types
                    .insert((crate_key.to_owned(), s.ident.to_string()));
            }
            Item::Union(u)
                if !is_cfg_test_attr(&u.attrs) && !matches!(u.vis, Visibility::Inherited) =>
            {
                catalog
                    .crate_non_repr_types
                    .insert((crate_key.to_owned(), u.ident.to_string()));
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_catalog_items(sub, crate_key, catalog);
                }
            }
            _ => {}
        }
    }
}

fn collect_local_types(items: &[Item], local: &mut LocalOverlay) {
    for item in items {
        match item {
            Item::Enum(en) if !is_cfg_test_attr(&en.attrs) => {
                let name = en.ident.to_string();
                if let Some(info) = extract_repr_unit_enum(en) {
                    local.non_repr_types.remove(&name);
                    local.repr_enums.insert(name, info);
                } else {
                    local.repr_enums.remove(&name);
                    local.non_repr_types.insert(name);
                }
            }
            Item::Struct(s) if !is_cfg_test_attr(&s.attrs) => {
                let name = s.ident.to_string();
                local.repr_enums.remove(&name);
                local.non_repr_types.insert(name);
            }
            Item::Union(u) if !is_cfg_test_attr(&u.attrs) => {
                let name = u.ident.to_string();
                local.repr_enums.remove(&name);
                local.non_repr_types.insert(name);
            }
            Item::Type(t) if !is_cfg_test_attr(&t.attrs) => {
                let name = t.ident.to_string();
                local.repr_enums.remove(&name);
                local.non_repr_types.insert(name);
            }
            _ => {}
        }
    }
}


#[allow(clippy::too_many_arguments)]
fn scan_items(
    rel: &str,
    crate_key: &str,
    raw: &str,
    items: &[Item],
    scope: &ImportScope,
    catalog: &EnumCatalog,
    local: &LocalOverlay,
    out: &mut Vec<Violation>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    let child_scope = ImportScope::child_for_inline_mod(scope, sub);
                    let mut child_local = local.clone();
                    collect_local_types(sub, &mut child_local);
                    scan_items(rel, crate_key, raw, sub, &child_scope, catalog, &child_local, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let self_qual = resolve_type_qual(&imp.self_ty, scope);
                if let Some((_, trait_path, _)) = &imp.trait_ {
                    check_from_trait_impl(
                        rel,
                        crate_key,
                        raw,
                        imp,
                        trait_path,
                        self_qual.as_ref(),
                        scope,
                        catalog,
                        local,
                        out,
                    );
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                    {
                        check_fn(
                            rel,
                            crate_key,
                            raw,
                            &f.sig,
                            &f.block,
                            self_qual.as_ref(),
                            scope,
                            catalog,
                            local,
                            out,
                        );
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                check_fn(
                    rel,
                    crate_key,
                    raw,
                    &f.sig,
                    &f.block,
                    None,
                    scope,
                    catalog,
                    local,
                    out,
                );
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_from_trait_impl(
    rel: &str,
    crate_key: &str,
    raw: &str,
    imp: &syn::ItemImpl,
    trait_path: &syn::Path,
    self_qual: Option<&EnumQualifier>,
    scope: &ImportScope,
    catalog: &EnumCatalog,
    local: &LocalOverlay,
    out: &mut Vec<Violation>,
) {
    let Some(seg) = trait_path.segments.last() else {
        return;
    };
    if seg.ident != "From" && seg.ident != "TryFrom" {
        return;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return;
    };
    let Some(syn::GenericArgument::Type(src_ty)) = args.args.first() else {
        return;
    };
    let Some(int_ty) = repr_int_type_name(src_ty) else {
        return;
    };
    let Some(self_q) = self_qual else {
        return;
    };
    let target_enum = self_q.type_name.as_str();
    let Some(info) = catalog.resolve(target_enum, self_q.origin_crate.as_deref(), crate_key, local)
    else {
        return;
    };
    let fn_item = imp.items.iter().find_map(|ii| match ii {
        syn::ImplItem::Fn(f) if f.sig.ident == "from" || f.sig.ident == "try_from" => Some(f),
        _ => None,
    });
    let body_matches = fn_item.is_some_and(|f| {
        let param_name = f.sig.inputs.first().and_then(fn_arg_ident);
        body_matches_repr_discriminants(
            &f.block,
            param_name.as_deref(),
            target_enum,
            Some(target_enum),
            scope,
            info,
            int_ty,
        )
    });
    if body_matches {
        let ln = imp.impl_token.span.start().line;
        out.push(Violation::new(
            rel,
            ln,
            line_at(raw, ln),
            format!(
                "Manual `impl {}<{int_ty}> for {target_enum}` for `#[repr({})]` unit enum; derive `#[derive(ReprEnum)]` (or `WordEnum` / `NveCodec`) instead [INV:no-manual-repr-enum-converters]",
                seg.ident, info.repr
            ),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn check_fn(
    rel: &str,
    crate_key: &str,
    raw: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_qual: Option<&EnumQualifier>,
    scope: &ImportScope,
    catalog: &EnumCatalog,
    local: &LocalOverlay,
    out: &mut Vec<Violation>,
) {
    let syn::ReturnType::Type(_, ret_ty) = &sig.output else {
        return;
    };
    let Some((target_enum, info)) =
        resolve_returned_repr_enum(ret_ty, self_qual, crate_key, scope, catalog, local)
    else {
        return;
    };

    let self_ty = self_qual.map(|q| q.type_name.as_str());
    // Accept either 1-arg `(int)` or 2-arg fallback `(int, TargetEnum)` / `(TargetEnum, int)`.
    let Some((int_ty, int_param_name)) = extract_int_converter_arg(sig, &target_enum, self_ty, scope) else {
        return;
    };

    let fn_name = sig.ident.to_string();
    let name_matches = is_manual_from_int_name(&fn_name)
        && (int_ty == info.repr || is_widening_or_same_repr(&info.repr, int_ty));
    let body_matches = body_matches_repr_discriminants(
        block,
        int_param_name.as_deref(),
        &target_enum,
        self_ty,
        scope,
        info,
        int_ty,
    );

    if name_matches || body_matches {
        let ln = sig.ident.span().start().line;
        out.push(Violation::new(
            rel,
            ln,
            line_at(raw, ln),
            format!(
                "Manual integer-to-enum converter `fn {fn_name}` for `#[repr({})]` unit enum `{target_enum}`; derive `#[derive(ReprEnum)]` (or `WordEnum` / `NveCodec`) instead [INV:no-manual-repr-enum-converters]",
                info.repr
            ),
        ));
    }
}

fn extract_int_converter_arg(
    sig: &syn::Signature,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
) -> Option<(&'static str, Option<String>)> {
    if sig.inputs.is_empty() || sig.inputs.len() > 2 {
        return None;
    }
    let mut int_arg: Option<(&'static str, Option<String>)> = None;
    let mut fallback_enum_arg = false;
    for input in &sig.inputs {
        let syn::FnArg::Typed(arg) = input else {
            return None;
        };
        if let Some(int_ty) = repr_int_type_name(&arg.ty) {
            if int_arg.is_some() {
                return None;
            }
            int_arg = Some((int_ty, pat_binding_ident(&arg.pat)));
        } else if type_matches_enum(&arg.ty, target_enum, self_ty, scope) {
            if fallback_enum_arg {
                return None;
            }
            fallback_enum_arg = true;
        } else {
            return None;
        }
    }
    int_arg
}

fn fn_arg_ident(arg: &syn::FnArg) -> Option<String> {
    let syn::FnArg::Typed(pt) = arg else {
        return None;
    };
    pat_binding_ident(&pt.pat)
}

fn pat_binding_ident(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(pi) => Some(pi.ident.to_string()),
        Pat::Paren(p) => pat_binding_ident(&p.pat),
        _ => None,
    }
}

fn type_matches_enum(
    ty: &Type,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    if tp.qself.is_some() {
        return false;
    }
    let Some(last) = tp.path.segments.last() else {
        return false;
    };
    if !matches!(last.arguments, syn::PathArguments::None) {
        return false;
    }
    let ident = last.ident.to_string();
    if ident == "Self" {
        return self_ty == Some(target_enum);
    }
    scope
        .resolve_type_path(&tp.path, self_ty)
        .map(|q| q.type_name)
        .unwrap_or(ident)
        == target_enum
}

fn repr_int_type_name(ty: &Type) -> Option<&'static str> {
    let Type::Path(tp) = ty else {
        return None;
    };
    if tp.qself.is_some()
        || tp.path.segments.len() != 1
        || !matches!(tp.path.segments[0].arguments, syn::PathArguments::None)
    {
        return None;
    }
    let ident = tp.path.segments[0].ident.to_string();
    REPR_INT_TYPES.iter().copied().find(|&r| r == ident)
}

fn is_widening_or_same_repr(enum_repr: &str, arg_int: &str) -> bool {
    let bits = |r: &str| -> (bool, u8) {
        match r {
            "u8" => (false, 8),
            "u16" => (false, 16),
            "u32" => (false, 32),
            "u64" => (false, 64),
            "i8" => (true, 8),
            "i16" => (true, 16),
            "i32" => (true, 32),
            "i64" => (true, 64),
            _ => (false, 0),
        }
    };
    let (es, eb) = bits(enum_repr);
    let (as_, ab) = bits(arg_int);
    (es == as_ && ab >= eb) || (!es && as_ && ab > eb)
}

fn is_manual_from_int_name(name: &str) -> bool {
    let rest = name
        .strip_prefix("try_from_")
        .or_else(|| name.strip_prefix("from_"));
    rest.is_some_and(|suffix| REPR_INT_TYPES.contains(&suffix))
}

fn resolve_type_qual(ty: &Type, scope: &ImportScope) -> Option<EnumQualifier> {
    if let Type::Path(tp) = ty
        && tp.qself.is_none()
        && let Some(q) = scope.resolve_type_path(&tp.path, None)
    {
        return Some(q);
    }
    last_type_ident(ty).map(EnumQualifier::unresolved)
}

fn unwrap_option_result(mut ty: &Type) -> &Type {
    loop {
        if let Type::Path(tp) = ty
            && let Some(last) = tp.path.segments.last()
            && matches!(last.ident.to_string().as_str(), "Option" | "Result")
            && let syn::PathArguments::AngleBracketed(args) = &last.arguments
            && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
        {
            ty = inner;
            continue;
        }
        return ty;
    }
}

fn resolve_returned_repr_enum<'a>(
    ret_ty: &Type,
    self_qual: Option<&EnumQualifier>,
    crate_key: &str,
    scope: &ImportScope,
    catalog: &'a EnumCatalog,
    local: &'a LocalOverlay,
) -> Option<(String, &'a ReprEnumInfo)> {
    let inner = unwrap_option_result(ret_ty);
    let Type::Path(tp) = inner else {
        return None;
    };
    if tp.qself.is_some() || scope.is_external_sdk_path(&tp.path) {
        return None;
    }
    let last = tp.path.segments.last()?;
    if !matches!(last.arguments, syn::PathArguments::None) {
        return None;
    }
    let ident = last.ident.to_string();
    let qual = if ident == "Self" {
        self_qual?.clone()
    } else {
        let self_ty = self_qual.map(|q| q.type_name.as_str());
        scope
            .resolve_type_path(&tp.path, self_ty)
            .unwrap_or_else(|| EnumQualifier::unresolved(ident))
    };
    let info = catalog.resolve(&qual.type_name, qual.origin_crate.as_deref(), crate_key, local)?;
    Some((qual.type_name, info))
}
