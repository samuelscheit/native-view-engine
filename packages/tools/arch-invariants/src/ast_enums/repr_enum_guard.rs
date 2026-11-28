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

use super::repr_enum_body::{body_matches_repr_discriminants, eval_int_literal_expr};
use super::scope::ImportScope;

const REPR_INT_TYPES: &[&str] = &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"];

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
    pub_enums: BTreeMap<String, Vec<PubEnumEntry>>,
    crate_enums: BTreeMap<(String, String), ReprEnumInfo>,
}

#[derive(Default)]
struct LocalOverlay {
    repr_enums: BTreeMap<String, ReprEnumInfo>,
    non_repr_types: BTreeSet<String>,
}

impl EnumCatalog {
    fn resolve<'a>(
        &'a self,
        ident: &str,
        crate_key: &str,
        local: &'a LocalOverlay,
    ) -> Option<&'a ReprEnumInfo> {
        if let Some(info) = local.repr_enums.get(ident) {
            return Some(info);
        }
        if local.non_repr_types.contains(ident) {
            return None;
        }
        if let Some(info) = self.crate_enums.get(&(crate_key.to_owned(), ident.to_owned())) {
            return Some(info);
        }
        if let Some(entries) = self.pub_enums.get(ident) {
            if let Some(same) = entries.iter().find(|e| e.crate_key == crate_key) {
                return Some(&same.info);
            }
            return entries.first().map(|e| &e.info);
        }
        None
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
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        collect_catalog_enums(&ast.items, crate_key, &mut catalog);
    }

    for (rel, crate_key, raw, ast) in files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        let mut local = LocalOverlay::default();
        collect_local_types(&ast.items, &mut local);
        let mut scope = ImportScope::default();
        scope.ingest_items(&ast.items);
        scan_items(rel, crate_key, raw, &ast.items, &scope, &catalog, &local, out);
    }
}

fn infer_crate_key(rel: &str) -> String {
    if let Some((prefix, _)) = rel.split_once("/src/") {
        prefix.to_owned()
    } else {
        rel.rsplit_once('/').map_or(rel, |(dir, _)| dir).to_owned()
    }
}

fn collect_catalog_enums(items: &[Item], crate_key: &str, catalog: &mut EnumCatalog) {
    for item in items {
        match item {
            Item::Enum(en) if !is_cfg_test_attr(&en.attrs) => {
                let Some(info) = extract_repr_unit_enum(en) else {
                    continue;
                };
                let name = en.ident.to_string();
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
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_catalog_enums(sub, crate_key, catalog);
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
                    local.repr_enums.insert(name, info);
                } else {
                    local.non_repr_types.insert(name);
                }
            }
            Item::Struct(s) if !is_cfg_test_attr(&s.attrs) => {
                local.non_repr_types.insert(s.ident.to_string());
            }
            Item::Union(u) if !is_cfg_test_attr(&u.attrs) => {
                local.non_repr_types.insert(u.ident.to_string());
            }
            Item::Type(t) if !is_cfg_test_attr(&t.attrs) => {
                local.non_repr_types.insert(t.ident.to_string());
            }
            _ => {}
        }
    }
}

fn extract_repr_unit_enum(en: &syn::ItemEnum) -> Option<ReprEnumInfo> {
    let repr = find_int_repr(&en.attrs)?;
    if en.variants.is_empty() || !en.variants.iter().all(|v| matches!(v.fields, syn::Fields::Unit)) {
        return None;
    }
    let mut variants = BTreeSet::new();
    let mut discs = BTreeSet::new();
    let mut all_discs_known = true;
    let mut next_disc: i128 = 0;

    for v in &en.variants {
        variants.insert(v.ident.to_string());
        if let Some((_, expr)) = &v.discriminant {
            if let Some(val) = eval_int_literal_expr(expr) {
                discs.insert(val);
                next_disc = val.saturating_add(1);
            } else {
                all_discs_known = false;
            }
        } else if all_discs_known {
            discs.insert(next_disc);
            next_disc = next_disc.saturating_add(1);
        }
    }

    Some(ReprEnumInfo {
        repr,
        variants,
        discriminants: all_discs_known.then_some(discs),
    })
}

fn find_int_repr(attrs: &[syn::Attribute]) -> Option<String> {
    let mut found = None;
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(id) = meta.path.get_ident() {
                let s = id.to_string();
                if REPR_INT_TYPES.contains(&s.as_str()) {
                    found = Some(s);
                }
            }
            Ok(())
        });
    }
    found
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
                    let mut child_local = LocalOverlay::default();
                    collect_local_types(sub, &mut child_local);
                    scan_items(rel, crate_key, raw, sub, &child_scope, catalog, &child_local, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let self_ty = resolve_type_name(&imp.self_ty, scope)
                    .or_else(|| last_type_ident(&imp.self_ty));
                if let Some((_, trait_path, _)) = &imp.trait_ {
                    check_from_trait_impl(
                        rel,
                        crate_key,
                        raw,
                        imp,
                        trait_path,
                        self_ty.as_deref(),
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
                            self_ty.as_deref(),
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
    self_ty: Option<&str>,
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
    let Some(target_enum) = self_ty else {
        return;
    };
    let Some(info) = catalog.resolve(target_enum, crate_key, local) else {
        return;
    };
    // Only flag when the source integer matches or widens the enum's repr (or body matches discriminants).
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
            &ImportScope::default(),
            info,
            int_ty,
        )
    });
    if int_ty == info.repr || body_matches {
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
    self_ty: Option<&str>,
    scope: &ImportScope,
    catalog: &EnumCatalog,
    local: &LocalOverlay,
    out: &mut Vec<Violation>,
) {
    let syn::ReturnType::Type(_, ret_ty) = &sig.output else {
        return;
    };
    let Some((target_enum, info)) =
        resolve_returned_repr_enum(ret_ty, self_ty, crate_key, scope, catalog, local)
    else {
        return;
    };

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

fn resolve_type_name(ty: &Type, scope: &ImportScope) -> Option<String> {
    let Type::Path(tp) = ty else {
        return None;
    };
    if tp.qself.is_some() {
        return None;
    }
    scope.resolve_type_path(&tp.path, None).map(|q| q.type_name)
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
    self_ty: Option<&str>,
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
    let candidate = if ident == "Self" {
        self_ty?.to_string()
    } else {
        scope
            .resolve_type_path(&tp.path, self_ty)
            .map(|q| q.type_name)
            .unwrap_or(ident)
    };
    let info = catalog.resolve(&candidate, crate_key, local)?;
    Some((candidate, info))
}
