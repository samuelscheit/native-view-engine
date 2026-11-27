//! Binding-pattern, macro/format-string, type, operator, and mirror-atom normalization helpers
//! for `ast_enums::normalize`.

use std::collections::BTreeMap;

use proc_macro2::{TokenStream, TokenTree};
use syn::{Expr, Lit, Pat, Stmt, Type};

use super::normalize::NormCtx;
use super::patterns::{
    canonical_path, lit_to_key_and_atom, member_to_string, split_enum_variant_path, MirrorAtom,
};
use super::scope::{is_type_ident, ImportScope};

pub(super) fn is_rust_keyword_or_primitive(s: &str) -> bool {
    matches!(
        s,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
    )
}

/// Shared inline format-capture scanner (`"{var}"`, `"{var:?}"`, skipping `"{{...}}"`).
/// Rewrites captures via `on_capture(ident, spec)` when `out` is `Some`, or purely observes when `None`.
pub(super) fn scan_format_captures(
    s: &str,
    mut out: Option<&mut String>,
    mut on_capture: impl FnMut(&str, &str) -> String,
) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                if let Some(buf) = out.as_deref_mut() {
                    buf.push_str("{{");
                }
                i += 2;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'}' {
                end += 1;
            }
            if end < bytes.len() {
                let inner = &s[start..end];
                let (ident, spec) = match inner.find(':') {
                    Some(colon) => (&inner[..colon], &inner[colon..]),
                    None => (inner, ""),
                };
                if !ident.is_empty()
                    && ident
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                    && ident
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    let replaced = on_capture(ident, spec);
                    if let Some(buf) = out.as_deref_mut() {
                        buf.push('{');
                        buf.push_str(&replaced);
                        buf.push_str(spec);
                        buf.push('}');
                    }
                    i = end + 1;
                    continue;
                }
            }
        }
        if let Some(ch) = s[i..].chars().next() {
            if let Some(buf) = out.as_deref_mut() {
                buf.push(ch);
            }
            i += ch.len_utf8();
        } else {
            break;
        }
    }
}

impl NormCtx<'_> {
    pub(super) fn norm_binding_pat(&mut self, pat: &Pat) -> String {
        self.norm_binding_pat_inner(pat, None)
    }

    fn norm_binding_pat_inner(
        &mut self,
        pat: &Pat,
        mut or_shared: Option<&mut BTreeMap<String, usize>>,
    ) -> String {
        match pat {
            Pat::Wild(_) => "_".to_string(),
            Pat::Rest(_) => "..".to_string(),
            Pat::Type(t) => self.norm_binding_pat_inner(&t.pat, or_shared),
            Pat::Paren(p) => self.norm_binding_pat_inner(&p.pat, or_shared),
            Pat::Reference(r) => self.norm_binding_pat_inner(&r.pat, or_shared),
            Pat::Lit(l) => lit_to_key_and_atom(&l.lit, false)
                .map(|(k, _)| k)
                .unwrap_or_else(|| self.outer.fresh_opaque("patlit")),
            Pat::Path(p) => canonical_path(&p.path, self.scope(), self.self_ty),
            Pat::Ident(pi) => {
                let name = pi.ident.to_string();
                if pi.by_ref.is_none()
                    && pi.mutability.is_none()
                    && pi.subpat.is_none()
                    && is_type_ident(&name)
                {
                    if let Some((qual, variant, _)) = self.scope().resolve_unqualified_variant(&name)
                    {
                        return match qual {
                            Some(q) => format!("{}::{variant}", q.canonical_key()),
                            None => variant,
                        };
                    }
                    if let Some(q) = self.scope().type_aliases.get(&name) {
                        return q.canonical_key();
                    }
                    return name;
                }
                let idx = if let Some(shared) = or_shared.as_deref_mut() {
                    *shared.entry(name.clone()).or_insert_with(|| {
                        let id = self.next_local;
                        self.next_local += 1;
                        id
                    })
                } else {
                    let id = self.next_local;
                    self.next_local += 1;
                    id
                };
                self.local_bindings.insert(name, idx);
                if let Some((_, sub)) = &pi.subpat {
                    format!("$l{idx}@{}", self.norm_binding_pat_inner(sub, or_shared))
                } else {
                    format!("$l{idx}")
                }
            }
            Pat::Tuple(t) => {
                let ps: Vec<String> = t
                    .elems
                    .iter()
                    .map(|e| self.norm_binding_pat_inner(e, or_shared.as_deref_mut()))
                    .collect();
                format!("({})", ps.join(","))
            }
            Pat::Slice(s) => {
                let ps: Vec<String> = s
                    .elems
                    .iter()
                    .map(|e| self.norm_binding_pat_inner(e, or_shared.as_deref_mut()))
                    .collect();
                format!("[{}]", ps.join(","))
            }
            Pat::TupleStruct(ts) => {
                let path = canonical_path(&ts.path, self.scope(), self.self_ty);
                let ps: Vec<String> = ts
                    .elems
                    .iter()
                    .map(|e| self.norm_binding_pat_inner(e, or_shared.as_deref_mut()))
                    .collect();
                format!("{path}({})", ps.join(","))
            }
            Pat::Struct(ps) => {
                let path = canonical_path(&ps.path, self.scope(), self.self_ty);
                let mut fields: Vec<(String, &Pat)> = ps
                    .fields
                    .iter()
                    .map(|fp| (member_to_string(&fp.member), &*fp.pat))
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                let mut parts: Vec<String> = fields
                    .into_iter()
                    .map(|(f, p)| {
                        format!(
                            "{f}:{}",
                            self.norm_binding_pat_inner(p, or_shared.as_deref_mut())
                        )
                    })
                    .collect();
                if ps.rest.is_some() {
                    parts.push("..".to_string());
                }
                format!("{path}{{{}}}", parts.join(","))
            }
            Pat::Or(or) => {
                // Rust requires identical binding names across `Pat::Or` alternatives; share `$lN`
                // and write back into any enclosing `or_shared` so `(A(x) | B(x)) | C(x)` aligns.
                let base_locals = self.local_bindings.clone();
                let mut shared = match &or_shared {
                    Some(existing) => (*existing).clone(),
                    None => BTreeMap::new(),
                };
                let mut cases: Vec<String> = or
                    .cases
                    .iter()
                    .map(|c| {
                        self.local_bindings = base_locals.clone();
                        self.norm_binding_pat_inner(c, Some(&mut shared))
                    })
                    .collect();
                self.local_bindings = base_locals;
                if let Some(outer_shared) = or_shared {
                    for (name, idx) in shared {
                        outer_shared.entry(name.clone()).or_insert(idx);
                        self.local_bindings.insert(name, idx);
                    }
                } else {
                    for (name, idx) in shared {
                        self.local_bindings.insert(name, idx);
                    }
                }
                cases.sort();
                cases.join("|")
            }
            Pat::Range(r) => {
                let s = r.start.as_deref().map(|e| self.norm_expr(e)).unwrap_or_default();
                let e = r.end.as_deref().map(|e| self.norm_expr(e)).unwrap_or_default();
                let op = match r.limits {
                    syn::RangeLimits::HalfOpen(_) => "..",
                    syn::RangeLimits::Closed(_) => "..=",
                };
                format!("{s}{op}{e}")
            }
            _ => self.outer.fresh_opaque("pat"),
        }
    }

    pub(super) fn norm_macro(&mut self, mac: &syn::Macro) -> String {
        let name = canonical_path(&mac.path, self.scope(), self.self_ty);
        let fmt_arg_idx = match mac.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
            Some("write" | "writeln") => Some(1usize),
            Some("format" | "format_args" | "print" | "println" | "eprint" | "eprintln") => {
                Some(0usize)
            }
            _ => None,
        };
        if let Ok(args) = mac.parse_body_with(
            syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
        ) {
            let norm_args: Vec<String> = args
                .iter()
                .enumerate()
                .map(|(idx, e)| {
                    if fmt_arg_idx == Some(idx)
                        && let Expr::Lit(l) = e
                        && let Lit::Str(s) = &l.lit
                    {
                        return format!("{:?}", self.norm_format_str(&s.value()));
                    }
                    self.norm_expr(e)
                })
                .collect();
            return format!("{name}!({})", norm_args.join(","));
        }
        let body = self.norm_token_stream(mac.tokens.clone());
        format!("{name}!({body})")
    }

    fn norm_format_str(&mut self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        scan_format_captures(s, Some(&mut out), |ident, _| self.resolve_var(ident));
        out
    }

    fn norm_token_stream(&mut self, ts: TokenStream) -> String {
        let mut out = String::new();
        let mut prev_member_sep = false;
        for tt in ts {
            match tt {
                TokenTree::Ident(id) => {
                    let s = id.to_string();
                    if prev_member_sep
                        || s.starts_with(|c: char| c.is_ascii_uppercase())
                        || is_rust_keyword_or_primitive(&s)
                    {
                        out.push_str(&s);
                    } else {
                        out.push_str(&self.resolve_var(&s));
                    }
                    out.push(' ');
                    prev_member_sep = false;
                }
                TokenTree::Group(g) => {
                    out.push('(');
                    out.push_str(&self.norm_token_stream(g.stream()));
                    out.push(')');
                    prev_member_sep = false;
                }
                TokenTree::Literal(l) => {
                    out.push_str(&l.to_string());
                    out.push(' ');
                    prev_member_sep = false;
                }
                TokenTree::Punct(p) => {
                    let ch = p.as_char();
                    out.push(ch);
                    prev_member_sep = ch == '.' || ch == ':';
                }
            }
        }
        out.trim().to_string()
    }
}

pub(super) fn extract_body_mirror_atom(
    expr: &Expr,
    scope: &ImportScope,
    self_ty: Option<&str>,
) -> Option<MirrorAtom> {
    match expr {
        Expr::Lit(l) => lit_to_key_and_atom(&l.lit, false).map(|(_, a)| a),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => match &*u.expr {
            Expr::Lit(l) => lit_to_key_and_atom(&l.lit, true).map(|(_, a)| a),
            _ => None,
        },
        Expr::Path(p) => {
            let (q, v) = split_enum_variant_path(&p.path, scope, self_ty)?;
            Some(MirrorAtom::Variant {
                qualifier: q,
                variant: v,
            })
        }
        Expr::Call(c) => {
            if let Expr::Path(p) = &*c.func
                && let Some(last) = p.path.segments.last()
                && matches!(last.ident.to_string().as_str(), "Some" | "Ok")
                && c.args.len() == 1
            {
                return extract_body_mirror_atom(c.args.first()?, scope, self_ty);
            }
            None
        }
        Expr::Return(r) => r
            .expr
            .as_deref()
            .and_then(|e| extract_body_mirror_atom(e, scope, self_ty)),
        Expr::Block(b) if b.label.is_none() && b.block.stmts.len() == 1 => {
            if let Stmt::Expr(e, _) = &b.block.stmts[0] {
                extract_body_mirror_atom(e, scope, self_ty)
            } else {
                None
            }
        }
        Expr::Paren(p) => extract_body_mirror_atom(&p.expr, scope, self_ty),
        Expr::Group(g) => extract_body_mirror_atom(&g.expr, scope, self_ty),
        _ => None,
    }
}

pub(super) fn is_noop_body(body: &str) -> bool {
    matches!(
        body,
        "" | "()"
            | "continue"
            | "break"
            | "return"
            | "None"
            | "Option::None"
            | "return None"
            | "Ok(())"
            | "return Ok(())"
            | "unreachable!()"
            | "todo!()"
            | "unimplemented!()"
    ) || is_abort_body(body)
}

/// True for aborting wildcard fallbacks (`unreachable!(...)`, `panic!(...)`, `todo!(...)`,
/// `unimplemented!(...)`) that indicate unreachable variants rather than a semantic default value.
pub(super) fn is_abort_body(body: &str) -> bool {
    body.starts_with("unreachable!(")
        || body.starts_with("panic!(")
        || body.starts_with("todo!(")
        || body.starts_with("unimplemented!(")
}

/// True when an arm body consists solely of a single bare outer variable (`$r0`, `$r1`, ...).
pub(super) fn is_bare_outer_var(body: &str) -> bool {
    body.strip_prefix("$r")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

pub(super) fn norm_type(ty: &Type) -> String {
    match ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "_".to_string()),
        Type::Reference(r) => format!("&{}", norm_type(&r.elem)),
        Type::Slice(s) => format!("[{}]", norm_type(&s.elem)),
        Type::Tuple(t) => {
            let es: Vec<String> = t.elems.iter().map(norm_type).collect();
            format!("({})", es.join(","))
        }
        _ => "_".to_string(),
    }
}

pub(super) fn norm_generic_arg(arg: &syn::GenericArgument) -> String {
    match arg {
        syn::GenericArgument::Type(t) => norm_type(t),
        _ => "_".to_string(),
    }
}

pub(super) fn bin_op_str(op: &syn::BinOp) -> &'static str {
    match op {
        syn::BinOp::Add(_) => "+",
        syn::BinOp::Sub(_) => "-",
        syn::BinOp::Mul(_) => "*",
        syn::BinOp::Div(_) => "/",
        syn::BinOp::Rem(_) => "%",
        syn::BinOp::And(_) => "&&",
        syn::BinOp::Or(_) => "||",
        syn::BinOp::BitXor(_) => "^",
        syn::BinOp::BitAnd(_) => "&",
        syn::BinOp::BitOr(_) => "|",
        syn::BinOp::Shl(_) => "<<",
        syn::BinOp::Shr(_) => ">>",
        syn::BinOp::Eq(_) => "==",
        syn::BinOp::Lt(_) => "<",
        syn::BinOp::Le(_) => "<=",
        syn::BinOp::Ne(_) => "!=",
        syn::BinOp::Ge(_) => ">=",
        syn::BinOp::Gt(_) => ">",
        syn::BinOp::AddAssign(_) => "+=",
        syn::BinOp::SubAssign(_) => "-=",
        syn::BinOp::MulAssign(_) => "*=",
        syn::BinOp::DivAssign(_) => "/=",
        syn::BinOp::RemAssign(_) => "%=",
        syn::BinOp::BitXorAssign(_) => "^=",
        syn::BinOp::BitAndAssign(_) => "&=",
        syn::BinOp::BitOrAssign(_) => "|=",
        syn::BinOp::ShlAssign(_) => "<<=",
        syn::BinOp::ShrAssign(_) => ">>=",
        _ => "?",
    }
}
