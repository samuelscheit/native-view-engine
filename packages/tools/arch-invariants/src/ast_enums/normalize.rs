//! Body / expression / block / macro normalization with per-arm alpha-renaming for `ast_enums`.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{TokenStream, TokenTree};
use syn::{Expr, Lit, Pat, Stmt, Type};

use super::patterns::{
    canonical_path, lit_to_key_and_atom, member_to_string, split_enum_variant_path, ImportScope,
    MirrorAtom,
};

pub(super) struct NormCtx<'a> {
    scope: &'a ImportScope,
    self_ty: Option<&'a str>,
    pat_bindings: BTreeMap<String, usize>,
    used_pat_bindings: BTreeSet<usize>,
    local_bindings: BTreeMap<String, usize>,
    outer_vars: BTreeMap<String, usize>,
    pub referenced_enum_qualifiers: BTreeSet<String>,
}

impl<'a> NormCtx<'a> {
    pub(super) fn new(scope: &'a ImportScope, self_ty: Option<&'a str>, pat_vars: &[String]) -> Self {
        let mut pat_bindings = BTreeMap::new();
        for (idx, name) in pat_vars.iter().enumerate() {
            pat_bindings.insert(name.clone(), idx);
        }
        Self {
            scope,
            self_ty,
            pat_bindings,
            used_pat_bindings: BTreeSet::new(),
            local_bindings: BTreeMap::new(),
            outer_vars: BTreeMap::new(),
            referenced_enum_qualifiers: BTreeSet::new(),
        }
    }

    pub(super) fn finalize_pat_key(&self, raw_pat_key: &str, guard: Option<&str>) -> String {
        let mut out = if !raw_pat_key.contains("$p") {
            raw_pat_key.to_string()
        } else {
            let mut s = String::with_capacity(raw_pat_key.len());
            let bytes = raw_pat_key.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if i + 2 < bytes.len()
                    && bytes[i] == b'$'
                    && bytes[i + 1] == b'p'
                    && bytes[i + 2].is_ascii_digit()
                {
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    if let Ok(idx) = raw_pat_key[i + 2..j].parse::<usize>() {
                        if self.used_pat_bindings.contains(&idx) {
                            s.push_str(&raw_pat_key[i..j]);
                        } else {
                            s.push('_');
                        }
                        i = j;
                        continue;
                    }
                }
                if let Some(ch) = raw_pat_key[i..].chars().next() {
                    s.push(ch);
                    i += ch.len_utf8();
                } else {
                    break;
                }
            }
            s
        };
        if let Some(g) = guard {
            out.push_str(" if ");
            out.push_str(g);
        }
        out
    }

    pub(super) fn norm_arm_body(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Return(r) => match &r.expr {
                Some(inner) => self.norm_expr(inner),
                None => "return".to_string(),
            },
            Expr::Block(b) if b.label.is_none() => self.norm_block_expr(&b.block),
            Expr::Paren(p) => self.norm_arm_body(&p.expr),
            Expr::Group(g) => self.norm_arm_body(&g.expr),
            _ => self.norm_expr(expr),
        }
    }

    fn norm_block_expr(&mut self, block: &syn::Block) -> String {
        if block.stmts.is_empty() {
            return "()".to_string();
        }
        if block.stmts.len() == 1 {
            if let Stmt::Expr(e, _) = &block.stmts[0] {
                return self.norm_arm_body(e);
            }
            if let Stmt::Macro(sm) = &block.stmts[0] {
                return self.norm_macro(&sm.mac);
            }
        }
        let mut parts = Vec::with_capacity(block.stmts.len());
        for stmt in &block.stmts {
            match stmt {
                Stmt::Local(local) => {
                    let init_str = local
                        .init
                        .as_ref()
                        .map(|init| {
                            let e = self.norm_expr(&init.expr);
                            if let Some((_, div)) = &init.diverge {
                                format!("{e} else {}", self.norm_expr(div))
                            } else {
                                e
                            }
                        })
                        .unwrap_or_default();
                    let pat_str = self.bind_local_pat(&local.pat);
                    parts.push(format!("let {pat_str}={init_str}"));
                }
                Stmt::Expr(e, _) => parts.push(self.norm_expr(e)),
                Stmt::Macro(sm) => parts.push(self.norm_macro(&sm.mac)),
                Stmt::Item(_) => {}
            }
        }
        if parts.is_empty() {
            "()".to_string()
        } else if parts.len() == 1 {
            parts.remove(0)
        } else {
            format!("{{{}}}", parts.join(";"))
        }
    }

    fn bind_local_pat(&mut self, pat: &Pat) -> String {
        match pat {
            Pat::Ident(pi) => {
                let name = pi.ident.to_string();
                let idx = self.local_bindings.len();
                self.local_bindings.insert(name, idx);
                format!("$l{idx}")
            }
            Pat::Wild(_) => "_".to_string(),
            Pat::Type(t) => self.bind_local_pat(&t.pat),
            Pat::Paren(p) => self.bind_local_pat(&p.pat),
            Pat::Reference(r) => self.bind_local_pat(&r.pat),
            Pat::Tuple(t) => {
                let ps: Vec<String> = t.elems.iter().map(|e| self.bind_local_pat(e)).collect();
                format!("({})", ps.join(","))
            }
            _ => "_".to_string(),
        }
    }

    fn resolve_var(&mut self, name: &str) -> String {
        if let Some(&idx) = self.pat_bindings.get(name) {
            self.used_pat_bindings.insert(idx);
            return format!("$p{idx}");
        }
        if let Some(&idx) = self.local_bindings.get(name) {
            return format!("$l{idx}");
        }
        let next = self.outer_vars.len();
        let idx = *self.outer_vars.entry(name.to_string()).or_insert(next);
        format!("$r{idx}")
    }

    pub(super) fn norm_expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Lit(l) => lit_to_key_and_atom(&l.lit, false)
                .map(|(k, _)| k)
                .unwrap_or_else(|| "lit".to_string()),
            Expr::Unary(u) => {
                if matches!(u.op, syn::UnOp::Neg(_))
                    && let Expr::Lit(l) = &*u.expr
                    && let Some((k, _)) = lit_to_key_and_atom(&l.lit, true)
                {
                    return k;
                }
                let op = match u.op {
                    syn::UnOp::Deref(_) => "*",
                    syn::UnOp::Not(_) => "!",
                    syn::UnOp::Neg(_) => "-",
                    _ => "?",
                };
                format!("({op}{})", self.norm_expr(&u.expr))
            }
            Expr::Path(p) => {
                if p.qself.is_none() && p.path.segments.len() == 1 {
                    let name = p.path.segments[0].ident.to_string();
                    if name == "self" || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
                        return self.resolve_var(&name);
                    }
                    if let Some((q, v)) = self.scope.variant_imports.get(&name) {
                        self.referenced_enum_qualifiers.insert(q.clone());
                        return format!("{q}::{v}");
                    }
                    return name;
                }
                if let Some((Some(q), _)) =
                    split_enum_variant_path(&p.path, self.scope, self.self_ty)
                {
                    self.referenced_enum_qualifiers.insert(q);
                }
                canonical_path(&p.path, self.scope, self.self_ty)
            }
            Expr::Assign(a) => {
                format!("({}={})", self.norm_expr(&a.left), self.norm_expr(&a.right))
            }
            Expr::Binary(b) => {
                let op = bin_op_str(&b.op);
                format!("({}{op}{})", self.norm_expr(&b.left), self.norm_expr(&b.right))
            }
            Expr::Field(f) => format!("{}.{}", self.norm_expr(&f.base), member_to_string(&f.member)),
            Expr::Index(i) => format!("{}[{}]", self.norm_expr(&i.expr), self.norm_expr(&i.index)),
            Expr::MethodCall(mc) => {
                let recv = self.norm_expr(&mc.receiver);
                let args: Vec<String> = mc.args.iter().map(|a| self.norm_expr(a)).collect();
                let turbo = mc
                    .turbofish
                    .as_ref()
                    .map(|tf| {
                        let ts: Vec<String> = tf.args.iter().map(norm_generic_arg).collect();
                        format!("::<{}>", ts.join(","))
                    })
                    .unwrap_or_default();
                format!("{recv}.{}{turbo}({})", mc.method, args.join(","))
            }
            Expr::Call(c) => {
                let callee = match &*c.func {
                    Expr::Path(p) => {
                        if let Some((Some(q), _)) =
                            split_enum_variant_path(&p.path, self.scope, self.self_ty)
                        {
                            self.referenced_enum_qualifiers.insert(q);
                        }
                        canonical_path(&p.path, self.scope, self.self_ty)
                    }
                    other => self.norm_expr(other),
                };
                let args: Vec<String> = c.args.iter().map(|a| self.norm_expr(a)).collect();
                format!("{callee}({})", args.join(","))
            }
            Expr::Macro(m) => self.norm_macro(&m.mac),
            Expr::Tuple(t) => {
                let elems: Vec<String> = t.elems.iter().map(|e| self.norm_expr(e)).collect();
                format!("({})", elems.join(","))
            }
            Expr::Array(a) => {
                let elems: Vec<String> = a.elems.iter().map(|e| self.norm_expr(e)).collect();
                format!("[{}]", elems.join(","))
            }
            Expr::Repeat(r) => format!("[{};{}]", self.norm_expr(&r.expr), self.norm_expr(&r.len)),
            Expr::Struct(s) => {
                if let Some((Some(q), _)) =
                    split_enum_variant_path(&s.path, self.scope, self.self_ty)
                {
                    self.referenced_enum_qualifiers.insert(q);
                }
                let ty = canonical_path(&s.path, self.scope, self.self_ty);
                let mut fields: Vec<(String, String)> = s
                    .fields
                    .iter()
                    .map(|f| (member_to_string(&f.member), self.norm_expr(&f.expr)))
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                let mut parts: Vec<String> = fields
                    .into_iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect();
                if let Some(rest) = &s.rest {
                    parts.push(format!("..{}", self.norm_expr(rest)));
                }
                format!("{ty}{{{}}}", parts.join(","))
            }
            Expr::Cast(c) => format!("({} as {})", self.norm_expr(&c.expr), norm_type(&c.ty)),
            Expr::Reference(r) => {
                let m = if r.mutability.is_some() { "&mut " } else { "&" };
                format!("{m}{}", self.norm_expr(&r.expr))
            }
            Expr::Block(b) => self.norm_block_expr(&b.block),
            Expr::If(i) => {
                let cond = self.norm_expr(&i.cond);
                let then_b = self.norm_block_expr(&i.then_branch);
                let else_b = i
                    .else_branch
                    .as_ref()
                    .map(|(_, e)| format!(" else {}", self.norm_expr(e)))
                    .unwrap_or_default();
                format!("if {cond}{{{then_b}}}{else_b}")
            }
            Expr::Return(r) => match &r.expr {
                Some(e) => format!("return {}", self.norm_expr(e)),
                None => "return".to_string(),
            },
            Expr::Break(b) => match &b.expr {
                Some(e) => format!("break {}", self.norm_expr(e)),
                None => "break".to_string(),
            },
            Expr::Continue(_) => "continue".to_string(),
            Expr::Try(t) => format!("{}?", self.norm_expr(&t.expr)),
            Expr::Paren(p) => self.norm_expr(&p.expr),
            Expr::Group(g) => self.norm_expr(&g.expr),
            Expr::Range(r) => {
                let s = r.start.as_deref().map(|e| self.norm_expr(e)).unwrap_or_default();
                let e = r.end.as_deref().map(|e| self.norm_expr(e)).unwrap_or_default();
                let op = match r.limits {
                    syn::RangeLimits::HalfOpen(_) => "..",
                    syn::RangeLimits::Closed(_) => "..=",
                };
                format!("{s}{op}{e}")
            }
            Expr::Closure(cl) => {
                let ps: Vec<String> = cl.inputs.iter().map(|p| self.bind_local_pat(p)).collect();
                format!("|{}|{}", ps.join(","), self.norm_expr(&cl.body))
            }
            Expr::Match(m) => {
                let scr = self.norm_expr(&m.expr);
                let mut arms: Vec<String> = m
                    .arms
                    .iter()
                    .map(|a| format!("{}={}", norm_pat_compact(&a.pat), self.norm_expr(&a.body)))
                    .collect();
                arms.sort();
                format!("match {scr}{{{}}}", arms.join(","))
            }
            Expr::Unsafe(u) => format!("unsafe{{{}}}", self.norm_block_expr(&u.block)),
            Expr::Await(a) => format!("{}.await", self.norm_expr(&a.base)),
            Expr::Let(l) => format!("let {}={}", self.bind_local_pat(&l.pat), self.norm_expr(&l.expr)),
            _ => "other".to_string(),
        }
    }

    fn norm_macro(&mut self, mac: &syn::Macro) -> String {
        let name = canonical_path(&mac.path, self.scope, self.self_ty);
        let fmt_arg_idx = match mac.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
            Some("write" | "writeln") => Some(1usize),
            Some("format" | "format_args" | "print" | "println" | "eprint" | "eprintln") => Some(0usize),
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
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    out.push_str("{{");
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
                        out.push('{');
                        out.push_str(&self.resolve_var(ident));
                        out.push_str(spec);
                        out.push('}');
                        i = end + 1;
                        continue;
                    }
                }
            }
            if let Some(ch) = s[i..].chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
        }
        out
    }

    fn norm_token_stream(&mut self, ts: TokenStream) -> String {
        let mut out = String::new();
        for tt in ts {
            match tt {
                TokenTree::Ident(id) => {
                    let s = id.to_string();
                    if s.starts_with(|c: char| c.is_ascii_uppercase())
                        || matches!(
                            s.as_str(),
                            "true" | "false" | "if" | "else" | "match" | "let" | "in" | "as"
                        )
                    {
                        out.push_str(&s);
                    } else {
                        out.push_str(&self.resolve_var(&s));
                    }
                    out.push(' ');
                }
                TokenTree::Group(g) => {
                    out.push('(');
                    out.push_str(&self.norm_token_stream(g.stream()));
                    out.push(')');
                }
                TokenTree::Literal(l) => {
                    out.push_str(&l.to_string());
                    out.push(' ');
                }
                TokenTree::Punct(p) => out.push(p.as_char()),
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
    )
}

fn norm_type(ty: &Type) -> String {
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

fn norm_generic_arg(arg: &syn::GenericArgument) -> String {
    match arg {
        syn::GenericArgument::Type(t) => norm_type(t),
        _ => "_".to_string(),
    }
}

fn norm_pat_compact(pat: &Pat) -> String {
    match pat {
        Pat::Wild(_) => "_".to_string(),
        Pat::Ident(pi) => pi.ident.to_string(),
        Pat::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Pat::Lit(l) => lit_to_key_and_atom(&l.lit, false)
            .map(|(k, _)| k)
            .unwrap_or_else(|| "lit".to_string()),
        _ => "_".to_string(),
    }
}

fn bin_op_str(op: &syn::BinOp) -> &'static str {
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
