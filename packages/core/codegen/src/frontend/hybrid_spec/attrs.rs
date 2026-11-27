//! Hybrid spec attributes: outer-only lookup, docs, every `#[repr(...)]`, and each marker attribute
//! parsed once into typed arguments checked against a strict key allow-list.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{AttrStyle, Attribute, Expr, Lit, LitInt, Meta};

use super::types::{error_at, unsigned, unsigned_u32};
use crate::error::CodegenError;

/// The `nve_hybrid_*` markers this frontend understands; any other `nve_hybrid_*` is a typo.
const MARKERS: &[&str] = &[
    "nve_hybrid_enum",
    "nve_hybrid_struct",
    "nve_hybrid_union",
    "nve_hybrid_module",
    "nve_hybrid_view",
    "nve_hybrid_const",
    "nve_hybrid_alias",
    "nve_hybrid_c_fn",
    "nve_hybrid_namespace",
];

/// Value-taking keys accepted by each attribute (`key = value`).
pub(super) mod keys {
    pub const ENUM: &[&str] = &["c_prefix", "dart_fallback"];
    pub const RECORD: &[&str] = &["size", "size_64", "align"];
    pub const MODULE: &[&str] = &["name", "interface_id", "c_name"];
    pub const VIEW: &[&str] = &["name", "interface_id"];
    pub const NAMESPACE: &[&str] = &["name"];
    pub const PROP: &[&str] = &["default"];
    pub const NONE: &[&str] = &[];
}

fn outer(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|a| matches!(a.style, AttrStyle::Outer))
}

fn is_named(a: &Attribute, name: &str) -> bool {
    a.path().segments.last().is_some_and(|s| s.ident == name)
}

/// Outer attribute whose last path segment is `name` (`nve_hybrid_module`, `nve::nve_hybrid_module`).
pub(super) fn find<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    outer(attrs).find(|a| is_named(a, name))
}

pub(super) fn has(attrs: &[Attribute], name: &str) -> bool {
    find(attrs, name).is_some()
}

/// Rejects misspelled `nve_hybrid_*` markers, which would otherwise make the item silently vanish.
pub(super) fn reject_unknown_markers(attrs: &[Attribute]) -> Result<(), CodegenError> {
    for a in attrs {
        if let Some(seg) = a.path().segments.last() {
            let name = seg.ident.to_string();
            if name.starts_with("nve_hybrid_") && !MARKERS.contains(&name.as_str()) {
                return Err(error_at(
                    seg.span(),
                    format!("unknown hybrid spec attribute `#[{name}]`"),
                ));
            }
        }
    }
    Ok(())
}

/// Outer `///` and `/** */` doc lines (each trimmed), joined with `\n`. Inner `//!` docs are ignored.
pub(super) fn docs(attrs: &[Attribute]) -> Option<String> {
    let lines: Vec<String> = outer(attrs)
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

/// One attribute's `key = value` arguments, parsed once.
pub(super) struct Args {
    entries: Vec<(String, Expr)>,
}

impl Args {
    /// Parses `#[attr]` / `#[attr(key = value, ...)]`; every key must be in `allowed` and carry a
    /// value, and no key may repeat. `#[attr = value]` is rejected.
    pub(super) fn parse(attr: &Attribute, allowed: &[&str]) -> Result<Self, CodegenError> {
        let mut entries: Vec<(String, Expr)> = Vec::new();
        match &attr.meta {
            Meta::Path(_) => {}
            Meta::NameValue(nv) => {
                return Err(error_at(
                    nv.eq_token.span,
                    "use `#[attr(key = value)]`, not `#[attr = value]`",
                ));
            }
            Meta::List(_) => {
                let mut failure: Option<CodegenError> = None;
                attr.parse_nested_meta(|meta| {
                    let key = meta
                        .path
                        .get_ident()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let span = meta.path.span();
                    if !allowed.contains(&key.as_str()) {
                        failure = Some(error_at(
                            span,
                            format!(
                                "unknown key `{key}` (expected one of: {})",
                                allowed.join(", ")
                            ),
                        ));
                    } else if !meta.input.peek(syn::Token![=]) {
                        failure = Some(error_at(
                            span,
                            format!("`{key}` needs a value: `{key} = ...`"),
                        ));
                    } else if entries.iter().any(|(k, _)| *k == key) {
                        failure = Some(error_at(span, format!("`{key}` is given twice")));
                    }
                    if failure.is_some() {
                        // Consume the rest of the argument so `syn` does not report its own error first.
                        let _: proc_macro2::TokenStream = meta.input.parse()?;
                        return Ok(());
                    }
                    entries.push((key, meta.value()?.parse::<Expr>()?));
                    Ok(())
                })
                .map_err(|e| error_at(e.span(), e.to_string()))?;
                if let Some(e) = failure {
                    return Err(e);
                }
            }
        }
        Ok(Self { entries })
    }

    fn expr(&self, key: &str) -> Option<&Expr> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub(super) fn default(&self) -> Option<&Expr> {
        self.expr("default")
    }

    pub(super) fn string(&self, key: &str) -> Result<Option<String>, CodegenError> {
        match self.expr(key) {
            None => Ok(None),
            Some(Expr::Lit(syn::ExprLit {
                lit: Lit::Str(s), ..
            })) => Ok(Some(s.value())),
            Some(other) => Err(error_at(
                other.span(),
                format!("expected a string for `{key}`"),
            )),
        }
    }

    pub(super) fn u32(&self, key: &str) -> Result<Option<u32>, CodegenError> {
        self.expr(key).map(|v| unsigned_u32(v, key)).transpose()
    }

    pub(super) fn usize(&self, key: &str) -> Result<Option<usize>, CodegenError> {
        self.expr(key)
            .map(|v| {
                let n = unsigned(v, key)?;
                usize::try_from(n)
                    .map_err(|_| error_at(v.span(), format!("`{key}` {n} exceeds usize::MAX")))
            })
            .transpose()
    }
}

/// Everything every `#[repr(...)]` on an item says.
#[derive(Default)]
pub(super) struct Repr {
    /// Integer repr of an enum (`u8`, `u16`, `u32`, `i32`).
    pub(super) int: Option<String>,
    pub(super) align: Option<usize>,
}

/// Merges every outer `#[repr(...)]`: `C`, one integer repr, `align(N)`. `packed`,
/// `transparent`, and anything else is rejected.
pub(super) fn repr(attrs: &[Attribute]) -> Result<Repr, CodegenError> {
    let mut out = Repr::default();
    for attr in outer(attrs).filter(|a| is_named(a, "repr")) {
        attr.parse_nested_meta(|meta| {
            let span = meta.path.span();
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "C" => Ok(()),
                "u8" | "u16" | "u32" | "i32" => match &out.int {
                    Some(prev) => {
                        Err(meta.error(format!("conflicting integer reprs `{prev}` and `{key}`")))
                    }
                    None => {
                        out.int = Some(key);
                        Ok(())
                    }
                },
                "align" => {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let n: LitInt = content
                        .parse()
                        .map_err(|_| meta.error("expected `align(N)` with an integer N"))?;
                    if !content.is_empty() {
                        return Err(meta.error("expected `align(N)` with an integer N"));
                    }
                    let n = n.base10_parse::<usize>()?;
                    if !n.is_power_of_two() {
                        return Err(meta.error("`align(N)` needs a power of two"));
                    }
                    out.align = Some(n);
                    Ok(())
                }
                "packed" => Err(syn::Error::new(
                    span,
                    "packed layouts are not supported in hybrid specs",
                )),
                _ => Err(syn::Error::new(
                    span,
                    format!("unsupported `#[repr({key})]` in hybrid spec"),
                )),
            }
        })
        .map_err(|e| error_at(e.span(), e.to_string()))?;
    }
    Ok(out)
}

/// `#[offset(N)]` / `#[offset64(N)]` on a record field.
pub(super) fn offset(attrs: &[Attribute], name: &str) -> Result<Option<usize>, CodegenError> {
    let Some(attr) = find(attrs, name) else {
        return Ok(None);
    };
    let lit: LitInt = attr
        .parse_args()
        .map_err(|e| error_at(e.span(), e.to_string()))?;
    lit.base10_parse::<usize>()
        .map(Some)
        .map_err(|_| error_at(lit.span(), "field offset exceeds usize::MAX"))
}

/// The span to report a whole attribute at.
pub(super) fn span(attr: &Attribute) -> Span {
    attr.span()
}
