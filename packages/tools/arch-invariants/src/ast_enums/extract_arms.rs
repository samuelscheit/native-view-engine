//! Arm normalization, free-variable scanning, and `MatchSite` construction for
//! `Expr::Match`, `matches!`, and `if let` chains.

use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;
use syn::{Expr, Pat};

use crate::{is_cfg_test_attr, line_at};

use super::extract::{MatchDirection, MatchSite, NormalizedArm};
use super::fingerprint::MIN_DUPLICATE_ARMS;
use super::free_vars::collect_free_vars;
use super::normalize::{MatchOuterScope, NormCtx};
use super::normalize_helpers::{extract_body_mirror_atom, is_bare_outer_var, is_noop_body};
use super::patterns::{
    extract_arm_patterns_checked, is_catchall_pat, ExtractedPat, MirrorAtom, PatternExpansion,
};
use super::scope::{EnumQualifier, ImportScope};

pub(super) struct SiteBuildCtx<'a> {
    pub rel: &'a str,
    pub raw: &'a str,
    pub symbol: &'a str,
    pub scope: &'a ImportScope,
    pub self_ty: Option<&'a str>,
    pub enclosing_locals: &'a [String],
}

pub(super) type IfLetBranch<'a> = (&'a Pat, &'a Expr, &'a syn::Block);

pub(super) fn collect_if_let_chain<'a>(
    head: &'a syn::ExprIf,
) -> Option<(Vec<IfLetBranch<'a>>, Option<&'a Expr>)> {
    let Expr::Let(first_let) = &*head.cond else {
        return None;
    };
    let scr_key = expr_scrutinee_key(&first_let.expr);
    let mut branches = vec![(&*first_let.pat, &*first_let.expr, &head.then_branch)];
    let mut cur_else = head.else_branch.as_ref().map(|(_, e)| &**e);

    while let Some(else_expr) = cur_else {
        if let Expr::If(next_if) = else_expr
            && !is_cfg_test_attr(&next_if.attrs)
            && let Expr::Let(next_let) = &*next_if.cond
            && expr_scrutinee_key(&next_let.expr) == scr_key
        {
            branches.push((&*next_let.pat, &*next_let.expr, &next_if.then_branch));
            cur_else = next_if.else_branch.as_ref().map(|(_, e)| &**e);
        } else {
            return Some((branches, Some(else_expr)));
        }
    }
    Some((branches, None))
}

fn expr_scrutinee_key(expr: &Expr) -> String {
    match expr {
        Expr::Paren(p) => expr_scrutinee_key(&p.expr),
        Expr::Group(g) => expr_scrutinee_key(&g.expr),
        Expr::Reference(r) => expr_scrutinee_key(&r.expr),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => expr_scrutinee_key(&u.expr),
        Expr::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        Expr::Field(f) => {
            let mem = match &f.member {
                syn::Member::Named(id) => id.to_string(),
                syn::Member::Unnamed(idx) => idx.index.to_string(),
            };
            format!("{}.{mem}", expr_scrutinee_key(&f.base))
        }
        Expr::Index(i) => format!(
            "{}[{}]",
            expr_scrutinee_key(&i.expr),
            expr_scrutinee_key(&i.index)
        ),
        Expr::MethodCall(mc) => {
            let args: Vec<String> = mc.args.iter().map(expr_scrutinee_key).collect();
            format!(
                "{}.{}({})",
                expr_scrutinee_key(&mc.receiver),
                mc.method,
                args.join(",")
            )
        }
        Expr::Call(c) => {
            let args: Vec<String> = c.args.iter().map(expr_scrutinee_key).collect();
            format!("{}({})", expr_scrutinee_key(&c.func), args.join(","))
        }
        Expr::Tuple(t) => {
            let es: Vec<String> = t.elems.iter().map(expr_scrutinee_key).collect();
            format!("({})", es.join(","))
        }
        _ => format!("@{}", expr.span().start().line),
    }
}

struct PendingArm<'a> {
    arm_index: usize,
    pat: ExtractedPat,
    guard: Option<&'a Expr>,
    body: &'a Expr,
}

pub(super) fn build_match_site(
    ctx: &SiteBuildCtx<'_>,
    opaque_seed: &mut usize,
    m: &syn::ExprMatch,
) -> Option<MatchSite> {
    let mut pending = Vec::new();
    let mut wild_expr: Option<&Expr> = None;

    for (arm_index, arm) in m.arms.iter().enumerate() {
        if is_cfg_test_attr(&arm.attrs) {
            continue;
        }
        if arm.guard.is_none() && is_catchall_pat(&arm.pat) {
            wild_expr = Some(&arm.body);
            continue;
        }
        let pats = match extract_arm_patterns_checked(&arm.pat, ctx.scope, ctx.self_ty) {
            PatternExpansion::Ok(v) => v,
            PatternExpansion::Overflow => return None,
        };
        for pat in pats {
            pending.push(PendingArm {
                arm_index,
                pat,
                guard: arm.guard.as_ref().map(|(_, g)| &**g),
                body: &arm.body,
            });
        }
    }

    normalize_pending_arms(
        ctx,
        m.match_token.span.start().line,
        opaque_seed,
        pending,
        wild_expr,
    )
}

pub(super) fn build_if_let_chain_site(
    ctx: &SiteBuildCtx<'_>,
    opaque_seed: &mut usize,
    line: usize,
    branches: &[IfLetBranch<'_>],
    else_body: Option<&Expr>,
) -> Option<MatchSite> {
    let owned_blocks: Vec<Expr> = branches
        .iter()
        .map(|(_, _, blk)| {
            Expr::Block(syn::ExprBlock {
                attrs: Vec::new(),
                label: None,
                block: (*blk).clone(),
            })
        })
        .collect();

    let mut pending = Vec::new();
    for (arm_index, ((pat, _, _), body_expr)) in branches.iter().zip(&owned_blocks).enumerate() {
        let pats = match extract_arm_patterns_checked(pat, ctx.scope, ctx.self_ty) {
            PatternExpansion::Ok(v) => v,
            PatternExpansion::Overflow => return None,
        };
        for ep in pats {
            pending.push(PendingArm {
                arm_index,
                pat: ep,
                guard: None,
                body: body_expr,
            });
        }
    }

    normalize_pending_arms(ctx, line, opaque_seed, pending, else_body)
}

type OrBodyCacheEntry = (String, BTreeSet<EnumQualifier>, bool, Option<MirrorAtom>);

fn normalize_pending_arms(
    ctx: &SiteBuildCtx<'_>,
    line: usize,
    opaque_seed: &mut usize,
    mut pending: Vec<PendingArm<'_>>,
    wild_expr: Option<&Expr>,
) -> Option<MatchSite> {
    if pending.len() < MIN_DUPLICATE_ARMS {
        return None;
    }

    pending.sort_by(|a, b| (&a.pat.pat_key, a.arm_index).cmp(&(&b.pat.pat_key, b.arm_index)));

    let mut referenced_idents = BTreeSet::new();
    for p in &pending {
        collect_free_vars(p.body, &p.pat.bindings, &mut referenced_idents);
        if let Some(g) = p.guard {
            collect_free_vars(g, &p.pat.bindings, &mut referenced_idents);
        }
    }
    if let Some(w) = wild_expr {
        collect_free_vars(w, &[], &mut referenced_idents);
    }

    let mut outer = MatchOuterScope::new(*opaque_seed);
    outer.preseed_from_referenced(ctx.enclosing_locals, &referenced_idents);

    let mut cached_or_bodies: BTreeMap<usize, OrBodyCacheEntry> = BTreeMap::new();
    let mut arms = Vec::with_capacity(pending.len());
    let mut seen = BTreeSet::new();

    for item in pending {
        let (final_pat_key, body_key, body_enum_qualifiers, has_unqualified_body_enum, mirror_pair) =
            if item.pat.bindings.is_empty() && item.guard.is_none() {
                let (bkey, quals, has_unq, body_mirror) =
                    if let Some(hit) = cached_or_bodies.get(&item.arm_index) {
                        hit.clone()
                    } else {
                        let mut nctx = NormCtx::new(ctx.scope, ctx.self_ty, &[], &mut outer);
                        let bkey = nctx.norm_arm_body(item.body);
                        let bm = extract_body_mirror_atom(item.body, ctx.scope, ctx.self_ty);
                        let entry = (
                            bkey,
                            nctx.referenced_enum_qualifiers,
                            nctx.has_unqualified_enum_ref,
                            bm,
                        );
                        cached_or_bodies.insert(item.arm_index, entry.clone());
                        entry
                    };
                let mp = item.pat.mirror_atom.clone().zip(body_mirror);
                (item.pat.pat_key, bkey, quals, has_unq, mp)
            } else {
                let mut nctx =
                    NormCtx::new(ctx.scope, ctx.self_ty, &item.pat.bindings, &mut outer);
                let guard_str = item.guard.map(|g| nctx.norm_expr(g));
                let body_key = nctx.norm_arm_body(item.body);
                let final_pat_key = nctx.finalize_pat_key(&item.pat.pat_key, guard_str.as_deref());
                let mp = if item.guard.is_none() {
                    item.pat
                        .mirror_atom
                        .clone()
                        .zip(extract_body_mirror_atom(item.body, ctx.scope, ctx.self_ty))
                } else {
                    None
                };
                (
                    final_pat_key,
                    body_key,
                    nctx.referenced_enum_qualifiers,
                    nctx.has_unqualified_enum_ref,
                    mp,
                )
            };

        let is_noop = is_noop_body(&body_key);
        let is_bare_outer = is_bare_outer_var(&body_key);
        if seen.insert((final_pat_key.clone(), body_key.clone())) {
            arms.push(NormalizedArm {
                arm_index: item.arm_index,
                is_enum_pat: item.pat.is_enum_variant,
                pat_qualifier: item.pat.enum_qualifier,
                tuple_qualifiers: item.pat.tuple_qualifiers,
                variant_head: item.pat.variant_head,
                pat_key: final_pat_key,
                body_key,
                is_noop,
                is_bare_outer,
                mirror_pair,
                body_enum_qualifiers,
                has_unqualified_body_enum,
            });
        }
    }

    let wildcard_body = wild_expr.map(|w| {
        let mut nctx = NormCtx::new(ctx.scope, ctx.self_ty, &[], &mut outer);
        nctx.norm_arm_body(w)
    });
    *opaque_seed = outer.next_opaque_seed();

    finalize_site(ctx, line, arms, wildcard_body)
}

pub(super) fn build_matches_macro_site(
    ctx: &SiteBuildCtx<'_>,
    opaque_seed: &mut usize,
    em: &syn::ExprMacro,
) -> Option<MatchSite> {
    struct MatchesInput {
        pat: Pat,
        guard: Option<Expr>,
    }
    impl syn::parse::Parse for MatchesInput {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let _expr: Expr = input.parse()?;
            let _comma: syn::Token![,] = input.parse()?;
            let pat = Pat::parse_multi_with_leading_vert(input)?;
            let guard = if input.peek(syn::Token![if]) {
                let _if: syn::Token![if] = input.parse()?;
                Some(input.parse()?)
            } else {
                None
            };
            let _ = input.parse::<Option<syn::Token![,]>>();
            Ok(Self { pat, guard })
        }
    }

    let parsed: MatchesInput = em.mac.parse_body().ok()?;
    let mut extracted = match extract_arm_patterns_checked(&parsed.pat, ctx.scope, ctx.self_ty) {
        PatternExpansion::Ok(v) => v,
        PatternExpansion::Overflow => return None,
    };
    if extracted.len() < MIN_DUPLICATE_ARMS {
        return None;
    }
    extracted.sort_by(|a, b| a.pat_key.cmp(&b.pat_key));

    let mut referenced_idents = BTreeSet::new();
    if let Some(g) = &parsed.guard {
        let bound: Vec<String> = extracted
            .first()
            .map(|p| p.bindings.clone())
            .unwrap_or_default();
        collect_free_vars(g, &bound, &mut referenced_idents);
    }
    let mut outer = MatchOuterScope::new(*opaque_seed);
    outer.preseed_from_referenced(ctx.enclosing_locals, &referenced_idents);

    let mut arms = Vec::new();
    let mut seen = BTreeSet::new();
    for pat in extracted {
        if !pat.is_enum_variant {
            continue;
        }
        let mut nctx = NormCtx::new(ctx.scope, ctx.self_ty, &pat.bindings, &mut outer);
        let guard_str = parsed.guard.as_ref().map(|g| nctx.norm_expr(g));
        let final_pat_key = nctx.finalize_pat_key(&pat.pat_key, guard_str.as_deref());
        if seen.insert(final_pat_key.clone()) {
            arms.push(NormalizedArm {
                arm_index: 0,
                is_enum_pat: true,
                pat_qualifier: pat.enum_qualifier,
                tuple_qualifiers: pat.tuple_qualifiers,
                variant_head: pat.variant_head,
                pat_key: final_pat_key,
                body_key: "true".to_string(),
                is_noop: false,
                is_bare_outer: false,
                mirror_pair: None,
                body_enum_qualifiers: BTreeSet::new(),
                has_unqualified_body_enum: false,
            });
        }
    }
    *opaque_seed = outer.next_opaque_seed();

    finalize_site(
        ctx,
        em.mac.path.span().start().line,
        arms,
        Some("false".to_string()),
    )
}

fn finalize_site(
    ctx: &SiteBuildCtx<'_>,
    line: usize,
    arms: Vec<NormalizedArm>,
    wildcard_body: Option<String>,
) -> Option<MatchSite> {
    let enum_arms: Vec<&NormalizedArm> = arms.iter().filter(|a| a.is_enum_pat).collect();
    let lit_to_enum_arms: Vec<&NormalizedArm> = arms
        .iter()
        .filter(|a| {
            !a.is_enum_pat && (!a.body_enum_qualifiers.is_empty() || a.has_unqualified_body_enum)
        })
        .collect();
    let mirror_pairs: Vec<(MirrorAtom, MirrorAtom)> = arms
        .iter()
        .filter_map(|a| a.mirror_pair.clone())
        .filter(|(l, r)| l != r && (l.is_variant() || r.is_variant()))
        .collect();

    let (direction, enum_qualifier, tuple_qualifiers, kept_arms) =
        if enum_arms.len() >= MIN_DUPLICATE_ARMS {
            let eq = dominant_item(enum_arms.iter().filter_map(|a| a.pat_qualifier.clone()));
            let tq = dominant_item(enum_arms.iter().filter_map(|a| a.tuple_qualifiers.clone()));
            let kept = arms.into_iter().filter(|a| a.is_enum_pat).collect();
            (Some(MatchDirection::EnumToBody), eq, tq, kept)
        } else if lit_to_enum_arms.len() >= MIN_DUPLICATE_ARMS {
            let eq = dominant_item(
                lit_to_enum_arms
                    .iter()
                    .flat_map(|a| a.body_enum_qualifiers.iter().cloned()),
            );
            let kept = arms
                .into_iter()
                .filter(|a| {
                    !a.is_enum_pat
                        && (!a.body_enum_qualifiers.is_empty() || a.has_unqualified_body_enum)
                })
                .collect();
            (Some(MatchDirection::LitToEnum), eq, None, kept)
        } else if mirror_pairs.len() >= MIN_DUPLICATE_ARMS {
            let eq = dominant_item(mirror_pairs.iter().filter_map(|(l, r)| match (l, r) {
                (MirrorAtom::Variant { qualifier: Some(q), .. }, _)
                | (_, MirrorAtom::Variant { qualifier: Some(q), .. }) => Some(q.clone()),
                _ => None,
            }));
            (None, eq, None, Vec::new())
        } else {
            return None;
        };

    let enum_name = enum_qualifier
        .as_ref()
        .map(EnumQualifier::display_name)
        .or_else(|| {
            tuple_qualifiers.as_ref().map(|qs| {
                let parts: Vec<String> = qs
                    .iter()
                    .map(|q| {
                        q.as_ref()
                            .map(EnumQualifier::display_name)
                            .unwrap_or_else(|| "_".to_string())
                    })
                    .collect();
                format!("({})", parts.join(","))
            })
        })
        .unwrap_or_else(|| "<unqualified>".to_string());

    Some(MatchSite {
        file: ctx.rel.to_string(),
        symbol: ctx.symbol.to_string(),
        line,
        snippet: line_at(ctx.raw, line).trim().to_string(),
        direction,
        enum_name,
        enum_qualifier,
        tuple_qualifiers,
        glob_enums: ctx.scope.glob_enums.clone(),
        has_unscoped_glob: ctx.scope.has_unscoped_glob,
        arms: kept_arms,
        wildcard_body,
        mirror_pairs,
    })
}

fn dominant_item<T: Ord>(iter: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: BTreeMap<T, usize> = BTreeMap::new();
    for item in iter {
        *counts.entry(item).or_insert(0) += 1;
    }
    if counts.len() == 1 {
        counts.into_keys().next()
    } else {
        counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .filter(|(_, c)| *c >= MIN_DUPLICATE_ARMS)
            .map(|(k, _)| k)
    }
}
