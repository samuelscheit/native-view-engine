//! Zero-tolerance AST guard enforcing `[INV:no-manual-repr-enum-converters]`.
//!
//! Scans production `.rs` files (`packages/`, `hosts/`, `examples/`) for any hand-written
//! integer-to-enum converter targeting a `#[repr(u8|u16|u32|u64|i8|i16|i32|i64)]` unit-only enum
//! (public or private). Deriving `#[derive(ReprEnum)]`, `#[derive(WordEnum)]`, or
//! `#[derive(NveCodec)]` generates these conversions without duplicating discriminant tables.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{Expr, Item, Lit, Pat, Type, UnOp, Visibility};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

use super::scope::ImportScope;

const REPR_INT_TYPES: &[&str] = &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"];

/// Scans all given production files for hand-written converters of `#[repr(<int>)]` unit enums.
pub fn scan_manual_repr_enum_converters(
    files: &[(String, &str, &syn::File)],
    out: &mut Vec<Violation>,
) {
    let mut pub_repr_enums: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, _, ast) in files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        collect_repr_unit_enums(&ast.items, true, &mut pub_repr_enums);
    }

    for (rel, raw, ast) in files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        let mut local_enums = pub_repr_enums.clone();
        collect_repr_unit_enums(&ast.items, false, &mut local_enums);
        let mut scope = ImportScope::default();
        scope.ingest_items(&ast.items);
        scan_items(rel, raw, &ast.items, &scope, &local_enums, out);
    }
}

fn collect_repr_unit_enums(
    items: &[Item],
    pub_only: bool,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    for item in items {
        match item {
            Item::Enum(en) if !is_cfg_test_attr(&en.attrs) => {
                if pub_only && !matches!(en.vis, Visibility::Public(_)) {
                    continue;
                }
                if !has_int_repr_attr(&en.attrs) || en.variants.is_empty() {
                    continue;
                }
                if !en.variants.iter().all(|v| matches!(v.fields, syn::Fields::Unit)) {
                    continue;
                }
                let entry = out.entry(en.ident.to_string()).or_default();
                for v in &en.variants {
                    entry.insert(v.ident.to_string());
                }
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_repr_unit_enums(sub, pub_only, out);
                }
            }
            _ => {}
        }
    }
}

fn has_int_repr_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(id) = meta.path.get_ident()
                && REPR_INT_TYPES.contains(&id.to_string().as_str())
            {
                found = true;
            }
            Ok(())
        });
        found
    })
}

fn scan_items(
    rel: &str,
    raw: &str,
    items: &[Item],
    scope: &ImportScope,
    repr_enums: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Violation>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    let child = ImportScope::child_for_inline_mod(scope, sub);
                    let mut child_enums = repr_enums.clone();
                    collect_repr_unit_enums(sub, false, &mut child_enums);
                    scan_items(rel, raw, sub, &child, &child_enums, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) && imp.trait_.is_none() => {
                let self_ty = resolve_type_name(&imp.self_ty, scope)
                    .or_else(|| last_type_ident(&imp.self_ty));
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                    {
                        check_fn(
                            rel,
                            raw,
                            &f.sig,
                            &f.block,
                            self_ty.as_deref(),
                            scope,
                            repr_enums,
                            out,
                        );
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                check_fn(rel, raw, &f.sig, &f.block, None, scope, repr_enums, out);
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_fn(
    rel: &str,
    raw: &str,
    sig: &syn::Signature,
    block: &syn::Block,
    self_ty: Option<&str>,
    scope: &ImportScope,
    repr_enums: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Violation>,
) {
    if sig.inputs.len() != 1 {
        return;
    }
    let syn::FnArg::Typed(arg) = sig.inputs.first().expect("len == 1") else {
        return;
    };
    if !is_repr_int_type(&arg.ty) {
        return;
    }
    let syn::ReturnType::Type(_, ret_ty) = &sig.output else {
        return;
    };
    let Some(target_enum) = resolve_returned_repr_enum(ret_ty, self_ty, scope, repr_enums) else {
        return;
    };
    let variants = &repr_enums[&target_enum];
    let fn_name = sig.ident.to_string();

    let name_matches = is_manual_from_int_name(&fn_name);
    let body_matches =
        body_has_int_literal_variant_match(block, &target_enum, self_ty, scope, variants);

    if name_matches || body_matches {
        let ln = sig.ident.span().start().line;
        out.push(Violation::new(
            rel,
            ln,
            line_at(raw, ln),
            format!(
                "Manual integer-to-enum converter `fn {fn_name}` for `#[repr(<int>)]` unit enum `{target_enum}`; derive `#[derive(ReprEnum)]` (or `WordEnum` / `NveCodec`) instead [INV:no-manual-repr-enum-converters]"
            ),
        ));
    }
}

fn is_repr_int_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Path(tp)
            if tp.qself.is_none()
                && tp.path.segments.len() == 1
                && matches!(tp.path.segments[0].arguments, syn::PathArguments::None)
                && REPR_INT_TYPES.contains(&tp.path.segments[0].ident.to_string().as_str())
    )
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

fn resolve_returned_repr_enum(
    ret_ty: &Type,
    self_ty: Option<&str>,
    scope: &ImportScope,
    repr_enums: &BTreeMap<String, BTreeSet<String>>,
) -> Option<String> {
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
    repr_enums.contains_key(&candidate).then_some(candidate)
}

fn body_has_int_literal_variant_match(
    block: &syn::Block,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    struct Scan<'a> {
        target_enum: &'a str,
        self_ty: Option<&'a str>,
        scope: &'a ImportScope,
        variants: &'a BTreeSet<String>,
        found: bool,
    }

    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            let mut matching_arms = 0usize;
            for arm in &m.arms {
                if pat_has_int_literal(&arm.pat)
                    && expr_constructs_enum_variant(
                        &arm.body,
                        self.target_enum,
                        self.self_ty,
                        self.scope,
                        self.variants,
                    )
                {
                    matching_arms += 1;
                }
            }
            if matching_arms >= 2 {
                self.found = true;
            }
            if !self.found {
                visit::visit_expr_match(self, m);
            }
        }
    }

    let mut scan = Scan {
        target_enum,
        self_ty,
        scope,
        variants,
        found: false,
    };
    scan.visit_block(block);
    scan.found
}

fn pat_has_int_literal(pat: &Pat) -> bool {
    match pat {
        Pat::Lit(l) => matches!(l.lit, Lit::Int(_)),
        Pat::Paren(p) => pat_has_int_literal(&p.pat),
        Pat::Reference(r) => pat_has_int_literal(&r.pat),
        Pat::Or(or) => or.cases.iter().any(pat_has_int_literal),
        Pat::Ident(pi) => pi.subpat.as_ref().is_some_and(|(_, p)| pat_has_int_literal(p)),
        Pat::Const(pc) => expr_is_int_literal(&pc.block),
        Pat::Verbatim(ts) => {
            let toks: Vec<String> = ts.clone().into_iter().map(|t| t.to_string()).collect();
            matches!(toks.as_slice(), [neg, lit] if neg == "-" && lit.starts_with(|c: char| c.is_ascii_digit()))
        }
        _ => false,
    }
}

fn expr_is_int_literal(block: &syn::Block) -> bool {
    let [syn::Stmt::Expr(expr, None)] = block.stmts.as_slice() else {
        return false;
    };
    match expr {
        Expr::Lit(l) => matches!(l.lit, Lit::Int(_)),
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => {
            matches!(&*u.expr, Expr::Lit(l) if matches!(l.lit, Lit::Int(_)))
        }
        _ => false,
    }
}

fn expr_constructs_enum_variant(
    mut expr: &Expr,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    loop {
        match expr {
            Expr::Paren(p) => expr = &p.expr,
            Expr::Group(g) => expr = &g.expr,
            Expr::Return(r) => match &r.expr {
                Some(inner) => expr = inner,
                None => return false,
            },
            Expr::Block(b) if b.block.stmts.len() == 1 => match &b.block.stmts[0] {
                syn::Stmt::Expr(inner, None) => expr = inner,
                _ => return false,
            },
            Expr::Call(call) if call.args.len() == 1 => {
                if let Expr::Path(ep) = &*call.func
                    && let Some(last) = ep.path.segments.last()
                    && matches!(last.ident.to_string().as_str(), "Some" | "Ok")
                {
                    expr = &call.args[0];
                    continue;
                }
                return false;
            }
            Expr::Path(ep) => {
                return path_is_target_variant(&ep.path, target_enum, self_ty, scope, variants);
            }
            _ => return false,
        }
    }
}

fn path_is_target_variant(
    path: &syn::Path,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.len() < 2 {
        return segs.len() == 1 && variants.contains(&segs[0]);
    }
    let variant = &segs[segs.len() - 1];
    if !variants.contains(variant) {
        return false;
    }
    let parent = &segs[..segs.len() - 1];
    if parent == ["Self"] {
        return self_ty == Some(target_enum);
    }
    if let Some(q) = scope.resolve_type_segments(parent, self_ty) {
        return q.type_name == target_enum;
    }
    parent.last().is_some_and(|s| s == target_enum)
}
