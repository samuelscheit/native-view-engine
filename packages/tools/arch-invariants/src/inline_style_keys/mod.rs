//! Repo-wide zero-tolerance ban on author-facing inline CSS `style:` RSX attributes and
//! string-typed `style` component props (zero runtime CSS string parser).
//!
//! All styling in NVE flows through typed [`TypedStyle`] (`nve_style: TypedStyle::new()...`),
//! numeric standalone attributes (`width`, `padding`, `opacity`, `z_index`, ...), or typed
//! `TableAttr` writers (`nve_table`).

mod rsx_walk;

use proc_macro2::TokenTree;
use syn::visit::{self, Visit};
use syn::{FnArg, ItemFn, ItemStruct, LitStr, Macro, Pat, Type};

use crate::{is_self_invariant_file, line_at, RepoIndex, Violation};
use rsx_walk::{scan_macro_for_nested_rsx, scan_rsx_children};

/// Roots holding RSX UI code (production, tests, examples, and hosts).
const UI_SOURCE_ROOTS: &[&str] = &["packages", "examples", "hosts"];

/// Scans one parsed Rust file for banned native-element RSX `style:` attributes (including
/// `"style":`, `r#style:`, and `rsx!` nested inside outer macros like `vec!`/`macro_rules!`),
/// embedded RSX doc snippets using `style:`, and string-typed `style` component props.
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
        let tokens: Vec<TokenTree> = mac.tokens.clone().into_iter().collect();
        if mac
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "rsx")
        {
            scan_rsx_children(self.rel, self.raw, &tokens, self.out);
        } else {
            scan_macro_for_nested_rsx(self.rel, self.raw, &tokens, self.out);
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
                    && type_is_string_like(&pat_ty.ty)
                {
                    let line = pat_ty.pat.span_line();
                    self.out.push(Violation::new(
                        self.rel,
                        line,
                        line_at(self.raw, line),
                        "#[component] string-typed `style` prop is banned; use `#[props(default)] nve_style: Option<TypedStyle>`",
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
                    && type_is_string_like(&field.ty)
                {
                    let line = field.ident.as_ref().map_or(1, |id| id.span().start().line);
                    self.out.push(Violation::new(
                        self.rel,
                        line,
                        line_at(self.raw, line),
                        "#[derive(Props)] string-typed `style` field is banned; use `nve_style: Option<TypedStyle>`",
                    ));
                }
            }
        }
        visit::visit_item_struct(self, item);
    }

    fn visit_lit_str(&mut self, lit: &'ast LitStr) {
        let val = lit.value();
        if val.contains("rsx!") && contains_banned_style_attr_snippet(&val) {
            let line = lit.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                line,
                line_at(self.raw, line),
                "embedded RSX snippet uses banned `style:` attribute; migrate example to `nve_style: TypedStyle::new()...`",
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

/// Returns `true` for `String`, `str`, `&str`, `Cow<..., str>`, `Box<str>`, `Arc<str>`, `Rc<str>`,
/// and `Option<T>` wrapping any of them.
fn type_is_string_like(ty: &Type) -> bool {
    match ty {
        Type::Reference(tr) => type_is_string_like(&tr.elem),
        Type::Group(tg) => type_is_string_like(&tg.elem),
        Type::Paren(tp) => type_is_string_like(&tp.elem),
        Type::Path(tp) => {
            let Some(last) = tp.path.segments.last() else {
                return false;
            };
            if last.ident == "String" || last.ident == "str" {
                return true;
            }
            if matches!(
                last.ident.to_string().as_str(),
                "Option" | "Box" | "Arc" | "Rc" | "Cow"
            ) && let syn::PathArguments::AngleBracketed(args) = &last.arguments
            {
                return args.args.iter().any(|arg| {
                    matches!(arg, syn::GenericArgument::Type(inner) if type_is_string_like(inner))
                });
            }
            false
        }
        _ => false,
    }
}

/// Detects `style:` / `"style":` / `r#style:` followed by a string literal or `format!` inside
/// embedded `rsx!` code snippets, ignoring `nve_style:` and non-string props like `style: ImpactStyle::Medium`.
fn contains_banned_style_attr_snippet(val: &str) -> bool {
    let bytes = val.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(rel_idx) = val[i..].find("style") else {
            break;
        };
        let start = i + rel_idx;
        let after = start + "style".len();
        let prev_ok = start == 0
            || !is_ident_byte(bytes[start - 1])
            || (start >= 2 && &bytes[start - 2..start] == b"r#");
        let mut cur = after;
        if cur < bytes.len() && bytes[cur] == b'"' && start > 0 && bytes[start - 1] == b'"' {
            cur += 1;
        }
        if prev_ok {
            while cur < bytes.len() && bytes[cur].is_ascii_whitespace() {
                cur += 1;
            }
            if cur < bytes.len() && bytes[cur] == b':' && bytes.get(cur + 1) != Some(&b':') {
                let mut rhs = cur + 1;
                while rhs < bytes.len() && bytes[rhs].is_ascii_whitespace() {
                    rhs += 1;
                }
                let rest = &val[rhs..];
                if rest.starts_with('"')
                    || rest.starts_with("r\"")
                    || rest.starts_with("r#")
                    || rest.starts_with("format!")
                {
                    return true;
                }
            }
        }
        i = after;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
    fn flags_rsx_style_attrs_bypasses_string_props_and_embedded_rsx_snippets() {
        let bad = r###"
#[component]
pub fn Card(
    #[props(default)] style: Option<String>,
) -> Element {
    let list = vec![rsx! {
        view {
            "style": "width: 100%;",
            div { r#style: "color: red;" }
            text { style: format!("padding: 4px;") }
        }
    }];
    let _ = list;
    rsx! {
        view {
            style: "width: 100%; background-color: #000;",
        }
    }
}

macro_rules! sneaky {
    () => {
        rsx! { view { style: "margin: 0;" } }
    };
}

#[derive(Props, Clone, PartialEq)]
struct LegacyProps<'a> {
    style: Cow<'a, str>,
}

#[derive(Props, Clone, PartialEq)]
struct RefProps {
    style: Option<&'static str>,
}

#[derive(Props, Clone, PartialEq)]
struct BoxProps {
    style: Box<str>,
}

const DOC_A: &str = r#"rsx! { view { style:"padding: 8px;" } }"#;
const DOC_B: &str = r##"rsx! { view { style: r#"padding: 8px;"# } }"##;
const DOC_C: &str = r#"rsx! { view { style: format!("padding: {}px;", 8) } }"#;
"###;
        let v = violations_for(bad);
        assert_eq!(v.len(), 12, "expected 12 violations, got: {v:#?}");
    }

    #[test]
    fn permits_typed_nve_style_component_enum_props_and_struct_literals_in_closures() {
        let ok = r##"
#[component]
pub fn Card(#[props(default)] nve_style: Option<TypedStyle>, style: ImpactStyle) -> Element {
    let js = r#"new Intl.NumberFormat(locale, { style: "currency", currency })"#;
    let doc_ok = r#"rsx! { Card { style: ImpactStyle::Medium, nve_style: TypedStyle::new().padding(8.0) } }"#;
    let _ = (js, doc_ok, style);
    rsx! {
        Card {
            style: ImpactStyle::Medium,
            view {
                nve_style: TypedStyle::new().width_pct(100.0).merge_opt(nve_style.as_ref()),
                onclick: move |_| {
                    let opts = Opts { style: ImpactStyle::Heavy };
                    let _ = opts;
                },
                text { "clean" }
            }
        }
    }
}
"##;
        let v = violations_for(ok);
        assert!(v.is_empty(), "expected 0 violations, got: {v:#?}");
    }
}
