//! Repo-wide zero-tolerance ban on author-facing inline CSS `style:` RSX attributes and
//! `style: Option<String>` / `style: String` component props (zero runtime CSS string parser).
//!
//! All styling in NVE flows through typed [`TypedStyle`] (`nve_style: TypedStyle::new()...`),
//! numeric standalone attributes (`width`, `padding`, `opacity`, `z_index`, ...), or typed
//! `TableAttr` writers (`nve_table`).

use proc_macro2::{Spacing, TokenStream, TokenTree};
use syn::visit::{self, Visit};
use syn::{FnArg, ItemFn, ItemStruct, LitStr, Macro, Pat, Type};

use crate::{is_self_invariant_file, line_at, RepoIndex, Violation};

/// Roots holding RSX UI code (production, tests, examples, and hosts).
const UI_SOURCE_ROOTS: &[&str] = &["packages", "examples", "hosts"];

/// Scans one parsed Rust file for banned RSX `style:` attributes, embedded RSX doc snippets
/// using `style:`, and `style: Option<String>` / `style: String` component props.
pub fn scan_banned_inline_style(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    let mut visitor = InlineStyleBanVisitor { rel, raw, out };
    visitor.visit_file(ast);
}

/// Repo-wide zero-tolerance check across `packages/`, `examples/`, and `hosts/`.
pub fn check_no_inline_css_style_attrs(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    for rel in UI_SOURCE_ROOTS.iter().flat_map(|root| idx.rs_files_under(root)) {
        if is_self_invariant_file(&rel) {
            continue;
        }
        let Some(parsed) = idx.parse_rust_if_contains(&rel, "style") else {
            continue;
        };
        scan_banned_inline_style(&rel, &parsed.raw, &parsed.ast, &mut out);
    }
    out
}

struct InlineStyleBanVisitor<'a> {
    rel: &'a str,
    raw: &'a str,
    out: &'a mut Vec<Violation>,
}

impl<'ast> Visit<'ast> for InlineStyleBanVisitor<'_> {
    fn visit_macro(&mut self, mac: &'ast Macro) {
        if mac
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "rsx")
        {
            scan_rsx_tokens(self.rel, self.raw, mac.tokens.clone(), self.out);
        }
        visit::visit_macro(self, mac);
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let is_component = item.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "component")
        });
        if is_component {
            for input in &item.sig.inputs {
                if let FnArg::Typed(pat_ty) = input
                    && pat_is_ident(&pat_ty.pat, "style")
                    && type_is_string_or_opt_string(&pat_ty.ty)
                {
                    let line = pat_ty.pat.span_line();
                    self.out.push(Violation::new(
                        self.rel,
                        line,
                        line_at(self.raw, line),
                        "#[component] prop `style: String`/`Option<String>` is banned; use `#[props(default)] nve_style: Option<TypedStyle>`",
                    ));
                }
            }
        }
        visit::visit_item_fn(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if has_props_derive(&item.attrs) {
            for field in &item.fields {
                if field.ident.as_ref().is_some_and(|id| id == "style")
                    && type_is_string_or_opt_string(&field.ty)
                {
                    let line = field.ident.as_ref().map_or(1, |id| id.span().start().line);
                    self.out.push(Violation::new(
                        self.rel,
                        line,
                        line_at(self.raw, line),
                        "#[derive(Props)] field `style: String`/`Option<String>` is banned; use `nve_style: Option<TypedStyle>`",
                    ));
                }
            }
        }
        visit::visit_item_struct(self, item);
    }

    fn visit_lit_str(&mut self, lit: &'ast LitStr) {
        let val = lit.value();
        if val.contains("rsx!") && val.contains("style: \"") {
            let line = lit.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                line,
                line_at(self.raw, line),
                "embedded RSX snippet uses banned `style: \"...\"`; migrate example to `nve_style: TypedStyle::new()...`",
            ));
        }
    }
}

trait SpanLine {
    fn span_line(&self) -> usize;
}

impl SpanLine for Pat {
    fn span_line(&self) -> usize {
        use syn::spanned::Spanned;
        self.span().start().line
    }
}

fn pat_is_ident(pat: &Pat, name: &str) -> bool {
    match pat {
        Pat::Ident(pi) => pi.ident == name,
        _ => false,
    }
}

fn has_props_derive(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.segments.last().is_some_and(|s| s.ident == "Props") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

fn type_is_string_or_opt_string(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    let Some(last) = tp.path.segments.last() else {
        return false;
    };
    if last.ident == "String" {
        return true;
    }
    if last.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return type_is_string_or_opt_string(inner);
    }
    false
}

fn scan_rsx_tokens(rel: &str, raw: &str, stream: TokenStream, out: &mut Vec<Violation>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (idx, tt) in tokens.iter().enumerate() {
        match tt {
            TokenTree::Group(g) => scan_rsx_tokens(rel, raw, g.stream(), out),
            TokenTree::Ident(id) if id == "style" => {
                if let Some(TokenTree::Punct(p)) = tokens.get(idx + 1)
                    && p.as_char() == ':'
                    && p.spacing() == Spacing::Alone
                {
                    let line = id.span().start().line;
                    out.push(Violation::new(
                        rel,
                        line,
                        line_at(raw, line),
                        "RSX `style:` attribute is banned (zero runtime CSS parser); use `nve_style: TypedStyle::new()...`",
                    ));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scan_banned_inline_style;

    fn violations_for(src: &str) -> Vec<crate::Violation> {
        let ast = syn::parse_file(src).expect("valid rust");
        let mut out = Vec::new();
        scan_banned_inline_style("test.rs", src, &ast, &mut out);
        out
    }

    #[test]
    fn flags_rsx_style_attrs_component_props_and_embedded_rsx_snippets() {
        let bad = r##"
#[component]
pub fn Card(#[props(default)] style: Option<String>) -> Element {
    rsx! {
        view {
            style: "width: 100%; background-color: #000;",
            text { style: format!("color: red;") }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct LegacyProps {
    style: String,
}

const DOC_SAMPLE: &str = r#"rsx! { view { style: "padding: 8px;" } }"#;
"##;
        let v = violations_for(bad);
        assert_eq!(v.len(), 5, "expected 5 violations, got: {v:#?}");
    }

    #[test]
    fn permits_typed_nve_style_and_non_rsx_style_identifiers() {
        let ok = r##"
#[component]
pub fn Card(#[props(default)] nve_style: Option<TypedStyle>, style: ImpactStyle) -> Element {
    let js = r#"new Intl.NumberFormat(locale, { style: "currency", currency })"#;
    let _ = (js, style);
    rsx! {
        view {
            nve_style: TypedStyle::new().width_pct(100.0).merge_opt(nve_style.as_ref()),
            text { "clean" }
        }
    }
}
"##;
        let v = violations_for(ok);
        assert!(v.is_empty(), "expected 0 violations, got: {v:#?}");
    }
}
