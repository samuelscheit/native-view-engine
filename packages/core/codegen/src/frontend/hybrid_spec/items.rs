//! Hybrid spec items: enums, structs / unions, traits (modules and views), and C functions.
//! Every Rust form the IR cannot represent is rejected with its location, never dropped.

use syn::spanned::Spanned;
use syn::{Abi, Attribute, Field, FnArg, Generics, Ident, Pat, ReturnType, Signature, TraitItem};

use super::attrs::{self, Args, keys};
use super::types::{convert_type, default_value, error_at, ident, unsigned_u32};
use crate::error::CodegenError;
use crate::schema::hybrid::{
    HybridEnumSpec, HybridEnumVariant, HybridEventSpec, HybridFieldSpec, HybridMethodSpec,
    HybridModuleSpec, HybridParamSpec, HybridPropSpec, HybridStructSpec, HybridType,
    HybridViewSpec,
};

/// Rejects `<T>` parameters and `where` clauses: the IR has no generics.
pub(super) fn reject_generics(generics: &Generics) -> Result<(), CodegenError> {
    if !generics.params.is_empty() {
        return Err(error_at(
            generics.params.span(),
            "generic parameters are not supported in hybrid specs",
        ));
    }
    if let Some(w) = &generics.where_clause {
        return Err(error_at(
            w.span(),
            "`where` clauses are not supported in hybrid specs",
        ));
    }
    Ok(())
}

pub(super) fn parse_enum(
    e: &syn::ItemEnum,
    marker: &Attribute,
) -> Result<HybridEnumSpec, CodegenError> {
    reject_generics(&e.generics)?;
    let args = Args::parse(marker, keys::ENUM)?;
    let name = ident(&e.ident)?;
    let mut variants = Vec::with_capacity(e.variants.len());
    // The value an implicit discriminant takes; `None` after `u32::MAX`, where only an explicit
    // discriminant may follow.
    let mut next: Option<u32> = Some(0);
    for v in &e.variants {
        if !matches!(v.fields, syn::Fields::Unit) {
            return Err(error_at(
                v.fields.span(),
                "hybrid enums only support unit variants",
            ));
        }
        let value = match &v.discriminant {
            Some((_, expr)) => unsigned_u32(expr, "enum discriminant")?,
            None => next.ok_or_else(|| {
                error_at(
                    v.ident.span(),
                    format!("enum '{name}' discriminant overflows u32 after the previous variant"),
                )
            })?,
        };
        next = value.checked_add(1);
        variants.push(HybridEnumVariant {
            name: ident(&v.ident)?,
            value,
            doc: attrs::docs(&v.attrs),
        });
    }
    Ok(HybridEnumSpec {
        name,
        variants,
        doc: attrs::docs(&e.attrs),
        c_prefix: args.string("c_prefix")?,
        repr: attrs::repr(&e.attrs)?.int,
        dart_fallback: args.string("dart_fallback")?,
    })
}

pub(super) fn parse_record<'a>(
    name: &Ident,
    item_attrs: &[Attribute],
    marker: &Attribute,
    fields: impl Iterator<Item = &'a Field>,
    is_union: bool,
) -> Result<HybridStructSpec, CodegenError> {
    let args = Args::parse(marker, keys::RECORD)?;
    let mut out = Vec::new();
    for f in fields {
        let Some(field_name) = &f.ident else {
            return Err(error_at(f.span(), "hybrid struct fields must be named"));
        };
        out.push(HybridFieldSpec {
            name: ident(field_name)?,
            ty: convert_type(&f.ty)?,
            doc: attrs::docs(&f.attrs),
            offset: attrs::offset(&f.attrs, "offset")?,
            offset_64: attrs::offset(&f.attrs, "offset64")?,
        });
    }
    Ok(HybridStructSpec {
        name: ident(name)?,
        fields: out,
        doc: attrs::docs(item_attrs),
        is_union,
        align: args.usize("align")?.or(attrs::repr(item_attrs)?.align),
        expected_size: args.usize("size")?,
        expected_size_64: args.usize("size_64")?,
    })
}

/// A trait is a module or a view; it declares only signatures, with no generics or supertraits.
pub(super) enum TraitSpec {
    Module(HybridModuleSpec),
    View(HybridViewSpec),
}

pub(super) fn parse_trait(t: &syn::ItemTrait) -> Result<TraitSpec, CodegenError> {
    let (module, view) = (
        attrs::find(&t.attrs, "nve_hybrid_module"),
        attrs::find(&t.attrs, "nve_hybrid_view"),
    );
    let (marker, allowed) = match (module, view) {
        (Some(a), Some(_)) => {
            return Err(error_at(
                attrs::span(a),
                "trait cannot carry both #[nve_hybrid_module] and #[nve_hybrid_view]",
            ));
        }
        (Some(a), None) => (a, keys::MODULE),
        (None, Some(a)) => (a, keys::VIEW),
        (None, None) => {
            return Err(error_at(
                t.ident.span(),
                "trait in hybrid spec must be annotated with #[nve_hybrid_module] or #[nve_hybrid_view]",
            ));
        }
    };
    reject_generics(&t.generics)?;
    if !t.supertraits.is_empty() {
        return Err(error_at(
            t.supertraits.span(),
            "supertraits are not supported in hybrid specs",
        ));
    }
    let args = Args::parse(marker, allowed)?;
    let trait_name = ident(&t.ident)?;
    let name = args
        .string("name")?
        .unwrap_or_else(|| strip_spec_suffix(&trait_name));
    let explicit_interface_id = args.u32("interface_id")?;
    let doc = attrs::docs(&t.attrs);
    let mut methods = Vec::new();
    let (mut props, mut events) = (Vec::new(), Vec::new());
    for item in &t.items {
        let TraitItem::Fn(f) = item else {
            return Err(error_at(
                item.span(),
                "hybrid traits may only declare `fn` members",
            ));
        };
        if let Some(body) = &f.default {
            return Err(error_at(
                body.span(),
                "hybrid trait methods are declarations only (no default body)",
            ));
        }
        attrs::reject_unknown_markers(&f.attrs)?;
        let sig = signature(&f.sig, attrs::docs(&f.attrs), Receiver::Allowed)?;
        let prop = attrs::find(&f.attrs, "prop");
        let event = attrs::find(&f.attrs, "event");
        let roles = [
            prop.is_some(),
            event.is_some(),
            attrs::has(&f.attrs, "method"),
        ];
        if roles.iter().filter(|r| **r).count() > 1 {
            return Err(error_at(
                f.sig.ident.span(),
                "member cannot combine #[prop], #[event], and #[method]",
            ));
        }
        if module.is_some() {
            if let Some(a) = prop.or(event) {
                return Err(error_at(
                    attrs::span(a),
                    "#[prop] / #[event] are only valid inside #[nve_hybrid_view], not #[nve_hybrid_module]",
                ));
            }
            methods.push(HybridMethodSpec {
                optional: attrs::has(&f.attrs, "optional"),
                ..sig
            });
        } else if let Some(a) = prop {
            if !sig.params.is_empty() {
                return Err(error_at(
                    attrs::span(a),
                    format!(
                        "#[prop] '{}' must take only &self (no parameters)",
                        sig.name
                    ),
                ));
            }
            let default_value = Args::parse(a, keys::PROP)?
                .default()
                .map(default_value)
                .transpose()?;
            props.push(HybridPropSpec {
                name: sig.name,
                ty: sig.return_type,
                default_value,
                doc: sig.doc,
            });
        } else if let Some(a) = event {
            if sig.return_type != HybridType::Void {
                return Err(error_at(
                    attrs::span(a),
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
    Ok(match module {
        Some(_) => {
            let c_name = args.string("c_name")?;
            TraitSpec::Module(HybridModuleSpec {
                name,
                trait_name,
                methods,
                doc,
                explicit_interface_id,
                c_name,
            })
        }
        None => TraitSpec::View(HybridViewSpec {
            name,
            trait_name,
            props,
            events,
            methods,
            doc,
            explicit_interface_id,
        }),
    })
}

fn strip_spec_suffix(trait_name: &str) -> String {
    trait_name
        .strip_suffix("Spec")
        .filter(|s| !s.is_empty())
        .unwrap_or(trait_name)
        .to_owned()
}

/// Whether a signature may take `self` (trait members) or must not (C functions).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Receiver {
    Allowed,
    Forbidden,
}

/// A body-less `#[nve_hybrid_c_fn] [pub] extern "C" fn f(..) [-> T];`.
pub(super) fn c_function(
    sig: &Signature,
    doc: Option<String>,
    abi: Option<&Abi>,
) -> Result<HybridMethodSpec, CodegenError> {
    match abi {
        None => {
            return Err(error_at(
                sig.fn_token.span,
                "C functions must be declared `extern \"C\"`",
            ));
        }
        Some(abi) if abi.name.as_ref().is_some_and(|n| n.value() != "C") => {
            return Err(error_at(
                abi.span(),
                "only the `extern \"C\"` ABI is supported",
            ));
        }
        Some(_) => {}
    }
    signature(sig, doc, Receiver::Forbidden)
}

/// A method / C function signature. `async`, `const`, variadics, generics, and an `extern` ABI on
/// a trait member are rejected; parameters must be `name: Type`.
pub(super) fn signature(
    sig: &Signature,
    doc: Option<String>,
    receiver: Receiver,
) -> Result<HybridMethodSpec, CodegenError> {
    if let Some(t) = sig.asyncness {
        return Err(error_at(
            t.span,
            "`async fn` is not supported across the hybrid FFI",
        ));
    }
    if let Some(t) = sig.constness {
        return Err(error_at(
            t.span,
            "`const fn` is not supported in hybrid specs",
        ));
    }
    if let Some(v) = &sig.variadic {
        return Err(error_at(
            v.span(),
            "variadic functions (`...`) are not supported in hybrid specs",
        ));
    }
    if receiver == Receiver::Allowed
        && let Some(abi) = &sig.abi
    {
        return Err(error_at(
            abi.span(),
            "trait members cannot declare an `extern` ABI",
        ));
    }
    reject_generics(&sig.generics)?;
    let mut params = Vec::new();
    let mut has_self = false;
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) if receiver == Receiver::Forbidden => {
                return Err(error_at(r.span(), "C functions cannot take `self`"));
            }
            FnArg::Receiver(_) => has_self = true,
            FnArg::Typed(t) => {
                let Pat::Ident(name) = t.pat.as_ref() else {
                    return Err(error_at(
                        t.pat.span(),
                        "parameters must be plain identifiers",
                    ));
                };
                params.push(HybridParamSpec {
                    name: ident(&name.ident)?,
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
        name: ident(&sig.ident)?,
        params,
        return_type,
        doc,
        has_self,
        optional: false,
    })
}
