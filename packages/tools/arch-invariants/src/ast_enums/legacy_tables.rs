//! Single-file AST checks kept for `Stage::Post1B1D`:
//! - same-file mirrored `<->` pairs (`>= 3` arms),
//! - compact integer-discriminant `match` (`>= 3` arms),
//! - core enum `<->` string literal tables (`>= 3` arms in `packages/core/{tables,components}`).

use std::collections::BTreeSet;

use syn::visit::{self, Visit};
use syn::{Expr, Item, Lit, Pat};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

use super::patterns::EXTERNAL_SDK_ROOTS;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ArmAtom {
    Str(String),
    Int(i128),
    Variant(String),
}

struct FnMatchProfile {
    name: String,
    line: usize,
    pairs: BTreeSet<(ArmAtom, ArmAtom)>,
}

pub fn scan_match_tables(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    let mut fns = Vec::new();
    collect_fn_profiles(&ast.items, &mut fns);

    let mut reported_mirror = BTreeSet::new();
    for a in 0..fns.len() {
        if fns[a].pairs.len() < 3 {
            continue;
        }
        for b in (a + 1)..fns.len() {
            if fns[b].pairs.len() < 3 {
                continue;
            }
            let overlap = fns[a]
                .pairs
                .iter()
                .filter(|(l, r)| l != r && fns[b].pairs.contains(&(r.clone(), l.clone())))
                .count();
            if overlap >= 3 {
                reported_mirror.insert(a);
                reported_mirror.insert(b);
                out.push(Violation::new(
                    rel,
                    fns[a].line,
                    line_at(raw, fns[a].line),
                    format!(
                        "Duplicated bidirectional mapping between `fn {}` (line {}) and `fn {}` (line {}): {overlap} mirrored `match` arms — generate from one source of truth",
                        fns[a].name, fns[a].line, fns[b].name, fns[b].line
                    ),
                ));
            }
        }
    }

    if !rel.starts_with("packages/core/codegen/") {
        for (idx, f) in fns.iter().enumerate() {
            if reported_mirror.contains(&idx) {
                continue;
            }
            let i2v: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Int(n), ArmAtom::Variant(_)) => Some(*n),
                    _ => None,
                })
                .collect();
            let v2i: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (ArmAtom::Variant(_), ArmAtom::Int(n)) => Some(*n),
                    _ => None,
                })
                .collect();
            let count = if is_compact_discriminant_set(&i2v) {
                i2v.len()
            } else if is_compact_discriminant_set(&v2i) {
                v2i.len()
            } else {
                0
            };
            if count >= 3 {
                out.push(Violation::new(
                    rel,
                    f.line,
                    line_at(raw, f.line),
                    format!(
                        "Manual integer-discriminant `match` in `fn {}` ({count} arms); derive via `#[derive(WordEnum)]` / `#[derive(NveCodec)]`",
                        f.name
                    ),
                ));
            }
        }
    }

    if rel.starts_with("packages/core/tables/") || rel.starts_with("packages/core/components/") {
        for f in &fns {
            let v2s = f
                .pairs
                .iter()
                .filter(|(l, r)| matches!((l, r), (ArmAtom::Variant(_), ArmAtom::Str(_))))
                .count();
            let s2v = f
                .pairs
                .iter()
                .filter(|(l, r)| matches!((l, r), (ArmAtom::Str(_), ArmAtom::Variant(_))))
                .count();
            if v2s >= 3 || s2v >= 3 {
                let count = v2s.max(s2v);
                out.push(Violation::new(
                    rel,
                    f.line,
                    line_at(raw, f.line),
                    format!(
                        "Core function `{}` maps {count} enum variants <-> string literals; move CSS/HTML/ARIA keyword tables to `nve-backend-web`",
                        f.name
                    ),
                ));
            }
        }
    }
}

fn collect_fn_profiles(items: &[Item], out: &mut Vec<FnMatchProfile>) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    collect_fn_profiles(sub, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let skip_trait = imp.trait_.as_ref().is_some_and(|(_, p, _)| {
                    p.segments
                        .last()
                        .is_some_and(|s| s.ident == "Display" || s.ident == "Debug")
                });
                let skip_err = last_type_ident(&imp.self_ty).is_some_and(|n| n.ends_with("Error"));
                if skip_trait || skip_err {
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                        && f.sig.ident != "fmt"
                    {
                        record_fn_profile(&f.sig, &f.block, out);
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) && f.sig.ident != "fmt" => {
                record_fn_profile(&f.sig, &f.block, out);
            }
            _ => {}
        }
    }
}

fn record_fn_profile(sig: &syn::Signature, block: &syn::Block, out: &mut Vec<FnMatchProfile>) {
    struct MatchScan(BTreeSet<(ArmAtom, ArmAtom)>);
    impl<'ast> Visit<'ast> for MatchScan {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            for arm in &m.arms {
                if arm.guard.is_none()
                    && let Some(rhs) = expr_atom(&arm.body)
                {
                    collect_pat_atoms(&arm.pat, &rhs, &mut self.0);
                }
            }
            visit::visit_expr_match(self, m);
        }
    }
    let mut scan = MatchScan(BTreeSet::new());
    scan.visit_block(block);
    if !scan.0.is_empty() {
        out.push(FnMatchProfile {
            name: sig.ident.to_string(),
            line: sig.ident.span().start().line,
            pairs: scan.0,
        });
    }
}

fn collect_pat_atoms(pat: &Pat, rhs: &ArmAtom, out: &mut BTreeSet<(ArmAtom, ArmAtom)>) {
    match pat {
        Pat::Or(or) => or.cases.iter().for_each(|c| collect_pat_atoms(c, rhs, out)),
        _ => {
            if let Some(lhs) = pat_atom(pat) {
                out.insert((lhs, rhs.clone()));
            }
        }
    }
}

fn pat_atom(pat: &Pat) -> Option<ArmAtom> {
    match pat {
        Pat::Lit(lit) => lit_atom(&lit.lit, false),
        Pat::Path(p) => path_variant_atom(&p.path),
        Pat::TupleStruct(ts) => {
            let last = ts.path.segments.last()?.ident.to_string();
            ((last == "Some" || last == "Ok") && ts.elems.len() == 1)
                .then(|| pat_atom(ts.elems.first()?))
                .flatten()
        }
        Pat::Paren(p) => pat_atom(&p.pat),
        _ => None,
    }
}

fn expr_atom(expr: &Expr) -> Option<ArmAtom> {
    match expr {
        Expr::Lit(l) => lit_atom(&l.lit, false),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Neg(_)) => match &*u.expr {
            Expr::Lit(l) => lit_atom(&l.lit, true),
            _ => None,
        },
        Expr::Path(p) => path_variant_atom(&p.path),
        Expr::Call(c) => {
            if let Expr::Path(p) = &*c.func
                && let Some(last) = p.path.segments.last()
                && (last.ident == "Some" || last.ident == "Ok")
                && c.args.len() == 1
            {
                return expr_atom(c.args.first()?);
            }
            None
        }
        Expr::Paren(p) => expr_atom(&p.expr),
        Expr::Group(g) => expr_atom(&g.expr),
        _ => None,
    }
}

fn lit_atom(lit: &Lit, neg: bool) -> Option<ArmAtom> {
    match lit {
        Lit::Str(s) if !neg => Some(ArmAtom::Str(s.value())),
        Lit::Int(i) => i
            .base10_parse::<i128>()
            .ok()
            .map(|v| ArmAtom::Int(if neg { -v } else { v })),
        _ => None,
    }
}

fn path_variant_atom(path: &syn::Path) -> Option<ArmAtom> {
    if path.segments.len() < 2
        || path
            .segments
            .iter()
            .any(|s| EXTERNAL_SDK_ROOTS.contains(&s.ident.to_string().as_str()))
    {
        return None;
    }
    let leaf = path.segments.last()?.ident.to_string();
    leaf.starts_with(|c: char| c.is_ascii_uppercase())
        .then_some(ArmAtom::Variant(leaf))
}

fn is_compact_discriminant_set(vals: &[i128]) -> bool {
    let unique: BTreeSet<i128> = vals.iter().copied().collect();
    if unique.len() < 3 {
        return false;
    }
    let min = *unique.first().unwrap_or(&0);
    let max = *unique.last().unwrap_or(&0);
    min >= 0 && max <= 255 && (max - min) <= (unique.len() as i128 * 2 + 4)
}
