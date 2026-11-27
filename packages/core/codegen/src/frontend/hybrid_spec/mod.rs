//! Declarative NVE Hybrid FFI `.rs` specifications, parsed with `syn` 2: the sanctioned file
//! frontend of `nve codegen hybrid`.
//!
//! Recognizes idiomatic Rust declarations annotated with:
//! - `#[nve_hybrid_enum]` on unit-only `#[repr(u8|u16|u32|i32)] pub enum ...` (`c_prefix`,
//!   `dart_fallback`)
//! - `#[nve_hybrid_struct]` / `#[nve_hybrid_union]` with named fields (`size`, `size_64`, `align`,
//!   or `#[repr(align(N))]`; fields take `#[offset(N)]` / `#[offset64(N)]`)
//! - `#[nve_hybrid_module]` / `#[nve_hybrid_view]` on traits (`name`, `interface_id`, module
//!   `c_name`), with view members `#[prop]` / `#[prop(default = ...)]`, `#[event]`, `#[method]`
//!   and module members `#[optional]`
//! - `#[nve_hybrid_const]`, `#[nve_hybrid_alias]`, and `#[nve_hybrid_c_fn]` on body-less
//!   `extern "C" fn`s, also inside `extern "C" { ... }`
//! - `#[nve_hybrid_namespace(name = "...")]` on any item, or `#![nve_hybrid_namespace(...)]` for the file
//!
//! Docs come from outer `///` and `/** */` comments. Unannotated enums, structs, unions, consts,
//! and aliases are ignored; everything else (impls, bodies, generics, variadics, `async`, tuple
//! structs, non-unit variants, `packed`, raw identifiers, `cfg`, ...) is a located error.
//!
//! Parsing runs on worker threads whose stack is sized from the input ([`stack`]), so no input
//! can overflow a stack, and the caller's `proc_macro2` span table is never touched.

mod attrs;
mod items;
mod stack;
mod types;

use std::path::Path;

use syn::spanned::Spanned;
use syn::{Attribute, ForeignItem, ForeignItemFn, Item};

use crate::error::CodegenError;
use crate::schema::hybrid::{HybridAliasSpec, HybridConstSpec, HybridFileSpec};
use attrs::{Args, keys};
use items::{TraitSpec, parse_enum, parse_record, parse_trait, reject_generics};
use types::{convert_type, error_at, ident, is_hex, syn_error, unsigned};

/// Parses a declarative Rust `.rs` hybrid specification source string into a validated
/// [`HybridFileSpec`].
pub fn parse_hybrid_rust_spec(source: &str) -> Result<HybridFileSpec, CodegenError> {
    let body = source.strip_prefix('\u{feff}').unwrap_or(source);
    stack::on_sized_stack(body, |body| {
        let mut file = parse_tokens(body)?;
        file.resolve_named_types();
        file.validate()?;
        Ok(file)
    })
}

/// Reads and parses a `.rs` hybrid specification file on disk.
pub fn parse_hybrid_rust_spec_file(path: &Path) -> Result<HybridFileSpec, CodegenError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| CodegenError::io(path.display().to_string(), e))?;
    parse_hybrid_rust_spec(&source)
}

/// Lowers `body` (on a [`stack::on_sized_stack`] thread: `syn` recurses).
fn parse_tokens(body: &str) -> Result<HybridFileSpec, CodegenError> {
    let ast: syn::File = syn::parse2(stack::lex(body)?).map_err(|e| syn_error(&e, body))?;
    let mut file = HybridFileSpec::default();
    attrs::check_markers(
        &ast.attrs,
        &[NAMESPACE],
        "the file (only `#![nve_hybrid_namespace]`)",
    )?;
    apply_namespace(&ast.attrs, &mut file)?;
    for item in &ast.items {
        parse_item(item, &mut file)?;
    }
    Ok(file)
}

const NAMESPACE: &str = "nve_hybrid_namespace";

/// `#[nve_hybrid_namespace(name = "...")]` (inner or outer) sets the file namespace; a second,
/// different namespace anywhere in the file is an error.
fn apply_namespace(attrs: &[Attribute], file: &mut HybridFileSpec) -> Result<(), CodegenError> {
    for attr in attrs.iter().filter(|a| attrs::is_named(a, NAMESPACE)) {
        match Args::parse(attr, keys::NAMESPACE)?.string("name")? {
            Some(name) => match &file.namespace {
                Some(prev) if *prev != name => {
                    return Err(error_at(
                        attr.span(),
                        format!("namespace `{name}` conflicts with `{prev}`"),
                    ));
                }
                _ => file.namespace = Some(name),
            },
            None => {
                return Err(error_at(
                    attr.span(),
                    "#[nve_hybrid_namespace] needs `name = \"...\"`",
                ));
            }
        }
    }
    Ok(())
}

/// Outer attributes of the items a hybrid spec may contain.
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

/// The markers valid on each item kind (besides the namespace, valid anywhere).
fn markers_for(item: &Item) -> (&'static [&'static str], &'static str) {
    match item {
        Item::Const(_) => (&[NAMESPACE, "nve_hybrid_const"], "a const"),
        Item::Type(_) => (&[NAMESPACE, "nve_hybrid_alias"], "a type alias"),
        Item::Enum(_) => (&[NAMESPACE, "nve_hybrid_enum"], "an enum"),
        Item::Struct(_) => (&[NAMESPACE, "nve_hybrid_struct"], "a struct"),
        Item::Union(_) => (&[NAMESPACE, "nve_hybrid_union"], "a union"),
        Item::Trait(_) => (
            &[NAMESPACE, "nve_hybrid_module", "nve_hybrid_view"],
            "a trait",
        ),
        _ => (&[NAMESPACE], "this item"),
    }
}

fn parse_item(item: &Item, file: &mut HybridFileSpec) -> Result<(), CodegenError> {
    let item_attrs = item_attrs(item);
    let (allowed, what) = markers_for(item);
    attrs::check_markers(item_attrs, allowed, what)?;
    apply_namespace(item_attrs, file)?;
    match item {
        Item::Use(_) => {}
        Item::Const(c) => {
            if let Some(marker) = attrs::find(&c.attrs, "nve_hybrid_const") {
                Args::parse(marker, keys::NONE)?;
                file.consts.push(HybridConstSpec {
                    name: ident(&c.ident)?,
                    ty: convert_type(&c.ty)?,
                    value: unsigned(&c.expr, "const value")?,
                    hex: is_hex(&c.expr),
                    doc: attrs::docs(&c.attrs),
                });
            }
        }
        Item::Type(t) => {
            if let Some(marker) = attrs::find(&t.attrs, "nve_hybrid_alias") {
                Args::parse(marker, keys::NONE)?;
                reject_generics(&t.generics)?;
                file.aliases.push(HybridAliasSpec {
                    name: ident(&t.ident)?,
                    target: convert_type(&t.ty)?,
                    doc: attrs::docs(&t.attrs),
                });
            }
        }
        Item::Enum(e) => {
            if let Some(marker) = attrs::find(&e.attrs, "nve_hybrid_enum") {
                file.enums.push(parse_enum(e, marker)?);
            }
        }
        Item::Struct(s) => {
            if let Some(marker) = attrs::find(&s.attrs, "nve_hybrid_struct") {
                reject_generics(&s.generics)?;
                let syn::Fields::Named(fields) = &s.fields else {
                    return Err(error_at(
                        s.fields.span(),
                        "hybrid structs need named fields",
                    ));
                };
                file.structs.push(parse_record(
                    &s.ident,
                    &s.attrs,
                    marker,
                    fields.named.iter(),
                    false,
                )?);
            }
        }
        Item::Union(u) => {
            if let Some(marker) = attrs::find(&u.attrs, "nve_hybrid_union") {
                reject_generics(&u.generics)?;
                file.structs.push(parse_record(
                    &u.ident,
                    &u.attrs,
                    marker,
                    u.fields.named.iter(),
                    true,
                )?);
            }
        }
        Item::Trait(t) => match parse_trait(t)? {
            TraitSpec::Module(m) => file.modules.push(m),
            TraitSpec::View(v) => file.views.push(v),
        },
        // A body-less `extern "C" fn f(..);` is not valid Rust, so `syn` keeps it verbatim.
        Item::Verbatim(tokens) => {
            let f: ForeignItemFn = syn::parse2(tokens.clone()).map_err(|_| {
                error_at(
                    tokens.span(),
                    "unsupported item (expected a body-less `extern \"C\" fn`)",
                )
            })?;
            foreign_fn(&f, f.sig.abi.as_ref(), file)?;
        }
        Item::ForeignMod(m) => {
            items::check_c_abi(&m.abi)?;
            for foreign in &m.items {
                let ForeignItem::Fn(f) = foreign else {
                    return Err(error_at(
                        foreign.span(),
                        "`extern \"C\" { ... }` blocks may only declare functions",
                    ));
                };
                foreign_fn(f, Some(&m.abi), file)?;
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

/// One `#[nve_hybrid_c_fn]` declaration; any other body-less function is an error.
fn foreign_fn(
    f: &ForeignItemFn,
    abi: Option<&syn::Abi>,
    file: &mut HybridFileSpec,
) -> Result<(), CodegenError> {
    attrs::check_markers(&f.attrs, &[NAMESPACE, "nve_hybrid_c_fn"], "a function")?;
    apply_namespace(&f.attrs, file)?;
    let Some(marker) = attrs::find(&f.attrs, "nve_hybrid_c_fn") else {
        return Err(error_at(
            f.sig.ident.span(),
            "body-less function without #[nve_hybrid_c_fn]",
        ));
    };
    Args::parse(marker, keys::NONE)?;
    file.c_functions
        .push(items::c_function(&f.sig, attrs::docs(&f.attrs), abi)?);
    Ok(())
}
