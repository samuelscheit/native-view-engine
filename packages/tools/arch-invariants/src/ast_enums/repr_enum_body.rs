//! Body analysis helpers for `[INV:no-manual-repr-enum-converters]` (`repr_enum_guard`).
//!
//! Recognizes `match` tables and `if`/`else if` discriminant chains mapping integer literals onto
//! variants of a `#[repr(<int>)]` unit-only enum.

use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{BinOp, Expr, Lit, Pat, UnOp};

use super::repr_enum_guard::{ReprEnumInfo, REPR_INT_TYPES};
use super::scope::ImportScope;

pub(super) fn extract_repr_unit_enum(en: &syn::ItemEnum) -> Option<ReprEnumInfo> {
    let repr = find_int_repr(&en.attrs)?;
    if en.variants.is_empty() || !en.variants.iter().all(|v| matches!(v.fields, syn::Fields::Unit)) {
        return None;
    }
    let mut variants = BTreeSet::new();
    let mut discs = BTreeSet::new();
    let mut all_discs_known = true;
    let mut next_disc: i128 = 0;

    for v in &en.variants {
        variants.insert(v.ident.to_string());
        if let Some((_, expr)) = &v.discriminant {
            if let Some(val) = eval_int_literal_expr(expr) {
                discs.insert(val);
                next_disc = val.saturating_add(1);
            } else {
                all_discs_known = false;
            }
        } else if all_discs_known {
            discs.insert(next_disc);
            next_disc = next_disc.saturating_add(1);
        }
    }

    Some(ReprEnumInfo {
        repr,
        variants,
        discriminants: all_discs_known.then_some(discs),
    })
}

fn find_int_repr(attrs: &[syn::Attribute]) -> Option<String> {
    let mut found = None;
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(id) = meta.path.get_ident() {
                let s = id.to_string();
                if REPR_INT_TYPES.contains(&s.as_str()) {
                    found = Some(s);
                }
            }
            Ok(())
        });
    }
    found
}

pub(super) fn body_matches_repr_discriminants(
    block: &syn::Block,
    int_param: Option<&str>,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    info: &ReprEnumInfo,
    arg_int: &str,
) -> bool {
    struct Scan<'a> {
        int_param: Option<&'a str>,
        target_enum: &'a str,
        self_ty: Option<&'a str>,
        scope: &'a ImportScope,
        info: &'a ReprEnumInfo,
        arg_int: &'a str,
        found: bool,
    }

    impl Scan<'_> {
        fn check_int_set(&self, matched: &[i128]) -> bool {
            if matched.len() < 2 {
                return false;
            }
            if let Some(discs) = &self.info.discriminants {
                let hits = matched.iter().filter(|v| discs.contains(v)).count();
                if self.arg_int == self.info.repr {
                    hits >= 2
                } else {
                    hits >= 2 && hits == matched.len()
                }
            } else {
                self.arg_int == self.info.repr
            }
        }
    }

    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_block(&mut self, block: &'ast syn::Block) {
            let mut early_returns: BTreeMap<String, Vec<i128>> = BTreeMap::new();
            for stmt in &block.stmts {
                if let syn::Stmt::Expr(Expr::If(expr_if), _) = stmt {
                    let mut cur = expr_if;
                    loop {
                        if let Some((scrutinee, val)) =
                            extract_eq_int_cond(&cur.cond, self.int_param)
                            && block_early_returns_enum_variant(
                                &cur.then_branch,
                                self.target_enum,
                                self.self_ty,
                                self.scope,
                                &self.info.variants,
                            )
                        {
                            early_returns.entry(scrutinee).or_default().push(val);
                        }
                        if let Some((_, else_expr)) = &cur.else_branch
                            && let Expr::If(next_if) = &**else_expr
                        {
                            cur = next_if;
                            continue;
                        }
                        break;
                    }
                }
            }
            if early_returns.values().any(|vals| self.check_int_set(vals)) {
                self.found = true;
                return;
            }
            visit::visit_block(self, block);
        }

        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            let mut matched_vals = Vec::new();
            for arm in &m.arms {
                if expr_constructs_enum_variant(
                    &arm.body,
                    self.target_enum,
                    self.self_ty,
                    self.scope,
                    &self.info.variants,
                ) {
                    collect_pat_int_literals(&arm.pat, &mut matched_vals);
                }
            }
            if self.check_int_set(&matched_vals) {
                self.found = true;
            }
            if !self.found {
                visit::visit_expr_match(self, m);
            }
        }

        fn visit_expr_if(&mut self, expr_if: &'ast syn::ExprIf) {
            let mut by_scrutinee: BTreeMap<String, Vec<i128>> = BTreeMap::new();
            let mut cur = expr_if;
            loop {
                if let Some((scrutinee, val)) = extract_eq_int_cond(&cur.cond, self.int_param)
                    && block_constructs_enum_variant(
                        &cur.then_branch,
                        self.target_enum,
                        self.self_ty,
                        self.scope,
                        &self.info.variants,
                    )
                {
                    by_scrutinee.entry(scrutinee).or_default().push(val);
                }
                if let Some((_, else_expr)) = &cur.else_branch
                    && let Expr::If(next_if) = &**else_expr
                {
                    cur = next_if;
                    continue;
                }
                break;
            }
            if by_scrutinee.values().any(|vals| self.check_int_set(vals)) {
                self.found = true;
            }
            if !self.found {
                visit::visit_expr_if(self, expr_if);
            }
        }
    }

    let mut scan = Scan {
        int_param,
        target_enum,
        self_ty,
        scope,
        info,
        arg_int,
        found: false,
    };
    scan.visit_block(block);
    scan.found
}

fn eval_int_literal_expr(expr: &Expr) -> Option<i128> {
    match expr {
        Expr::Paren(p) => eval_int_literal_expr(&p.expr),
        Expr::Group(g) => eval_int_literal_expr(&g.expr),
        Expr::Lit(l) => match &l.lit {
            Lit::Int(li) => li.base10_parse::<i128>().ok(),
            _ => None,
        },
        Expr::Unary(u) if matches!(u.op, UnOp::Neg(_)) => {
            eval_int_literal_expr(&u.expr).and_then(i128::checked_neg)
        }
        _ => None,
    }
}

fn extract_scrutinee_ident(mut expr: &Expr) -> Option<String> {
    loop {
        match expr {
            Expr::Paren(p) => expr = &p.expr,
            Expr::Group(g) => expr = &g.expr,
            Expr::Path(ep) => return ep.path.get_ident().map(ToString::to_string),
            _ => return None,
        }
    }
}

fn extract_eq_int_cond(cond: &Expr, int_param: Option<&str>) -> Option<(String, i128)> {
    let Expr::Binary(bin) = cond else {
        return None;
    };
    if !matches!(bin.op, BinOp::Eq(_)) {
        return None;
    }
    let check = |e: &Expr| -> Option<String> {
        let id = extract_scrutinee_ident(e)?;
        if int_param.is_some_and(|p| p != id) {
            return None;
        }
        Some(id)
    };
    if let Some(id) = check(&bin.left)
        && let Some(v) = eval_int_literal_expr(&bin.right)
    {
        return Some((id, v));
    }
    if let Some(id) = check(&bin.right)
        && let Some(v) = eval_int_literal_expr(&bin.left)
    {
        return Some((id, v));
    }
    None
}

fn collect_pat_int_literals(pat: &Pat, out: &mut Vec<i128>) {
    match pat {
        Pat::Lit(l) => {
            if let Lit::Int(li) = &l.lit
                && let Ok(v) = li.base10_parse::<i128>()
            {
                out.push(v);
            }
        }
        Pat::Paren(p) => collect_pat_int_literals(&p.pat, out),
        Pat::Reference(r) => collect_pat_int_literals(&r.pat, out),
        Pat::Or(or) => {
            for c in &or.cases {
                collect_pat_int_literals(c, out);
            }
        }
        Pat::Ident(pi) => {
            if let Some((_, p)) = &pi.subpat {
                collect_pat_int_literals(p, out);
            }
        }
        Pat::Const(pc) => {
            if let [syn::Stmt::Expr(expr, None)] = pc.block.stmts.as_slice()
                && let Some(v) = eval_int_literal_expr(expr)
            {
                out.push(v);
            }
        }
        Pat::Verbatim(ts) => {
            if let Some(v) = syn::parse2::<Expr>(ts.clone())
                .ok()
                .and_then(|e| eval_int_literal_expr(&e))
            {
                out.push(v);
            }
        }
        _ => {}
    }
}

fn block_early_returns_enum_variant(
    block: &syn::Block,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    match block.stmts.as_slice() {
        [syn::Stmt::Expr(Expr::Return(r), _)] => r.expr.as_deref().is_some_and(|e| {
            expr_constructs_enum_variant(e, target_enum, self_ty, scope, variants)
        }),
        _ => false,
    }
}

fn block_constructs_enum_variant(
    block: &syn::Block,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    match block.stmts.as_slice() {
        [syn::Stmt::Expr(inner, _)] => {
            expr_constructs_enum_variant(inner, target_enum, self_ty, scope, variants)
        }
        _ => false,
    }
}

fn expr_constructs_enum_variant(
    mut expr: &Expr,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    loop {
        match expr {
            Expr::Paren(p) => expr = &p.expr,
            Expr::Group(g) => expr = &g.expr,
            Expr::Return(r) => match &r.expr {
                Some(inner) => expr = inner,
                None => return false,
            },
            Expr::Block(b) if b.block.stmts.len() == 1 => match &b.block.stmts[0] {
                syn::Stmt::Expr(inner, _) => expr = inner,
                _ => return false,
            },
            Expr::Call(call) if call.args.len() == 1 => {
                if let Expr::Path(ep) = &*call.func
                    && let Some(last) = ep.path.segments.last()
                    && matches!(last.ident.to_string().as_str(), "Some" | "Ok")
                {
                    expr = &call.args[0];
                    continue;
                }
                return false;
            }
            Expr::Path(ep) => {
                return path_is_target_variant(&ep.path, target_enum, self_ty, scope, variants);
            }
            _ => return false,
        }
    }
}

fn path_is_target_variant(
    path: &syn::Path,
    target_enum: &str,
    self_ty: Option<&str>,
    scope: &ImportScope,
    variants: &BTreeSet<String>,
) -> bool {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.len() < 2 {
        return segs.len() == 1 && variants.contains(&segs[0]);
    }
    let variant = &segs[segs.len() - 1];
    if !variants.contains(variant) {
        return false;
    }
    let parent = &segs[..segs.len() - 1];
    if parent == ["Self"] {
        return self_ty == Some(target_enum);
    }
    if let Some(q) = scope.resolve_type_segments(parent, self_ty) {
        return q.type_name == target_enum;
    }
    parent.last().is_some_and(|s| s == target_enum)
}
