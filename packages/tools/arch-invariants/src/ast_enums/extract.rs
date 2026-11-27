//! AST visitor extracting `MatchSite` fingerprints from `Expr::Match`, `matches!`, `if let` chains,
//! and nested expressions inside macro invocations.

use std::collections::BTreeSet;

use syn::visit::{self, Visit};
use syn::{Expr, Item, Pat, Stmt};

use crate::ast_layout::last_type_ident;
use crate::is_cfg_test_attr;

use super::extract_arms::{
    build_if_let_chain_site, build_match_site, build_matches_macro_site, collect_if_let_chain,
    SiteBuildCtx,
};
use super::patterns::MirrorAtom;
use super::scope::{EnumQualifier, ImportScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MatchDirection {
    EnumToBody,
    LitToEnum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedArm {
    pub arm_index: usize,
    pub is_enum_pat: bool,
    pub pat_qualifier: Option<EnumQualifier>,
    pub tuple_qualifiers: Option<Vec<Option<EnumQualifier>>>,
    pub variant_head: String,
    pub pat_key: String,
    pub body_key: String,
    pub is_noop: bool,
    pub is_bare_outer: bool,
    pub mirror_pair: Option<(MirrorAtom, MirrorAtom)>,
    pub body_enum_qualifiers: BTreeSet<EnumQualifier>,
    pub has_unqualified_body_enum: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MatchSite {
    pub file: String,
    pub symbol: String,
    pub line: usize,
    pub snippet: String,
    pub(super) direction: Option<MatchDirection>,
    pub enum_name: String,
    pub(super) enum_qualifier: Option<EnumQualifier>,
    pub(super) tuple_qualifiers: Option<Vec<Option<EnumQualifier>>>,
    pub(super) glob_enums: BTreeSet<EnumQualifier>,
    pub(super) has_unscoped_glob: bool,
    pub(super) arms: Vec<NormalizedArm>,
    pub(super) wildcard_body: Option<String>,
    pub(super) mirror_pairs: Vec<(MirrorAtom, MirrorAtom)>,
}

pub(super) fn extract_match_sites_with_crate(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    current_crate: Option<&str>,
    opaque_seed: &mut usize,
    out: &mut Vec<MatchSite>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    if !raw.contains("match") && !raw.contains("if") {
        return;
    }
    let mut scope = ImportScope::with_crate(current_crate);
    scope.ingest_items(&ast.items);
    let mut state = FileExtractState {
        rel,
        raw,
        mod_path: Vec::new(),
        opaque_seed,
        out,
    };
    state.extract_from_items(&ast.items, &scope, None);
}

struct FileExtractState<'a> {
    rel: &'a str,
    raw: &'a str,
    mod_path: Vec<String>,
    opaque_seed: &'a mut usize,
    out: &'a mut Vec<MatchSite>,
}

impl FileExtractState<'_> {
    fn extract_from_items(&mut self, items: &[Item], scope: &ImportScope, self_ty: Option<&str>) {
        for item in items {
            match item {
                Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                    if let Some((_, sub)) = &m.content {
                        self.mod_path.push(m.ident.to_string());
                        let child = ImportScope::child_for_inline_mod(scope, sub);
                        self.extract_from_items(sub, &child, None);
                        self.mod_path.pop();
                    }
                }
                Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                    let sym = qualify_symbol(&self.mod_path, &f.sig.ident.to_string());
                    let params = collect_sig_params(&f.sig);
                    self.run_block_extractor(sym, scope, self_ty, params, &f.block);
                }
                Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                    let impl_ty = last_type_ident(&imp.self_ty);
                    let impl_ty_ref = impl_ty.as_deref();
                    let impl_prefix = format_impl_prefix(impl_ty_ref, imp.trait_.as_ref());
                    for ii in &imp.items {
                        match ii {
                            syn::ImplItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                                let base = match &impl_prefix {
                                    Some(p) => format!("{p}::{}", f.sig.ident),
                                    None => f.sig.ident.to_string(),
                                };
                                let sym = qualify_symbol(&self.mod_path, &base);
                                let params = collect_sig_params(&f.sig);
                                self.run_block_extractor(sym, scope, impl_ty_ref, params, &f.block);
                            }
                            syn::ImplItem::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                                let base = match &impl_prefix {
                                    Some(p) => format!("{p}::{}", c.ident),
                                    None => c.ident.to_string(),
                                };
                                let sym = qualify_symbol(&self.mod_path, &base);
                                self.run_expr_extractor(sym, scope, impl_ty_ref, &c.expr);
                            }
                            _ => {}
                        }
                    }
                }
                Item::Trait(tr) if !is_cfg_test_attr(&tr.attrs) => {
                    let tr_name = tr.ident.to_string();
                    for ti in &tr.items {
                        match ti {
                            syn::TraitItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                                if let Some(block) = &f.default {
                                    let sym = qualify_symbol(
                                        &self.mod_path,
                                        &format!("{tr_name}::{}", f.sig.ident),
                                    );
                                    let params = collect_sig_params(&f.sig);
                                    self.run_block_extractor(sym, scope, None, params, block);
                                }
                            }
                            syn::TraitItem::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                                if let Some((_, expr)) = &c.default {
                                    let sym = qualify_symbol(
                                        &self.mod_path,
                                        &format!("{tr_name}::{}", c.ident),
                                    );
                                    self.run_expr_extractor(sym, scope, None, expr);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Item::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                    let sym = qualify_symbol(&self.mod_path, &format!("const {}", c.ident));
                    self.run_expr_extractor(sym, scope, self_ty, &c.expr);
                }
                Item::Static(s) if !is_cfg_test_attr(&s.attrs) => {
                    let sym = qualify_symbol(&self.mod_path, &format!("static {}", s.ident));
                    self.run_expr_extractor(sym, scope, self_ty, &s.expr);
                }
                _ => {}
            }
        }
    }

    fn run_block_extractor(
        &mut self,
        symbol: String,
        scope: &ImportScope,
        self_ty: Option<&str>,
        params: Vec<String>,
        block: &syn::Block,
    ) {
        let mut v = MatchExtractor {
            rel: self.rel,
            raw: self.raw,
            symbol,
            base_scope: scope,
            scope_stack: Vec::new(),
            self_ty_stack: vec![self_ty.map(ToString::to_string)],
            enclosing_locals: params,
            opaque_seed: *self.opaque_seed,
            out: self.out,
        };
        v.visit_block(block);
        *self.opaque_seed = v.opaque_seed;
    }

    fn run_expr_extractor(
        &mut self,
        symbol: String,
        scope: &ImportScope,
        self_ty: Option<&str>,
        expr: &Expr,
    ) {
        let mut v = MatchExtractor {
            rel: self.rel,
            raw: self.raw,
            symbol,
            base_scope: scope,
            scope_stack: Vec::new(),
            self_ty_stack: vec![self_ty.map(ToString::to_string)],
            enclosing_locals: Vec::new(),
            opaque_seed: *self.opaque_seed,
            out: self.out,
        };
        v.visit_expr(expr);
        *self.opaque_seed = v.opaque_seed;
    }
}

fn qualify_symbol(mod_path: &[String], base: &str) -> String {
    if mod_path.is_empty() {
        base.to_string()
    } else {
        format!("{}::{base}", mod_path.join("::"))
    }
}

fn format_impl_prefix(
    impl_ty: Option<&str>,
    trait_: Option<&(Option<syn::Token![!]>, syn::Path, syn::Token![for])>,
) -> Option<String> {
    let ty = impl_ty?;
    if let Some((_, tr_path, _)) = trait_
        && let Some(seg) = tr_path.segments.last()
    {
        return Some(format!("<{ty} as {}>", seg.ident));
    }
    Some(ty.to_string())
}

fn collect_sig_params(sig: &syn::Signature) -> Vec<String> {
    let mut out = Vec::new();
    for input in &sig.inputs {
        match input {
            syn::FnArg::Receiver(_) => out.push("self".to_string()),
            syn::FnArg::Typed(pt) => collect_pat_idents(&pt.pat, &mut out),
        }
    }
    out
}

pub(super) fn collect_pat_idents(pat: &Pat, out: &mut Vec<String>) {
    match pat {
        Pat::Ident(pi) => {
            let name = pi.ident.to_string();
            if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
                out.push(name);
            }
            if let Some((_, sub)) = &pi.subpat {
                collect_pat_idents(sub, out);
            }
        }
        Pat::Tuple(t) => t.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::TupleStruct(ts) => ts.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::Struct(ps) => ps.fields.iter().for_each(|fp| collect_pat_idents(&fp.pat, out)),
        Pat::Slice(s) => s.elems.iter().for_each(|p| collect_pat_idents(p, out)),
        Pat::Or(o) => {
            if let Some(first) = o.cases.first() {
                collect_pat_idents(first, out);
            }
        }
        Pat::Paren(p) => collect_pat_idents(&p.pat, out),
        Pat::Reference(r) => collect_pat_idents(&r.pat, out),
        Pat::Type(t) => collect_pat_idents(&t.pat, out),
        _ => {}
    }
}

struct MatchExtractor<'a> {
    rel: &'a str,
    raw: &'a str,
    symbol: String,
    base_scope: &'a ImportScope,
    scope_stack: Vec<ImportScope>,
    self_ty_stack: Vec<Option<String>>,
    enclosing_locals: Vec<String>,
    opaque_seed: usize,
    out: &'a mut Vec<MatchSite>,
}

impl MatchExtractor<'_> {
    fn build_ctx(&self) -> SiteBuildCtx<'_> {
        SiteBuildCtx {
            rel: self.rel,
            raw: self.raw,
            symbol: &self.symbol,
            scope: self.scope_stack.last().unwrap_or(self.base_scope),
            self_ty: self.self_ty_stack.last().and_then(|o| o.as_deref()),
            enclosing_locals: &self.enclosing_locals,
        }
    }

    fn with_nested_scope(
        &mut self,
        next_sym: String,
        locals: Vec<String>,
        f: impl FnOnce(&mut Self),
    ) {
        let prev_sym = std::mem::replace(&mut self.symbol, next_sym);
        let prev_locals = std::mem::replace(&mut self.enclosing_locals, locals);
        f(self);
        self.enclosing_locals = prev_locals;
        self.symbol = prev_sym;
    }

    fn visit_macro_body_exprs(&mut self, mac: &syn::Macro) {
        if let Ok(args) = mac.parse_body_with(
            syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
        ) {
            for arg in &args {
                self.visit_expr(arg);
            }
            return;
        }
        struct RepeatArgs(Expr, Expr);
        impl syn::parse::Parse for RepeatArgs {
            fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
                let e: Expr = input.parse()?;
                let _semi: syn::Token![;] = input.parse()?;
                let len: Expr = input.parse()?;
                Ok(Self(e, len))
            }
        }
        if let Ok(RepeatArgs(e, len)) = mac.parse_body::<RepeatArgs>() {
            self.visit_expr(&e);
            self.visit_expr(&len);
        }
    }
}

impl<'ast> Visit<'ast> for MatchExtractor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if item_attrs(item).is_some_and(is_cfg_test_attr) {
            return;
        }
        match item {
            Item::Mod(m) => {
                if let Some((_, sub)) = &m.content {
                    let next_sym = format!("{}::{}", self.symbol, m.ident);
                    self.with_nested_scope(next_sym, Vec::new(), |this| {
                        let child = ImportScope::child_for_inline_mod(
                            this.scope_stack.last().unwrap_or(this.base_scope),
                            sub,
                        );
                        this.scope_stack.push(child);
                        for it in sub {
                            this.visit_item(it);
                        }
                        this.scope_stack.pop();
                    });
                }
            }
            Item::Fn(f) => {
                // Nested `fn` cannot capture outer locals in Rust.
                let next_sym = format!("{}::{}", self.symbol, f.sig.ident);
                self.with_nested_scope(next_sym, collect_sig_params(&f.sig), |this| {
                    this.visit_block(&f.block);
                });
            }
            Item::Impl(imp) => {
                let impl_ty = last_type_ident(&imp.self_ty);
                let impl_prefix = format_impl_prefix(impl_ty.as_deref(), imp.trait_.as_ref());
                self.self_ty_stack.push(impl_ty);
                for ii in &imp.items {
                    match ii {
                        syn::ImplItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                            let suffix = match &impl_prefix {
                                Some(p) => format!("{p}::{}", f.sig.ident),
                                None => f.sig.ident.to_string(),
                            };
                            let next_sym = format!("{}::{suffix}", self.symbol);
                            self.with_nested_scope(next_sym, collect_sig_params(&f.sig), |this| {
                                this.visit_block(&f.block);
                            });
                        }
                        syn::ImplItem::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                            let suffix = match &impl_prefix {
                                Some(p) => format!("{p}::{}", c.ident),
                                None => c.ident.to_string(),
                            };
                            let next_sym = format!("{}::{suffix}", self.symbol);
                            self.with_nested_scope(next_sym, Vec::new(), |this| {
                                this.visit_expr(&c.expr);
                            });
                        }
                        _ => {}
                    }
                }
                self.self_ty_stack.pop();
            }
            Item::Trait(tr) => {
                let tr_name = tr.ident.to_string();
                self.self_ty_stack.push(None);
                for ti in &tr.items {
                    match ti {
                        syn::TraitItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                            if let Some(block) = &f.default {
                                let next_sym =
                                    format!("{}::{tr_name}::{}", self.symbol, f.sig.ident);
                                self.with_nested_scope(
                                    next_sym,
                                    collect_sig_params(&f.sig),
                                    |this| this.visit_block(block),
                                );
                            }
                        }
                        syn::TraitItem::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                            if let Some((_, expr)) = &c.default {
                                let next_sym = format!("{}::{tr_name}::{}", self.symbol, c.ident);
                                self.with_nested_scope(next_sym, Vec::new(), |this| {
                                    this.visit_expr(expr);
                                });
                            }
                        }
                        _ => {}
                    }
                }
                self.self_ty_stack.pop();
            }
            Item::Const(c) => {
                let next_sym = format!("{}::const {}", self.symbol, c.ident);
                self.with_nested_scope(next_sym, Vec::new(), |this| this.visit_expr(&c.expr));
            }
            Item::Static(s) => {
                let next_sym = format!("{}::static {}", self.symbol, s.ident);
                self.with_nested_scope(next_sym, Vec::new(), |this| this.visit_expr(&s.expr));
            }
            _ => visit::visit_item(self, item),
        }
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let pushed_scope = if ImportScope::has_block_scope_items(&block.stmts) {
            let mut child = self.scope_stack.last().unwrap_or(self.base_scope).clone();
            child.ingest_block_stmts(&block.stmts);
            self.scope_stack.push(child);
            true
        } else {
            false
        };
        let saved_locals = self.enclosing_locals.len();
        for stmt in &block.stmts {
            match stmt {
                Stmt::Local(local) => {
                    if is_cfg_test_attr(&local.attrs) {
                        continue;
                    }
                    visit::visit_local(self, local);
                    collect_pat_idents(&local.pat, &mut self.enclosing_locals);
                }
                Stmt::Expr(e, _) => self.visit_expr(e),
                Stmt::Macro(sm) => {
                    if !is_cfg_test_attr(&sm.attrs) {
                        self.visit_macro_body_exprs(&sm.mac);
                    }
                }
                Stmt::Item(it) => self.visit_item(it),
            }
        }
        self.enclosing_locals.truncate(saved_locals);
        if pushed_scope {
            self.scope_stack.pop();
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if expr_attrs(expr).is_some_and(is_cfg_test_attr) {
            return;
        }
        visit::visit_expr(self, expr);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if is_cfg_test_attr(&arm.attrs) {
            return;
        }
        let saved = self.enclosing_locals.len();
        collect_pat_idents(&arm.pat, &mut self.enclosing_locals);
        if let Some((_, g)) = &arm.guard {
            self.visit_expr(g);
        }
        self.visit_expr(&arm.body);
        self.enclosing_locals.truncate(saved);
    }

    fn visit_expr_closure(&mut self, cl: &'ast syn::ExprClosure) {
        let saved = self.enclosing_locals.len();
        for input in &cl.inputs {
            collect_pat_idents(input, &mut self.enclosing_locals);
        }
        self.visit_expr(&cl.body);
        self.enclosing_locals.truncate(saved);
    }

    fn visit_expr_for_loop(&mut self, fl: &'ast syn::ExprForLoop) {
        self.visit_expr(&fl.expr);
        let saved = self.enclosing_locals.len();
        collect_pat_idents(&fl.pat, &mut self.enclosing_locals);
        self.visit_block(&fl.body);
        self.enclosing_locals.truncate(saved);
    }

    fn visit_expr_while(&mut self, w: &'ast syn::ExprWhile) {
        let saved = self.enclosing_locals.len();
        self.visit_expr(&w.cond);
        self.visit_block(&w.body);
        self.enclosing_locals.truncate(saved);
    }

    fn visit_expr_let(&mut self, l: &'ast syn::ExprLet) {
        self.visit_expr(&l.expr);
        collect_pat_idents(&l.pat, &mut self.enclosing_locals);
    }

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        let mut seed = self.opaque_seed;
        let site = build_match_site(&self.build_ctx(), &mut seed, m);
        self.opaque_seed = seed;
        if let Some(site) = site {
            self.out.push(site);
        }
        visit::visit_expr_match(self, m);
    }

    fn visit_expr_macro(&mut self, em: &'ast syn::ExprMacro) {
        if em
            .mac
            .path
            .segments
            .last()
            .is_some_and(|s| s.ident == "matches")
        {
            let mut seed = self.opaque_seed;
            let site = build_matches_macro_site(&self.build_ctx(), &mut seed, em);
            self.opaque_seed = seed;
            if let Some(site) = site {
                self.out.push(site);
            }
        } else {
            self.visit_macro_body_exprs(&em.mac);
        }
        visit::visit_expr_macro(self, em);
    }

    fn visit_expr_if(&mut self, expr_if: &'ast syn::ExprIf) {
        if let Some((branches, else_body)) = collect_if_let_chain(expr_if)
            && branches.len() >= 2
        {
            let mut seed = self.opaque_seed;
            let site = build_if_let_chain_site(
                &self.build_ctx(),
                &mut seed,
                expr_if.if_token.span.start().line,
                &branches,
                else_body,
            );
            self.opaque_seed = seed;
            if let Some(site) = site {
                self.out.push(site);
                for (pat, scr, body_block) in &branches {
                    self.visit_expr(scr);
                    let saved = self.enclosing_locals.len();
                    collect_pat_idents(pat, &mut self.enclosing_locals);
                    self.visit_block(body_block);
                    self.enclosing_locals.truncate(saved);
                }
                if let Some(eb) = else_body {
                    self.visit_expr(eb);
                }
                return;
            }
        }
        let saved = self.enclosing_locals.len();
        self.visit_expr(&expr_if.cond);
        self.visit_block(&expr_if.then_branch);
        self.enclosing_locals.truncate(saved);
        if let Some((_, eb)) = &expr_if.else_branch {
            self.visit_expr(eb);
        }
    }
}

fn item_attrs(item: &Item) -> Option<&[syn::Attribute]> {
    match item {
        Item::Mod(x) => Some(&x.attrs),
        Item::Fn(x) => Some(&x.attrs),
        Item::Impl(x) => Some(&x.attrs),
        Item::Trait(x) => Some(&x.attrs),
        Item::Const(x) => Some(&x.attrs),
        Item::Static(x) => Some(&x.attrs),
        Item::Struct(x) => Some(&x.attrs),
        Item::Enum(x) => Some(&x.attrs),
        Item::Type(x) => Some(&x.attrs),
        Item::Use(x) => Some(&x.attrs),
        _ => None,
    }
}

fn expr_attrs(expr: &Expr) -> Option<&[syn::Attribute]> {
    match expr {
        Expr::Block(x) => Some(&x.attrs),
        Expr::Match(x) => Some(&x.attrs),
        Expr::If(x) => Some(&x.attrs),
        Expr::Macro(x) => Some(&x.attrs),
        Expr::Loop(x) => Some(&x.attrs),
        Expr::While(x) => Some(&x.attrs),
        Expr::ForLoop(x) => Some(&x.attrs),
        Expr::Closure(x) => Some(&x.attrs),
        Expr::Async(x) => Some(&x.attrs),
        Expr::Unsafe(x) => Some(&x.attrs),
        Expr::Const(x) => Some(&x.attrs),
        Expr::Call(x) => Some(&x.attrs),
        Expr::MethodCall(x) => Some(&x.attrs),
        _ => None,
    }
}
