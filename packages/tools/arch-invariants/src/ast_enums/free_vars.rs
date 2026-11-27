//! Lexical free-variable scanner for per-match outer-variable (`$rN`) pre-seeding.
//!
//! Collects ONLY genuine free single-segment value `Expr::Path` variables and inline format-string
//! captures, ignoring struct field names (`dst.align`), method names (`v.len()`), qualified paths,
//! Rust keywords/primitives, and arm-local or pattern bindings.

use std::collections::BTreeSet;

use syn::visit::Visit;
use syn::{Expr, Stmt};

use super::extract::collect_pat_idents;
use super::normalize_helpers::{is_rust_keyword_or_primitive, scan_format_captures};

pub(super) fn collect_free_vars(expr: &Expr, initial_bound: &[String], out: &mut BTreeSet<String>) {
    struct FreeVarScanner<'a> {
        bound: Vec<String>,
        out: &'a mut BTreeSet<String>,
    }

    impl FreeVarScanner<'_> {
        fn record_if_free(&mut self, name: &str) {
            if (name == "self" || !name.starts_with(|c: char| c.is_ascii_uppercase()))
                && !is_rust_keyword_or_primitive(name)
                && !self.bound.iter().any(|b| b == name)
            {
                self.out.insert(name.to_string());
            }
        }

        fn scan_format_lit(&mut self, s: &str) {
            let mut captures = Vec::new();
            scan_format_captures(s, None, |ident, _| {
                captures.push(ident.to_string());
                String::new()
            });
            for cap in captures {
                self.record_if_free(&cap);
            }
        }

        fn scan_tokens(&mut self, ts: proc_macro2::TokenStream) {
            let mut prev_member_sep = false;
            for tt in ts {
                match tt {
                    proc_macro2::TokenTree::Ident(id) => {
                        if !prev_member_sep {
                            self.record_if_free(&id.to_string());
                        }
                        prev_member_sep = false;
                    }
                    proc_macro2::TokenTree::Group(g) => {
                        self.scan_tokens(g.stream());
                        prev_member_sep = false;
                    }
                    proc_macro2::TokenTree::Literal(lit) => {
                        if let Ok(syn::Lit::Str(ls)) = syn::parse_str::<syn::Lit>(&lit.to_string())
                        {
                            self.scan_format_lit(&ls.value());
                        }
                        prev_member_sep = false;
                    }
                    proc_macro2::TokenTree::Punct(p) => {
                        let ch = p.as_char();
                        prev_member_sep = ch == '.' || ch == ':';
                    }
                }
            }
        }
    }

    impl<'ast> Visit<'ast> for FreeVarScanner<'_> {
        fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
            if p.qself.is_none() && p.path.segments.len() == 1 {
                self.record_if_free(&p.path.segments[0].ident.to_string());
            }
        }
        fn visit_expr_field(&mut self, f: &'ast syn::ExprField) {
            self.visit_expr(&f.base);
        }
        fn visit_expr_method_call(&mut self, mc: &'ast syn::ExprMethodCall) {
            self.visit_expr(&mc.receiver);
            for arg in &mc.args {
                self.visit_expr(arg);
            }
        }
        fn visit_expr_struct(&mut self, s: &'ast syn::ExprStruct) {
            for f in &s.fields {
                self.visit_expr(&f.expr);
            }
            if let Some(rest) = &s.rest {
                self.visit_expr(rest);
            }
        }
        fn visit_block(&mut self, block: &'ast syn::Block) {
            let saved = self.bound.len();
            for stmt in &block.stmts {
                match stmt {
                    Stmt::Local(l) => {
                        if let Some(init) = &l.init {
                            self.visit_expr(&init.expr);
                            if let Some((_, div)) = &init.diverge {
                                self.visit_expr(div);
                            }
                        }
                        collect_pat_idents(&l.pat, &mut self.bound);
                    }
                    Stmt::Expr(e, _) => self.visit_expr(e),
                    Stmt::Macro(sm) => self.visit_macro(&sm.mac),
                    Stmt::Item(_) => {}
                }
            }
            self.bound.truncate(saved);
        }
        fn visit_expr_closure(&mut self, cl: &'ast syn::ExprClosure) {
            let saved = self.bound.len();
            for p in &cl.inputs {
                collect_pat_idents(p, &mut self.bound);
            }
            self.visit_expr(&cl.body);
            self.bound.truncate(saved);
        }
        fn visit_expr_for_loop(&mut self, fl: &'ast syn::ExprForLoop) {
            self.visit_expr(&fl.expr);
            let saved = self.bound.len();
            collect_pat_idents(&fl.pat, &mut self.bound);
            self.visit_block(&fl.body);
            self.bound.truncate(saved);
        }
        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            let saved = self.bound.len();
            collect_pat_idents(&arm.pat, &mut self.bound);
            if let Some((_, g)) = &arm.guard {
                self.visit_expr(g);
            }
            self.visit_expr(&arm.body);
            self.bound.truncate(saved);
        }
        fn visit_expr_if(&mut self, expr_if: &'ast syn::ExprIf) {
            let saved = self.bound.len();
            self.visit_expr(&expr_if.cond);
            self.visit_block(&expr_if.then_branch);
            self.bound.truncate(saved);
            if let Some((_, eb)) = &expr_if.else_branch {
                self.visit_expr(eb);
            }
        }
        fn visit_expr_while(&mut self, w: &'ast syn::ExprWhile) {
            let saved = self.bound.len();
            self.visit_expr(&w.cond);
            self.visit_block(&w.body);
            self.bound.truncate(saved);
        }
        fn visit_expr_let(&mut self, l: &'ast syn::ExprLet) {
            self.visit_expr(&l.expr);
            collect_pat_idents(&l.pat, &mut self.bound);
        }
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if let Ok(args) = mac.parse_body_with(
                syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated,
            ) {
                for arg in &args {
                    if let Expr::Lit(l) = arg
                        && let syn::Lit::Str(ls) = &l.lit
                    {
                        self.scan_format_lit(&ls.value());
                    } else {
                        self.visit_expr(arg);
                    }
                }
                return;
            }
            self.scan_tokens(mac.tokens.clone());
        }
        fn visit_item(&mut self, _: &'ast syn::Item) {}
    }

    let mut scanner = FreeVarScanner {
        bound: initial_bound.to_vec(),
        out,
    };
    scanner.visit_expr(expr);
}
