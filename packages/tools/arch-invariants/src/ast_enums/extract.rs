//! AST extraction of `MatchSite` fingerprints from `Expr::Match` and `matches!` invocations.

use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, Item, Pat, Stmt};

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at};

use super::fingerprint::MIN_DUPLICATE_ARMS;
use super::normalize::{extract_body_mirror_atom, is_noop_body, NormCtx};
use super::patterns::{extract_arm_patterns, is_catchall_pat, ImportScope, MirrorAtom};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MatchDirection {
    EnumToBody,
    LitToEnum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedArm {
    pub arm_index: usize,
    pub is_enum_pat: bool,
    pub pat_qualifier: Option<String>,
    pub pat_key: String,
    pub body_key: String,
    pub is_noop: bool,
    pub mirror_pair: Option<(MirrorAtom, MirrorAtom)>,
    pub body_enum_qualifiers: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct MatchSite {
    pub file: String,
    pub symbol: String,
    pub line: usize,
    pub snippet: String,
    pub(super) direction: Option<MatchDirection>,
    pub enum_name: String,
    pub(super) qualifier: Option<String>,
    pub(super) glob_enums: BTreeSet<String>,
    pub(super) has_unscoped_glob: bool,
    pub(super) arms: Vec<NormalizedArm>,
    pub(super) wildcard_body: Option<String>,
    pub(super) mirror_pairs: Vec<(MirrorAtom, MirrorAtom)>,
}

pub fn extract_match_sites(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<MatchSite>) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    let mut scope = ImportScope::default();
    scope.ingest_items(&ast.items);
    extract_from_items(rel, raw, &ast.items, &scope, None, out);
}

fn extract_from_items(
    rel: &str,
    raw: &str,
    items: &[Item],
    scope: &ImportScope,
    self_ty: Option<&str>,
    out: &mut Vec<MatchSite>,
) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    let mut child_scope = scope.clone();
                    child_scope.ingest_items(sub);
                    extract_from_items(rel, raw, sub, &child_scope, None, out);
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                let sym = f.sig.ident.to_string();
                let mut fn_scope = scope.clone();
                ingest_block_uses(&f.block, &mut fn_scope);
                MatchExtractor {
                    rel,
                    raw,
                    symbol: &sym,
                    scope: &fn_scope,
                    self_ty,
                    out,
                }
                .visit_block(&f.block);
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let impl_ty = last_type_ident(&imp.self_ty);
                let impl_ty_ref = impl_ty.as_deref();
                for ii in &imp.items {
                    match ii {
                        syn::ImplItem::Fn(f) if !is_cfg_test_attr(&f.attrs) => {
                            let sym = match impl_ty_ref {
                                Some(t) => format!("{t}::{}", f.sig.ident),
                                None => f.sig.ident.to_string(),
                            };
                            let mut fn_scope = scope.clone();
                            ingest_block_uses(&f.block, &mut fn_scope);
                            MatchExtractor {
                                rel,
                                raw,
                                symbol: &sym,
                                scope: &fn_scope,
                                self_ty: impl_ty_ref,
                                out,
                            }
                            .visit_block(&f.block);
                        }
                        syn::ImplItem::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                            let sym = match impl_ty_ref {
                                Some(t) => format!("{t}::{}", c.ident),
                                None => c.ident.to_string(),
                            };
                            MatchExtractor {
                                rel,
                                raw,
                                symbol: &sym,
                                scope,
                                self_ty: impl_ty_ref,
                                out,
                            }
                            .visit_expr(&c.expr);
                        }
                        _ => {}
                    }
                }
            }
            Item::Trait(tr) if !is_cfg_test_attr(&tr.attrs) => {
                let tr_name = tr.ident.to_string();
                for ti in &tr.items {
                    if let syn::TraitItem::Fn(f) = ti
                        && !is_cfg_test_attr(&f.attrs)
                        && let Some(block) = &f.default
                    {
                        let sym = format!("{tr_name}::{}", f.sig.ident);
                        let mut fn_scope = scope.clone();
                        ingest_block_uses(block, &mut fn_scope);
                        MatchExtractor {
                            rel,
                            raw,
                            symbol: &sym,
                            scope: &fn_scope,
                            self_ty: None,
                            out,
                        }
                        .visit_block(block);
                    }
                }
            }
            Item::Const(c) if !is_cfg_test_attr(&c.attrs) => {
                let sym = format!("const {}", c.ident);
                MatchExtractor {
                    rel,
                    raw,
                    symbol: &sym,
                    scope,
                    self_ty,
                    out,
                }
                .visit_expr(&c.expr);
            }
            Item::Static(s) if !is_cfg_test_attr(&s.attrs) => {
                let sym = format!("static {}", s.ident);
                MatchExtractor {
                    rel,
                    raw,
                    symbol: &sym,
                    scope,
                    self_ty,
                    out,
                }
                .visit_expr(&s.expr);
            }
            _ => {}
        }
    }
}

fn ingest_block_uses(block: &syn::Block, scope: &mut ImportScope) {
    for stmt in &block.stmts {
        if let Stmt::Item(Item::Use(u)) = stmt
            && !is_cfg_test_attr(&u.attrs)
        {
            let mut prefix = Vec::new();
            scope.ingest_tree(&u.tree, &mut prefix);
        }
    }
}

struct MatchExtractor<'a> {
    rel: &'a str,
    raw: &'a str,
    symbol: &'a str,
    scope: &'a ImportScope,
    self_ty: Option<&'a str>,
    out: &'a mut Vec<MatchSite>,
}

impl<'ast> Visit<'ast> for MatchExtractor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if let Item::Fn(f) = item
            && is_cfg_test_attr(&f.attrs)
        {
            return;
        }
        if let Item::Mod(m) = item
            && is_cfg_test_attr(&m.attrs)
        {
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
        if let Some(site) =
            build_match_site(self.rel, self.raw, self.symbol, self.scope, self.self_ty, m)
        {
            self.out.push(site);
        }
        visit::visit_expr_match(self, m);
    }

    fn visit_expr_macro(&mut self, em: &'ast syn::ExprMacro) {
        if em.mac.path.is_ident("matches")
            && let Some(site) = build_matches_macro_site(
                self.rel,
                self.raw,
                self.symbol,
                self.scope,
                self.self_ty,
                em,
            )
        {
            self.out.push(site);
        }
        visit::visit_expr_macro(self, em);
    }
}

fn build_match_site(
    rel: &str,
    raw: &str,
    symbol: &str,
    scope: &ImportScope,
    self_ty: Option<&str>,
    m: &syn::ExprMatch,
) -> Option<MatchSite> {
    let mut arms = Vec::new();
    let mut wildcard_body = None;
    let mut seen = BTreeSet::new();

    for (arm_index, arm) in m.arms.iter().enumerate() {
        if is_cfg_test_attr(&arm.attrs) {
            continue;
        }
        if arm.guard.is_none() && is_catchall_pat(&arm.pat) {
            let mut ctx = NormCtx::new(scope, self_ty, &[]);
            wildcard_body = Some(ctx.norm_arm_body(&arm.body));
            continue;
        }
        let extracted = extract_arm_patterns(&arm.pat, scope, self_ty);
        for pat in extracted {
            let mut ctx = NormCtx::new(scope, self_ty, &pat.bindings);
            let guard_str = arm.guard.as_ref().map(|(_, g)| ctx.norm_expr(g));
            let body_key = ctx.norm_arm_body(&arm.body);
            let final_pat_key = ctx.finalize_pat_key(&pat.pat_key, guard_str.as_deref());
            let is_noop = is_noop_body(&body_key);
            let mirror_pair = if arm.guard.is_none() {
                pat.mirror_atom
                    .clone()
                    .and_then(|l| extract_body_mirror_atom(&arm.body, scope, self_ty).map(|r| (l, r)))
            } else {
                None
            };
            if seen.insert((final_pat_key.clone(), body_key.clone())) {
                arms.push(NormalizedArm {
                    arm_index,
                    is_enum_pat: pat.is_enum_variant,
                    pat_qualifier: pat.enum_qualifier,
                    pat_key: final_pat_key,
                    body_key,
                    is_noop,
                    mirror_pair,
                    body_enum_qualifiers: ctx.referenced_enum_qualifiers,
                });
            }
        }
    }

    finalize_site(
        rel,
        raw,
        symbol,
        m.match_token.span.start().line,
        scope,
        arms,
        wildcard_body,
    )
}

fn build_matches_macro_site(
    rel: &str,
    raw: &str,
    symbol: &str,
    scope: &ImportScope,
    self_ty: Option<&str>,
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
    let extracted = extract_arm_patterns(&parsed.pat, scope, self_ty);
    if extracted.len() < MIN_DUPLICATE_ARMS {
        return None;
    }
    let mut arms = Vec::new();
    let mut seen = BTreeSet::new();
    for pat in extracted {
        if !pat.is_enum_variant {
            continue;
        }
        let mut ctx = NormCtx::new(scope, self_ty, &pat.bindings);
        let guard_str = parsed.guard.as_ref().map(|g| ctx.norm_expr(g));
        let final_pat_key = ctx.finalize_pat_key(&pat.pat_key, guard_str.as_deref());
        if seen.insert(final_pat_key.clone()) {
            arms.push(NormalizedArm {
                arm_index: 0,
                is_enum_pat: true,
                pat_qualifier: pat.enum_qualifier,
                pat_key: final_pat_key,
                body_key: "true".to_string(),
                is_noop: false,
                mirror_pair: None,
                body_enum_qualifiers: BTreeSet::new(),
            });
        }
    }

    finalize_site(
        rel,
        raw,
        symbol,
        em.mac.path.span().start().line,
        scope,
        arms,
        Some("false".to_string()),
    )
}

fn finalize_site(
    rel: &str,
    raw: &str,
    symbol: &str,
    line: usize,
    scope: &ImportScope,
    arms: Vec<NormalizedArm>,
    wildcard_body: Option<String>,
) -> Option<MatchSite> {
    let enum_arms: Vec<&NormalizedArm> = arms.iter().filter(|a| a.is_enum_pat).collect();
    let lit_to_enum_arms: Vec<&NormalizedArm> = arms
        .iter()
        .filter(|a| !a.is_enum_pat && !a.body_enum_qualifiers.is_empty())
        .collect();
    let mirror_pairs: Vec<(MirrorAtom, MirrorAtom)> = arms
        .iter()
        .filter_map(|a| a.mirror_pair.clone())
        .filter(|(l, r)| l != r && (l.is_variant() || r.is_variant()))
        .collect();

    let (direction, qualifier, kept_arms) = if enum_arms.len() >= MIN_DUPLICATE_ARMS {
        let qual = dominant_qualifier(enum_arms.iter().filter_map(|a| a.pat_qualifier.as_deref()));
        let kept = arms.into_iter().filter(|a| a.is_enum_pat).collect();
        (Some(MatchDirection::EnumToBody), qual, kept)
    } else if lit_to_enum_arms.len() >= MIN_DUPLICATE_ARMS {
        let qual = dominant_qualifier(
            lit_to_enum_arms
                .iter()
                .flat_map(|a| a.body_enum_qualifiers.iter().map(String::as_str)),
        )
        .map(|q| format!("lit->{q}"));
        let kept = arms
            .into_iter()
            .filter(|a| !a.is_enum_pat && !a.body_enum_qualifiers.is_empty())
            .collect();
        (Some(MatchDirection::LitToEnum), qual, kept)
    } else if mirror_pairs.len() >= MIN_DUPLICATE_ARMS {
        let qual = dominant_qualifier(mirror_pairs.iter().filter_map(|(l, r)| match (l, r) {
            (MirrorAtom::Variant { qualifier: Some(q), .. }, _)
            | (_, MirrorAtom::Variant { qualifier: Some(q), .. }) => Some(q.as_str()),
            _ => None,
        }));
        (None, qual, Vec::new())
    } else {
        return None;
    };

    let enum_name = qualifier
        .as_deref()
        .map(|q| q.strip_prefix("lit->").unwrap_or(q))
        .unwrap_or("<unqualified>")
        .to_string();

    Some(MatchSite {
        file: rel.to_string(),
        symbol: symbol.to_string(),
        line,
        snippet: line_at(raw, line).trim().to_string(),
        direction,
        enum_name,
        qualifier,
        glob_enums: scope.glob_enums.clone(),
        has_unscoped_glob: scope.has_unscoped_glob,
        arms: kept_arms,
        wildcard_body,
        mirror_pairs,
    })
}

fn dominant_qualifier<'a>(iter: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts: BTreeMap<&'a str, usize> = BTreeMap::new();
    for q in iter {
        *counts.entry(q).or_insert(0) += 1;
    }
    if counts.len() == 1 {
        counts.into_keys().next().map(ToString::to_string)
    } else {
        counts
            .into_iter()
            .max_by_key(|&(_, c)| c)
            .filter(|&(_, c)| c >= MIN_DUPLICATE_ARMS)
            .map(|(q, _)| q.to_string())
    }
}
