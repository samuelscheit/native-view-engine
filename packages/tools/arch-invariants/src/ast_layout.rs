//! Cross-file `#[repr(C)]` ABI layout & cache-line locks, bare-primitive domain field checks,
//! hardcoded `*_SCHEMA_HASH` detection, and `syn` token helpers.

use std::collections::BTreeMap;

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, Fields, Item, Lit, Type, Visibility};

use crate::{is_cfg_test_attr, line_at, Violation};

const BARE_PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64",
];

const DOMAIN_FIELD_SUFFIXES: &[&str] = &[
    "_id", "_key", "_slot", "_index", "_off", "_len", "_mask", "_flags", "_us", "_ms",
];

const DOMAIN_FIELD_EXACT: &[&str] = &["parent", "surface", "node", "rel"];

struct ReprCStructInfo {
    rel: String,
    name: String,
    line: usize,
    is_pub: bool,
    repr_c: bool,
    derive_nve_abi: bool,
    nve_abi_size: Option<usize>,
    named_fields: Vec<String>,
}

pub fn scan_repr_c_abi_locks(
    abi_files: &[(String, &syn::File)],
    store_files: &[(String, &syn::File)],
) -> Vec<Violation> {
    let mut abi_structs = Vec::new();
    let mut store_structs = Vec::new();
    let mut const_tokens = Vec::new();
    let mut usize_consts = BTreeMap::new();

    for (rel, ast) in abi_files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        collect_abi_items(rel, &ast.items, &mut abi_structs, &mut const_tokens, &mut usize_consts);
    }
    for (rel, ast) in store_files {
        if is_cfg_test_attr(&ast.attrs) {
            continue;
        }
        collect_abi_items(rel, &ast.items, &mut store_structs, &mut const_tokens, &mut usize_consts);
    }

    let compact: String = const_tokens
        .into_iter()
        .flat_map(|ts| {
            let mut s = ts.to_string();
            s.push(';');
            s.into_bytes()
        })
        .map(char::from)
        .filter(|c| !c.is_whitespace())
        .collect();

    let mut violations = Vec::new();
    for st in &abi_structs {
        if !st.is_pub || !st.repr_c || st.derive_nve_abi {
            continue;
        }
        let has_size = compact.contains(&format!("size_of::<{}>()", st.name));
        let has_align = compact.contains(&format!("align_of::<{}>()", st.name));
        let missing_offsets: Vec<&str> = st
            .named_fields
            .iter()
            .filter(|f| !compact.contains(&format!("offset_of!({},{})", st.name, f)))
            .map(String::as_str)
            .collect();
        if !(has_size && has_align && missing_offsets.is_empty()) {
            violations.push(Violation::new(
                &st.rel,
                st.line,
                format!("pub struct {}", st.name),
                format!(
                    "#[repr(C)] ABI struct must derive `NveAbi` or declare compile-time size_of/align_of/offset_of! assertions (size={has_size}, align={has_align}, missing_offsets={missing_offsets:?})"
                ),
            ));
        }
    }

    for (ty, expected) in [("NodeRow", 32usize), ("NveNodeRow", 32), ("NveEvent", 64)] {
        let locked = abi_structs
            .iter()
            .chain(store_structs.iter())
            .find(|st| st.name == ty)
            .is_some_and(|st| {
                st.nve_abi_size == Some(expected)
                    || has_const_size_eq(&compact, &usize_consts, ty, expected)
            });
        if !locked {
            violations.push(Violation::new(
                "packages/core/abi/src",
                0,
                format!("expected struct `{ty}` locked == {expected} bytes"),
                format!("Missing exact struct or compile-time cache-line size lock for `{ty}` ({expected} bytes)"),
            ));
        }
    }
    violations
}

fn collect_abi_items(
    rel: &str,
    items: &[Item],
    structs: &mut Vec<ReprCStructInfo>,
    const_tokens: &mut Vec<TokenStream>,
    usize_consts: &mut BTreeMap<String, usize>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_abi_items(rel, sub, structs, const_tokens, usize_consts);
                }
            }
            Item::Struct(st) if !is_cfg_test_attr(&st.attrs) => {
                let named_fields = match &st.fields {
                    Fields::Named(n) => n
                        .named
                        .iter()
                        .filter_map(|f| f.ident.as_ref().map(ToString::to_string))
                        .collect(),
                    _ => Vec::new(),
                };
                structs.push(ReprCStructInfo {
                    rel: rel.to_string(),
                    name: st.ident.to_string(),
                    line: st.ident.span().start().line,
                    is_pub: matches!(st.vis, Visibility::Public(_)),
                    repr_c: has_attr_list_ident(&st.attrs, "repr", "C"),
                    derive_nve_abi: has_attr_list_ident(&st.attrs, "derive", "NveAbi"),
                    nve_abi_size: extract_nve_abi_size(&st.attrs),
                    named_fields,
                });
            }
            Item::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                if last_type_ident(&c.ty).as_deref() == Some("usize")
                    && let Expr::Lit(l) = &*c.expr
                    && let Lit::Int(int) = &l.lit
                    && let Ok(v) = int.base10_parse::<usize>()
                {
                    usize_consts.insert(c.ident.to_string(), v);
                }
                collect_macro_tokens_in_expr(&c.expr, const_tokens);
            }
            Item::Macro(m) if !is_cfg_test_attr(&m.attrs) => {
                const_tokens.push(m.mac.tokens.clone());
            }
            _ => {}
        }
    }
}

fn collect_macro_tokens_in_expr(expr: &Expr, out: &mut Vec<TokenStream>) {
    struct MacScan<'a>(&'a mut Vec<TokenStream>);
    impl<'ast> Visit<'ast> for MacScan<'_> {
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            self.0.push(m.tokens.clone());
        }
    }
    MacScan(out).visit_expr(expr);
}

fn has_const_size_eq(
    compact: &str,
    usize_consts: &BTreeMap<String, usize>,
    ty: &str,
    expected: usize,
) -> bool {
    let needle = format!("size_of::<{ty}>()==");
    let mut pos = 0;
    while let Some(idx) = compact[pos..].find(&needle) {
        let start = pos + idx + needle.len();
        let rhs: String = compact[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if rhs.replace('_', "").parse::<usize>().ok() == Some(expected)
            || usize_consts.get(&rhs).copied() == Some(expected)
        {
            return true;
        }
        pos = start;
    }
    false
}

pub fn scan_bare_primitive_domain_fields(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    scan_domain_fields_in_items(rel, raw, &ast.items, out);
}

fn scan_domain_fields_in_items(rel: &str, raw: &str, items: &[Item], out: &mut Vec<Violation>) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    scan_domain_fields_in_items(rel, raw, sub, out);
                }
            }
            Item::Struct(st)
                if !is_cfg_test_attr(&st.attrs)
                    && matches!(st.vis, Visibility::Public(_))
                    && !has_attr_list_ident(&st.attrs, "repr", "transparent") =>
            {
                let st_name = st.ident.to_string();
                match &st.fields {
                    Fields::Named(named) => {
                        for field in &named.named {
                            let Some(ident) = &field.ident else { continue };
                            let fname = ident.to_string();
                            let is_domain = DOMAIN_FIELD_EXACT.contains(&fname.as_str())
                                || DOMAIN_FIELD_SUFFIXES.iter().any(|s| fname.ends_with(s));
                            if is_domain && is_bare_or_wrapped_primitive(&field.ty) {
                                let ln = ident.span().start().line;
                                out.push(Violation::new(
                                    rel,
                                    ln,
                                    line_at(raw, ln),
                                    format!(
                                        "pub struct `{st_name}` field `{fname}` uses bare primitive instead of a `#[repr(transparent)]` handle/unit newtype"
                                    ),
                                ));
                            }
                        }
                    }
                    Fields::Unnamed(unnamed) => {
                        let is_domain = DOMAIN_FIELD_SUFFIXES.iter().any(|s| {
                            st_name.len() >= s.len()
                                && st_name.as_bytes()[st_name.len() - s.len()..]
                                    .eq_ignore_ascii_case(s.as_bytes())
                        });
                        if is_domain {
                            for field in &unnamed.unnamed {
                                if is_bare_or_wrapped_primitive(&field.ty) {
                                    let ln = field.span().start().line;
                                    out.push(Violation::new(
                                        rel,
                                        ln,
                                        line_at(raw, ln),
                                        format!(
                                            "pub tuple struct `{st_name}` wraps bare primitive without `#[repr(transparent)]`"
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                    Fields::Unit => {}
                }
            }
            _ => {}
        }
    }
}

pub fn scan_hardcoded_schema_hashes(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    scan_schema_hash_items(rel, raw, &ast.items, out);
}

fn is_hex_int_expr(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Lit(l) if matches!(&l.lit, Lit::Int(i) if i.to_string().starts_with("0x") || i.to_string().starts_with("0X"))
    )
}

fn scan_schema_hash_items(rel: &str, raw: &str, items: &[Item], out: &mut Vec<Violation>) {
    for item in items {
        match item {
            Item::Const(c)
                if !is_cfg_test_attr(&c.attrs)
                    && c.ident.to_string().contains("SCHEMA_HASH")
                    && is_hex_int_expr(&c.expr) =>
            {
                let ln = c.ident.span().start().line;
                out.push(Violation::new(
                    rel,
                    ln,
                    line_at(raw, ln),
                    "Hardcoded `*_SCHEMA_HASH = 0x...` hex literal is banned; compute via `const fn xxh64`",
                ));
            }
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    scan_schema_hash_items(rel, raw, sub, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                for ii in &imp.items {
                    if let syn::ImplItem::Const(c) = ii
                        && !is_cfg_test_attr(&c.attrs)
                        && c.ident.to_string().contains("SCHEMA_HASH")
                        && is_hex_int_expr(&c.expr)
                    {
                        let ln = c.ident.span().start().line;
                        out.push(Violation::new(
                            rel,
                            ln,
                            line_at(raw, ln),
                            "Hardcoded `*_SCHEMA_HASH = 0x...` hex literal is banned; compute via `const fn xxh64`",
                        ));
                    }
                }
            }
            Item::Trait(tr) if !is_cfg_test_attr(&tr.attrs) => {
                for ti in &tr.items {
                    if let syn::TraitItem::Const(c) = ti
                        && !is_cfg_test_attr(&c.attrs)
                        && c.ident.to_string().contains("SCHEMA_HASH")
                        && c.default.as_ref().is_some_and(|(_, expr)| is_hex_int_expr(expr))
                    {
                        let ln = c.ident.span().start().line;
                        out.push(Violation::new(
                            rel,
                            ln,
                            line_at(raw, ln),
                            "Hardcoded `*_SCHEMA_HASH = 0x...` hex literal is banned; compute via `const fn xxh64`",
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn last_type_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        Type::Reference(r) => last_type_ident(&r.elem),
        _ => None,
    }
}

pub fn file_contains_ident(src: &str, target: &str) -> bool {
    src.parse::<TokenStream>()
        .is_ok_and(|ts| stream_contains_ident(ts, target))
}

fn stream_contains_ident(ts: TokenStream, target: &str) -> bool {
    ts.into_iter().any(|tt| match tt {
        TokenTree::Ident(i) => i == target,
        TokenTree::Group(g) => stream_contains_ident(g.stream(), target),
        _ => false,
    })
}

pub(crate) fn has_attr_list_ident(attrs: &[Attribute], attr_name: &str, target: &str) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident(attr_name) {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.segments.last().is_some_and(|s| s.ident == target) {
                found = true;
            }
            if meta.input.peek(syn::token::Paren) {
                let _ = meta.parse_nested_meta(|_| Ok(()));
            }
            Ok(())
        });
        found
    })
}

fn extract_nve_abi_size(attrs: &[Attribute]) -> Option<usize> {
    for a in attrs {
        if !a.path().is_ident("nve_abi") {
            continue;
        }
        let mut found = None;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("size") || meta.path.is_ident("size_64") {
                if let Ok(val) = meta.value()
                    && let Ok(Lit::Int(int)) = val.parse::<Lit>()
                {
                    found = int.base10_parse::<usize>().ok();
                }
            } else if meta.input.peek(syn::Token![=]) {
                let _: Result<Expr, _> = meta.value().and_then(|v| v.parse());
            } else if meta.input.peek(syn::token::Paren) {
                let _ = meta.parse_nested_meta(|_| Ok(()));
            }
            Ok(())
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn is_bare_primitive_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.qself.is_none() && tp.path.segments.len() == 1
        && BARE_PRIMITIVES.contains(&tp.path.segments[0].ident.to_string().as_str()))
}

fn is_bare_or_wrapped_primitive(ty: &Type) -> bool {
    if is_bare_primitive_type(ty) {
        return true;
    }
    match ty {
        Type::Path(tp) if tp.qself.is_none() => {
            if let Some(last) = tp.path.segments.last()
                && matches!(last.ident.to_string().as_str(), "Option" | "Vec")
                && let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
            {
                return is_bare_primitive_type(inner);
            }
            false
        }
        Type::Reference(r) => matches!(&*r.elem, Type::Slice(s) if is_bare_primitive_type(&s.elem)),
        Type::Array(a) => is_bare_primitive_type(&a.elem),
        _ => false,
    }
}
