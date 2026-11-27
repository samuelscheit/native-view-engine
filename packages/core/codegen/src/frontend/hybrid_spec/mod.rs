//! Declarative NVE Hybrid FFI `.rs` specifications, parsed with `syn` 2.
//!
//! Recognizes idiomatic Rust declarations annotated with:
//! - `#[nve_hybrid_enum]` on `#[repr(u32)] pub enum ...` (plus `c_prefix`, `dart_fallback`)
//! - `#[nve_hybrid_struct]` / `#[nve_hybrid_union]` (plus `size`, `size_64`, `align`; fields take
//!   `#[offset(N)]` / `#[offset64(N)]`)
//! - `#[nve_hybrid_module(name = "...")]` / `#[nve_hybrid_view(name = "...")]` on traits, with view
//!   members `#[prop]`, `#[prop(default = ...)]`, `#[event]`, `#[method]` and module members
//!   `#[optional]`
//! - `#[nve_hybrid_const]`, `#[nve_hybrid_alias]`, `#[nve_hybrid_c_fn]` (body-less `extern "C" fn`)
//!   and `#[nve_hybrid_namespace(name = "...")]`
//!
//! Unannotated enums, structs, unions, consts, and aliases are ignored; any other item is an error.

mod types;

use std::collections::BTreeSet;
use std::path::Path;

use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, Fields, FnArg, ForeignItemFn, Item, Lit, LitInt, Meta, Pat, ReturnType,
    Signature, TraitItem,
};

use crate::error::CodegenError;
use crate::schema::hybrid::{
    HybridAliasSpec, HybridConstSpec, HybridEnumSpec, HybridEnumVariant, HybridEventSpec,
    HybridFieldSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec,
    HybridPropSpec, HybridStructSpec, HybridType, HybridViewSpec,
};
use types::{convert_type, default_value, error_at, is_hex, unsigned};

/// Parses a declarative Rust `.rs` hybrid specification source string into a validated
/// [`HybridFileSpec`].
pub fn parse_hybrid_rust_spec(source: &str) -> Result<HybridFileSpec, CodegenError> {
    let ast = syn::parse_file(source)?;
    let mut file = HybridFileSpec::default();
    for item in &ast.items {
        parse_item(item, &mut file)?;
    }
    resolve_named_types(&mut file);
    file.validate()?;
    Ok(file)
}

/// Reads and parses a `.rs` hybrid specification file on disk.
pub fn parse_hybrid_rust_spec_file(path: &Path) -> Result<HybridFileSpec, CodegenError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| CodegenError::io(path.display().to_string(), e))?;
    parse_hybrid_rust_spec(&source)
}

fn parse_item(item: &Item, file: &mut HybridFileSpec) -> Result<(), CodegenError> {
    if let Some(ns) = find_attr(item_attrs(item), "nve_hybrid_namespace")
        .map(|a| string_arg(a, "name"))
        .transpose()?
    {
        file.namespace = ns;
    }
    match item {
        Item::Use(_) => {}
        Item::Const(c) if has_attr(&c.attrs, "nve_hybrid_const") => {
            let value = unsigned(&c.expr, "const value")?;
            file.consts.push(HybridConstSpec {
                name: c.ident.to_string(),
                ty: convert_type(&c.ty)?,
                value,
                hex: is_hex(&c.expr),
                doc: docs(&c.attrs),
            });
        }
        Item::Type(t) if has_attr(&t.attrs, "nve_hybrid_alias") => {
            file.aliases.push(HybridAliasSpec {
                name: t.ident.to_string(),
                target: convert_type(&t.ty)?,
                doc: docs(&t.attrs),
            })
        }
        Item::Enum(e) if has_attr(&e.attrs, "nve_hybrid_enum") => file.enums.push(parse_enum(e)?),
        Item::Struct(s) if has_attr(&s.attrs, "nve_hybrid_struct") => {
            let Fields::Named(fields) = &s.fields else {
                return Err(error_at(s.span(), "hybrid structs need named fields"));
            };
            let fields = fields
                .named
                .iter()
                .map(|f| (f.ident.as_ref(), &f.ty, &f.attrs));
            file.structs.push(parse_record(
                &s.ident,
                &s.attrs,
                fields,
                "nve_hybrid_struct",
                false,
            )?);
        }
        Item::Union(u) if has_attr(&u.attrs, "nve_hybrid_union") => {
            let fields = u
                .fields
                .named
                .iter()
                .map(|f| (f.ident.as_ref(), &f.ty, &f.attrs));
            file.structs.push(parse_record(
                &u.ident,
                &u.attrs,
                fields,
                "nve_hybrid_union",
                true,
            )?);
        }
        Item::Const(_) | Item::Type(_) | Item::Enum(_) | Item::Struct(_) | Item::Union(_) => {}
        Item::Trait(t) => parse_trait(t, file)?,
        // `extern "C" fn f(..);` has no body, so `syn` keeps it verbatim; it is a foreign fn signature.
        Item::Verbatim(tokens) => {
            let f: ForeignItemFn = syn::parse2(tokens.clone())?;
            if has_attr(&f.attrs, "nve_hybrid_c_fn") {
                file.c_functions
                    .push(signature(&f.sig, docs(&f.attrs), false)?);
            }
        }
        Item::ForeignMod(m) => {
            for foreign in &m.items {
                if let syn::ForeignItem::Fn(f) = foreign
                    && has_attr(&f.attrs, "nve_hybrid_c_fn")
                {
                    file.c_functions
                        .push(signature(&f.sig, docs(&f.attrs), false)?);
                }
            }
        }
        other => {
            return Err(error_at(
                other.span(),
                "unexpected item at top level (expected enum, struct, union, trait, const, type, extern fn, or use)",
            ));
        }
    }
    Ok(())
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::ForeignMod(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

fn parse_enum(e: &syn::ItemEnum) -> Result<HybridEnumSpec, CodegenError> {
    let name = e.ident.to_string();
    let mut variants = Vec::with_capacity(e.variants.len());
    let mut next: u32 = 0;
    for v in &e.variants {
        let value = match &v.discriminant {
            Some((_, expr)) => u32::try_from(unsigned(expr, "enum discriminant")?)
                .map_err(|_| error_at(expr.span(), "enum discriminant exceeds u32::MAX"))?,
            None => next,
        };
        next = value.checked_add(1).ok_or_else(|| {
            CodegenError::validation(format!("enum '{name}' discriminant overflow"))
        })?;
        variants.push(HybridEnumVariant {
            name: v.ident.to_string(),
            value,
            doc: docs(&v.attrs),
        });
    }
    let attr = find_attr(&e.attrs, "nve_hybrid_enum");
    let opt = |key| {
        attr.map(|a| string_arg(a, key))
            .transpose()
            .map(Option::flatten)
    };
    let repr = find_attr(&e.attrs, "repr")
        .map(meta_args)
        .transpose()?
        .and_then(|args| {
            args.into_iter()
                .find(|(k, v)| v.is_none() && matches!(k.as_str(), "u8" | "u16" | "u32" | "i32"))
        })
        .map(|(k, _)| k);
    Ok(HybridEnumSpec {
        name,
        variants,
        doc: docs(&e.attrs),
        c_prefix: opt("c_prefix")?,
        repr,
        dart_fallback: opt("dart_fallback")?,
    })
}

fn parse_record<'a>(
    ident: &syn::Ident,
    attrs: &[Attribute],
    fields: impl Iterator<Item = (Option<&'a syn::Ident>, &'a syn::Type, &'a Vec<Attribute>)>,
    marker: &str,
    is_union: bool,
) -> Result<HybridStructSpec, CodegenError> {
    let mut out = Vec::new();
    for (name, ty, fattrs) in fields {
        let Some(name) = name else {
            return Err(error_at(ident.span(), "hybrid struct fields must be named"));
        };
        let offset = |key| {
            find_attr(fattrs, key)
                .map(|a| a.parse_args::<LitInt>())
                .transpose()
        };
        let parse_offset = |lit: Option<LitInt>| -> Result<Option<usize>, CodegenError> {
            lit.map(|l| {
                l.base10_parse::<usize>()
                    .map_err(|_| error_at(l.span(), "field offset exceeds usize::MAX"))
            })
            .transpose()
        };
        out.push(HybridFieldSpec {
            name: name.to_string(),
            ty: convert_type(ty)?,
            doc: docs(fattrs),
            offset: parse_offset(offset("offset")?)?,
            offset_64: parse_offset(offset("offset64")?)?,
        });
    }
    let layout = find_attr(attrs, marker);
    let size = |key| {
        layout
            .map(|a| usize_arg(a, key))
            .transpose()
            .map(Option::flatten)
    };
    let repr_align = find_attr(attrs, "repr")
        .map(|a| usize_arg(a, "align"))
        .transpose()?
        .flatten();
    Ok(HybridStructSpec {
        name: ident.to_string(),
        fields: out,
        doc: docs(attrs),
        is_union,
        align: size("align")?.or(repr_align),
        expected_size: size("size")?,
        expected_size_64: size("size_64")?,
    })
}

fn parse_trait(t: &syn::ItemTrait, file: &mut HybridFileSpec) -> Result<(), CodegenError> {
    let module = find_attr(&t.attrs, "nve_hybrid_module");
    let view = find_attr(&t.attrs, "nve_hybrid_view");
    let attr = match (module, view) {
        (Some(a), Some(_)) => {
            return Err(error_at(
                a.span(),
                "trait cannot carry both #[nve_hybrid_module] and #[nve_hybrid_view]",
            ));
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => {
            return Err(error_at(
                t.ident.span(),
                "trait in hybrid spec must be annotated with #[nve_hybrid_module] or #[nve_hybrid_view]",
            ));
        }
    };
    let trait_name = t.ident.to_string();
    let name = string_arg(attr, "name")?.unwrap_or_else(|| strip_spec_suffix(&trait_name));
    let explicit_interface_id = usize_arg(attr, "interface_id")?
        .map(|v| {
            u32::try_from(v).map_err(|_| {
                error_at(
                    attr.span(),
                    format!("attribute 'interface_id' value {v} exceeds u32::MAX"),
                )
            })
        })
        .transpose()?;
    let members = t
        .items
        .iter()
        .map(|item| match item {
            TraitItem::Fn(f) => Ok(f),
            other => Err(error_at(
                other.span(),
                "hybrid traits may only declare `fn` members",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if module.is_some() {
        let mut methods = Vec::with_capacity(members.len());
        for f in members {
            if let Some(a) = find_attr(&f.attrs, "prop").or_else(|| find_attr(&f.attrs, "event")) {
                return Err(error_at(
                    a.span(),
                    "#[prop] / #[event] are only valid inside #[nve_hybrid_view], not #[nve_hybrid_module]",
                ));
            }
            let mut sig = signature(&f.sig, docs(&f.attrs), true)?;
            sig.optional = has_attr(&f.attrs, "optional");
            methods.push(sig);
        }
        let c_name = string_arg(attr, "c_name")?;
        file.modules.push(HybridModuleSpec {
            name,
            trait_name,
            methods,
            doc: docs(&t.attrs),
            explicit_interface_id,
            c_name,
        });
        return Ok(());
    }
    let (mut props, mut events, mut methods) = (Vec::new(), Vec::new(), Vec::new());
    for f in members {
        let prop = find_attr(&f.attrs, "prop");
        let event = find_attr(&f.attrs, "event");
        let roles = [
            prop.is_some(),
            event.is_some(),
            has_attr(&f.attrs, "method"),
        ];
        if roles.iter().filter(|r| **r).count() > 1 {
            return Err(error_at(
                f.sig.ident.span(),
                "view member cannot combine #[prop], #[event], and #[method]",
            ));
        }
        let sig = signature(&f.sig, docs(&f.attrs), true)?;
        if let Some(a) = prop {
            if !sig.params.is_empty() {
                return Err(error_at(
                    a.span(),
                    format!(
                        "#[prop] '{}' must take only &self (no parameters)",
                        sig.name
                    ),
                ));
            }
            let default_expr = match &a.meta {
                Meta::List(_) => meta_args(a)?
                    .into_iter()
                    .find(|(k, _)| k == "default")
                    .and_then(|(_, v)| v),
                _ => None,
            };
            let default_value = default_expr.as_ref().map(default_value).transpose()?;
            props.push(HybridPropSpec {
                name: sig.name,
                ty: sig.return_type,
                default_value,
                doc: sig.doc,
            });
        } else if let Some(a) = event {
            if sig.return_type != HybridType::Void {
                return Err(error_at(
                    a.span(),
                    format!("#[event] '{}' must have void return type", sig.name),
                ));
            }
            events.push(HybridEventSpec {
                name: sig.name,
                params: sig.params,
                doc: sig.doc,
            });
        } else {
            methods.push(sig);
        }
    }
    file.views.push(HybridViewSpec {
        name,
        trait_name,
        props,
        events,
        methods,
        doc: docs(&t.attrs),
        explicit_interface_id,
    });
    Ok(())
}

/// A method / C function signature: `self` receivers set `has_self`; parameters must be `name: Type`.
fn signature(
    sig: &Signature,
    doc: Option<String>,
    allow_self: bool,
) -> Result<HybridMethodSpec, CodegenError> {
    let mut params = Vec::new();
    let mut has_self = false;
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(_) if allow_self => has_self = true,
            FnArg::Receiver(r) => return Err(error_at(r.span(), "C functions cannot take `self`")),
            FnArg::Typed(t) => {
                let Pat::Ident(name) = t.pat.as_ref() else {
                    return Err(error_at(
                        t.pat.span(),
                        "parameters must be plain identifiers",
                    ));
                };
                params.push(HybridParamSpec {
                    name: name.ident.to_string(),
                    ty: convert_type(&t.ty)?,
                });
            }
        }
    }
    let return_type = match &sig.output {
        ReturnType::Default => HybridType::Void,
        ReturnType::Type(_, ty) => convert_type(ty)?,
    };
    Ok(HybridMethodSpec {
        name: sig.ident.to_string(),
        params,
        return_type,
        doc,
        has_self,
        optional: false,
    })
}

/// `///` doc lines (each trimmed) joined with `\n`.
fn docs(attrs: &[Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            Meta::NameValue(nv) => match &nv.value {
                Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) => Some(s.value().trim().to_owned()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Attribute whose last path segment is `name` (`nve_hybrid_module` or `nve::nve_hybrid_module`).
fn find_attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs
        .iter()
        .find(|a| a.path().segments.last().is_some_and(|s| s.ident == name))
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    find_attr(attrs, name).is_some()
}

/// `key = value` / `key(value)` / `key` arguments of a list attribute (`#[a(x = 1, y, z(2))]`).
/// A parenthesised single value is returned as the value of `key`; a bare flag has none.
fn meta_args(attr: &Attribute) -> Result<Vec<(String, Option<Expr>)>, CodegenError> {
    let mut args = Vec::new();
    if !matches!(attr.meta, Meta::List(_)) {
        return Ok(args);
    }
    attr.parse_nested_meta(|meta| {
        let key = meta
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if meta.input.peek(syn::Token![=]) {
            args.push((key, Some(meta.value()?.parse::<Expr>()?)));
        } else if meta.input.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in meta.input);
            let tokens: proc_macro2::TokenStream = content.parse()?;
            // `align(8)` carries a value; nested lists such as `cfg(not(test))` are just flags.
            args.push((key, syn::parse2::<Expr>(tokens).ok()));
        } else {
            args.push((key, None));
        }
        Ok(())
    })?;
    Ok(args)
}

fn arg<'a>(args: &'a [(String, Option<Expr>)], key: &str) -> Option<&'a Expr> {
    args.iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_ref())
}

fn string_arg(attr: &Attribute, key: &str) -> Result<Option<String>, CodegenError> {
    let args = meta_args(attr)?;
    match arg(&args, key) {
        None => Ok(None),
        Some(Expr::Lit(syn::ExprLit {
            lit: Lit::Str(s), ..
        })) => Ok(Some(s.value())),
        Some(other) => Err(error_at(
            other.span(),
            format!("expected a string for attribute '{key}'"),
        )),
    }
}

fn usize_arg(attr: &Attribute, key: &str) -> Result<Option<usize>, CodegenError> {
    let args = meta_args(attr)?;
    arg(&args, key)
        .map(|v| {
            let n = unsigned(v, &format!("attribute '{key}'"))?;
            usize::try_from(n).map_err(|_| {
                error_at(
                    v.span(),
                    format!("attribute '{key}' value {n} exceeds usize::MAX"),
                )
            })
        })
        .transpose()
}

fn strip_spec_suffix(trait_name: &str) -> String {
    trait_name
        .strip_suffix("Spec")
        .filter(|s| !s.is_empty())
        .unwrap_or(trait_name)
        .to_owned()
}

/// Rewrites `Struct(name)` references to enums and handle aliases once every declaration is known.
fn resolve_named_types(file: &mut HybridFileSpec) {
    let enums: BTreeSet<String> = file.enums.iter().map(|e| e.name.clone()).collect();
    let aliases: BTreeSet<String> = file.aliases.iter().map(|a| a.name.clone()).collect();
    let mut types: Vec<&mut HybridType> = Vec::new();
    types.extend(file.consts.iter_mut().map(|c| &mut c.ty));
    types.extend(file.aliases.iter_mut().map(|a| &mut a.target));
    types.extend(
        file.structs
            .iter_mut()
            .flat_map(|s| s.fields.iter_mut().map(|f| &mut f.ty)),
    );
    for m in file.modules.iter_mut().flat_map(|m| m.methods.iter_mut()) {
        signature_types(m, &mut types);
    }
    for v in &mut file.views {
        types.extend(v.props.iter_mut().map(|p| &mut p.ty));
        types.extend(
            v.events
                .iter_mut()
                .flat_map(|e| e.params.iter_mut().map(|p| &mut p.ty)),
        );
        for m in &mut v.methods {
            signature_types(m, &mut types);
        }
    }
    for f in &mut file.c_functions {
        signature_types(f, &mut types);
    }
    for ty in types {
        rewrite_named_type(ty, &enums, &aliases);
    }
}

fn signature_types<'a>(m: &'a mut HybridMethodSpec, out: &mut Vec<&'a mut HybridType>) {
    out.extend(m.params.iter_mut().map(|p| &mut p.ty));
    out.push(&mut m.return_type);
}

fn rewrite_named_type(ty: &mut HybridType, enums: &BTreeSet<String>, aliases: &BTreeSet<String>) {
    match ty {
        HybridType::Struct(name) if enums.contains(name) => *ty = HybridType::Enum(name.clone()),
        HybridType::Struct(name) if aliases.contains(name) => {
            *ty = HybridType::Handle(name.clone())
        }
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::ConstPtr(inner)
        | HybridType::MutPtr(inner)
        | HybridType::FixedArray { elem: inner, .. } => rewrite_named_type(inner, enums, aliases),
        HybridType::Result { ok, err } => {
            rewrite_named_type(ok, enums, aliases);
            rewrite_named_type(err, enums, aliases);
        }
        _ => {}
    }
}
