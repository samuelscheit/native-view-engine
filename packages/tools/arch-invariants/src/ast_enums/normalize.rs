//! Core expression/statement/block normalization with per-match outer-var alpha-renaming,
//! monotonic scoped local variable tracking, and block-scoped `use`/`type` ingestion.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Expr, Stmt};

use super::normalize_helpers::{bin_op_str, norm_generic_arg, norm_type};
use super::patterns::{
    canonical_path, lit_to_key_and_atom, member_to_string, rewrite_pat_placeholders,
    split_enum_variant_path,
};
use super::scope::{EnumQualifier, ImportScope};

/// Shared per-match outer-variable scope so the same outer variable keeps a single canonical
/// `$rN` index across every arm of the match.
///
/// Pre-seeding from enclosing declaration order (`enclosing_locals`) establishes declaration-order
/// positional alpha-equivalence: same-function or same-order swapped tables (`Top => top + 1` vs
/// `bottom + 1`, swapped corners) map to distinct `$rN` indices and never falsely collide, while
/// two separate functions with permuted parameter declarations remain alpha-equivalent when their
/// non-bare bodies apply identical operations to positional parameters (pure bare outer-var
/// selectors remain rejected unconditionally via `is_bare_outer_var`).
#[derive(Debug, Default)]
pub(super) struct MatchOuterScope {
    outer_vars: BTreeMap<String, usize>,
    next_opaque: usize,
}

impl MatchOuterScope {
    pub(super) fn new(opaque_seed: usize) -> Self {
        Self {
            outer_vars: BTreeMap::new(),
            next_opaque: opaque_seed,
        }
    }

    pub(super) fn next_opaque_seed(&self) -> usize {
        self.next_opaque
    }

    pub(super) fn preseed_from_referenced(
        &mut self,
        enclosing_locals: &[String],
        referenced: &BTreeSet<String>,
    ) {
        for name in enclosing_locals {
            if referenced.contains(name) && !self.outer_vars.contains_key(name) {
                let idx = self.outer_vars.len();
                self.outer_vars.insert(name.clone(), idx);
            }
        }
    }

    pub(super) fn resolve_outer(&mut self, name: &str) -> String {
        let next = self.outer_vars.len();
        let idx = *self.outer_vars.entry(name.to_string()).or_insert(next);
        format!("$r{idx}")
    }

    pub(super) fn fresh_opaque(&mut self, tag: &str) -> String {
        let id = self.next_opaque;
        self.next_opaque += 1;
        format!("$opaque_{tag}#{id}")
    }
}

pub(super) struct NormCtx<'a> {
    base_scope: &'a ImportScope,
    scope_stack: Vec<ImportScope>,
    pub(super) self_ty: Option<&'a str>,
    pat_bindings: BTreeMap<String, usize>,
    used_pat_bindings: BTreeSet<usize>,
    pub(super) local_bindings: BTreeMap<String, usize>,
    pub(super) next_local: usize,
    pub(super) outer: &'a mut MatchOuterScope,
    pub referenced_enum_qualifiers: BTreeSet<EnumQualifier>,
    pub has_unqualified_enum_ref: bool,
}

impl<'a> NormCtx<'a> {
    pub(super) fn new(
        scope: &'a ImportScope,
        self_ty: Option<&'a str>,
        pat_vars: &[String],
        outer: &'a mut MatchOuterScope,
    ) -> Self {
        let mut pat_bindings = BTreeMap::new();
        for (idx, name) in pat_vars.iter().enumerate() {
            pat_bindings.insert(name.clone(), idx);
        }
        Self {
            base_scope: scope,
            scope_stack: Vec::new(),
            self_ty,
            pat_bindings,
            used_pat_bindings: BTreeSet::new(),
            local_bindings: BTreeMap::new(),
            next_local: 0,
            outer,
            referenced_enum_qualifiers: BTreeSet::new(),
            has_unqualified_enum_ref: false,
        }
    }

    pub(super) fn scope(&self) -> &ImportScope {
        self.scope_stack.last().unwrap_or(self.base_scope)
    }

    pub(super) fn finalize_pat_key(&self, raw_pat_key: &str, guard: Option<&str>) -> String {
        let mut out = rewrite_pat_placeholders(raw_pat_key, |idx, buf| {
            if self.used_pat_bindings.contains(&idx) {
                buf.push_str(&format!("$p{idx}"));
            } else {
                buf.push('_');
            }
        });
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
        let pushed_scope = if ImportScope::has_block_scope_items(&block.stmts) {
            let mut child = self.scope().clone();
            child.ingest_block_stmts(&block.stmts);
            self.scope_stack.push(child);
            true
        } else {
            false
        };
        let saved_locals = self.local_bindings.clone();
        let out = if block.stmts.len() == 1 {
            match &block.stmts[0] {
                Stmt::Expr(e, _) => self.norm_arm_body(e),
                Stmt::Macro(sm) => self.norm_macro(&sm.mac),
                _ => self.norm_stmt_list(&block.stmts),
            }
        } else {
            self.norm_stmt_list(&block.stmts)
        };
        self.local_bindings = saved_locals;
        if pushed_scope {
            self.scope_stack.pop();
        }
        out
    }

    fn norm_stmt_list(&mut self, stmts: &[Stmt]) -> String {
        let mut parts = Vec::with_capacity(stmts.len());
        for stmt in stmts {
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
                    let pat_str = self.norm_binding_pat(&local.pat);
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

    /// Resolves local variables first (so `let` shadowing a pattern binding resolves to `$lN`),
    /// then pattern bindings (`$pN`), then per-match outer variables (`$rN`).
    pub(super) fn resolve_var(&mut self, name: &str) -> String {
        if let Some(&idx) = self.local_bindings.get(name) {
            return format!("$l{idx}");
        }
        if let Some(&idx) = self.pat_bindings.get(name) {
            self.used_pat_bindings.insert(idx);
            return format!("$p{idx}");
        }
        self.outer.resolve_outer(name)
    }

    fn record_enum_path_ref(&mut self, path: &syn::Path) {
        if let Some((qual, _)) = split_enum_variant_path(path, self.scope(), self.self_ty) {
            if let Some(q) = qual {
                self.referenced_enum_qualifiers.insert(q);
            } else {
                self.has_unqualified_enum_ref = true;
            }
        }
    }

    pub(super) fn norm_expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Lit(l) => lit_to_key_and_atom(&l.lit, false)
                .map(|(k, _)| k)
                .unwrap_or_else(|| self.outer.fresh_opaque("lit")),
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
                    let (key, variant_qual) = self.scope().canonical_single_ident_with_qual(&name);
                    if let Some(qual) = variant_qual {
                        if let Some(q) = qual {
                            self.referenced_enum_qualifiers.insert(q);
                        } else {
                            self.has_unqualified_enum_ref = true;
                        }
                    }
                    return key;
                }
                self.record_enum_path_ref(&p.path);
                canonical_path(&p.path, self.scope(), self.self_ty)
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
                        self.record_enum_path_ref(&p.path);
                        canonical_path(&p.path, self.scope(), self.self_ty)
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
                self.record_enum_path_ref(&s.path);
                let ty = canonical_path(&s.path, self.scope(), self.self_ty);
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
                let saved = self.local_bindings.clone();
                let cond = self.norm_expr(&i.cond);
                let then_b = self.norm_block_expr(&i.then_branch);
                self.local_bindings = saved;
                let else_b = i
                    .else_branch
                    .as_ref()
                    .map(|(_, e)| format!(" else {}", self.norm_expr(e)))
                    .unwrap_or_default();
                format!("if {cond}{{{then_b}}}{else_b}")
            }
            Expr::Loop(l) => format!("loop{{{}}}", self.norm_block_expr(&l.body)),
            Expr::While(w) => {
                let saved = self.local_bindings.clone();
                let cond = self.norm_expr(&w.cond);
                let body = self.norm_block_expr(&w.body);
                self.local_bindings = saved;
                format!("while {cond}{{{body}}}")
            }
            Expr::ForLoop(fl) => {
                let iter = self.norm_expr(&fl.expr);
                let saved = self.local_bindings.clone();
                let pat = self.norm_binding_pat(&fl.pat);
                let body = self.norm_block_expr(&fl.body);
                self.local_bindings = saved;
                format!("for {pat} in {iter}{{{body}}}")
            }
            Expr::Async(a) => format!("async{{{}}}", self.norm_block_expr(&a.block)),
            Expr::Const(c) => format!("const{{{}}}", self.norm_block_expr(&c.block)),
            Expr::TryBlock(tb) => format!("try{{{}}}", self.norm_block_expr(&tb.block)),
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
                let saved = self.local_bindings.clone();
                let ps: Vec<String> = cl
                    .inputs
                    .iter()
                    .map(|p| self.norm_binding_pat(p))
                    .collect();
                let body = self.norm_expr(&cl.body);
                self.local_bindings = saved;
                format!("|{}|{body}", ps.join(","))
            }
            Expr::Match(m) => {
                let scr = self.norm_expr(&m.expr);
                let mut arms: Vec<String> = m
                    .arms
                    .iter()
                    .map(|a| {
                        let saved = self.local_bindings.clone();
                        let p = self.norm_binding_pat(&a.pat);
                        let g = a
                            .guard
                            .as_ref()
                            .map(|(_, g)| format!(" if {}", self.norm_expr(g)))
                            .unwrap_or_default();
                        let b = self.norm_expr(&a.body);
                        self.local_bindings = saved;
                        format!("{p}{g}={b}")
                    })
                    .collect();
                arms.sort();
                format!("match {scr}{{{}}}", arms.join(","))
            }
            Expr::Unsafe(u) => format!("unsafe{{{}}}", self.norm_block_expr(&u.block)),
            Expr::Await(a) => format!("{}.await", self.norm_expr(&a.base)),
            Expr::Let(l) => {
                let val = self.norm_expr(&l.expr);
                let pat = self.norm_binding_pat(&l.pat);
                format!("let {pat}={val}")
            }
            Expr::Infer(_) => "_".to_string(),
            Expr::RawAddr(ra) => {
                let kind = match ra.mutability {
                    syn::PointerMutability::Mut(_) => "mut",
                    syn::PointerMutability::Const(_) => "const",
                };
                format!("&raw {kind} {}", self.norm_expr(&ra.expr))
            }
            _ => self.outer.fresh_opaque("expr"),
        }
    }
}
