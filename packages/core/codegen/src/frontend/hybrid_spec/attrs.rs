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

pub(super) fn is_named(a: &Attribute, name: &str) -> bool {
    a.path().segments.last().is_some_and(|s| s.ident == name)
}

/// Outer attribute whose last path segment is `name` (`nve_hybrid_module`, `nve::nve_hybrid_module`).
pub(super) fn find<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    outer(attrs).find(|a| is_named(a, name))
}

/// Outer member / field attribute spelled exactly `#[name]` (`#[a::prop]` is not `#[prop]`).
pub(super) fn find_member<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    outer(attrs).find(|a| a.path().is_ident(name))
}

pub(super) fn has_member(attrs: &[Attribute], name: &str) -> bool {
    find_member(attrs, name).is_some()
}

/// `#[cfg]` / `#[cfg_attr]` would silently keep or drop declarations: the frontend never
/// evaluates them, so they are rejected everywhere.
fn reject_cfg(a: &Attribute) -> Result<(), CodegenError> {
    match a.path().get_ident() {
        Some(i) if i == "cfg" || i == "cfg_attr" => Err(error_at(
            a.span(),
            format!("`#[{i}]` is not evaluated in hybrid specs; remove it"),
        )),
        _ => Ok(()),
    }
}

/// The `nve_hybrid_*` markers in `attrs` are known, valid on this item kind (`allowed`,
/// described as `what`), outer (only the namespace may be inner), and each given at most once.
/// `cfg` is rejected.
pub(super) fn check_markers(
    attrs: &[Attribute],
    allowed: &[&str],
    what: &str,
) -> Result<(), CodegenError> {
    let mut seen: Vec<String> = Vec::new();
    for a in attrs {
        reject_cfg(a)?;
        let Some(seg) = a.path().segments.last() else {
            continue;
        };
        if a.path().is_ident("doc") || a.path().is_ident("repr") {
            continue;
        }
        let name = seg.ident.to_string();
        if !name.starts_with("nve_hybrid_") {
            continue;
        }
        if !MARKERS.contains(&name.as_str()) {
            return Err(error_at(
                seg.span(),
                format!("unknown hybrid spec attribute `#[{name}]`"),
            ));
        }
        if !allowed.contains(&name.as_str()) {
            return Err(error_at(
                seg.span(),
                format!("`#[{name}]` is not valid on {what}"),
            ));
        }
        if matches!(a.style, AttrStyle::Inner(_)) && name != "nve_hybrid_namespace" {
            return Err(error_at(
                a.span(),
                format!("`#![{name}]` must be an outer attribute: `#[{name}]`"),
            ));
        }
        if seen.contains(&name) {
            return Err(error_at(seg.span(), format!("`#[{name}]` is given twice")));
        }
        seen.push(name);
    }
    Ok(())
}

/// Standard single-identifier Rust attributes that are harmless on any spec member or field.
const STANDARD_ATTRS: &[&str] = &[
    "doc",
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "deprecated",
    "must_use",
];

/// Tool namespaces whose attributes (`#[rustfmt::skip]`, `#[clippy::...]`) are harmless.
const TOOL_NAMESPACES: &[&str] = &["rustfmt", "clippy"];

/// The attributes on a trait member / record field / variant / parameter: standard Rust
/// attributes, plus each of `allowed` (single identifiers) at most once. `misplaced` names
/// attributes valid elsewhere, with the message explaining where. Anything else (`#[porp]`, a
/// misplaced `#[offset]`, `#[clippy::prop]`) is an error; the first offending attribute in source
/// order is the one reported.
pub(super) fn check_member_attrs(
    attrs: &[Attribute],
    allowed: &[&str],
    misplaced: &[(&str, &str)],
    what: &str,
) -> Result<(), CodegenError> {
    let mut seen: Vec<String> = Vec::new();
    for a in attrs {
        reject_cfg(a)?;
        let path = a.path();
        let last = path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        // A tool attribute is harmless, unless it is spelled like a member attribute.
        let member_like =
            allowed.contains(&last.as_str()) || misplaced.iter().any(|(m, _)| *m == last);
        let tool = path.segments.len() > 1
            && !member_like
            && path
                .segments
                .first()
                .is_some_and(|s| TOOL_NAMESPACES.iter().any(|t| s.ident == *t));
        if tool
            || path
                .get_ident()
                .is_some_and(|i| STANDARD_ATTRS.iter().any(|s| i == *s))
        {
            continue;
        }
        if let Some((_, message)) = misplaced.iter().find(|(m, _)| path.is_ident(m)) {
            return Err(error_at(a.span(), *message));
        }
        // Member attributes are single identifiers: `#[doc::prop]` is not `#[prop]`.
        let name = path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_else(|| quote_path(path));
        if !allowed.contains(&name.as_str()) {
            return Err(error_at(
                a.span(),
                format!("unexpected attribute `#[{name}]` on {what}"),
            ));
        }
        if seen.contains(&name) {
            return Err(error_at(a.span(), format!("`#[{name}]` is given twice")));
        }
        seen.push(name);
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
                }) => Some(doc_text(&s.value())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// One doc attribute's text, trimmed. A `///` line is kept verbatim (`* bullet`, `**bold**`); in a
/// multi-line `/** ... */` comment each line's leading ` * ` / ` *` decoration is removed, and only
/// it, so Markdown and code-block indentation after it survive.
fn doc_text(raw: &str) -> String {
    if !raw.contains('\n') {
        return raw.trim().to_owned();
    }
    let lines: Vec<&str> = raw
        .lines()
        .map(|l| {
            let t = l.trim_start();
            match t.strip_prefix("* ") {
                Some(rest) => rest.trim_end(),
                None if t.trim_end() == "*" => "",
                None => t.trim_end(),
            }
        })
        .collect();
    lines.join("\n").trim_matches('\n').to_owned()
}

/// `a::b` for an attribute path in an error message.
fn quote_path(path: &syn::Path) -> String {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segments.join("::")
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
            Meta::NameValue(nv) if allowed.is_empty() => {
                return Err(error_at(
                    nv.eq_token.span,
                    "this attribute takes no arguments",
                ));
            }
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
                    if allowed.is_empty() {
                        failure = Some(error_at(span, "this attribute takes no arguments"));
                    } else if meta.path.get_ident().is_none() {
                        failure = Some(error_at(span, "attribute keys are plain identifiers"));
                    } else if !allowed.contains(&key.as_str()) {
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
    for attr in outer(attrs).filter(|a| a.path().is_ident("repr")) {
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
                    Some(prev) if *prev == key => {
                        Err(meta.error(format!("`repr({key})` is given twice")))
                    }
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
                    if out.align.replace(n).is_some() {
                        return Err(meta.error("`align(N)` is given twice"));
                    }
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
    let Some(attr) = find_member(attrs, name) else {
        return Ok(None);
    };
    let lit: LitInt = attr.parse_args().map_err(|e| {
        error_at(
            e.span(),
            format!("expected `#[{name}(N)]` with a non-negative integer N: {e}"),
        )
    })?;
    lit.base10_parse::<usize>().map(Some).map_err(|_| {
        error_at(
            lit.span(),
            format!("expected `#[{name}(N)]` with a non-negative integer N that fits usize"),
        )
    })
}

/// The span to report a whole attribute at.
pub(super) fn span(attr: &Attribute) -> Span {
    attr.span()
}
