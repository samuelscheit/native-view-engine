//! Staged (`Stage::Post1B1D`) single-file AST checks unified onto `patterns::MirrorAtom`:
//! - compact integer-discriminant `match` (`>= 3` arms),
//! - core enum `<->` string literal tables (`>= 3` arms in `packages/core/{tables,components}`).
//!
//! Mirrored bidirectional pairs are now handled cross-file by `fingerprint.rs` (zero double-reporting).

use std::collections::BTreeSet;

use syn::visit::{self, Visit};
use syn::Item;

use crate::ast_layout::last_type_ident;
use crate::{is_cfg_test_attr, line_at, Violation};

use super::normalize_helpers::extract_body_mirror_atom;
use super::patterns::{
    extract_arm_patterns_checked, is_compact_discriminant_set, MirrorAtom, PatternExpansion,
};
use super::scope::ImportScope;

struct FnMirrorProfile {
    name: String,
    line: usize,
    pairs: BTreeSet<(MirrorAtom, MirrorAtom)>,
}

pub fn scan_match_tables(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    let mut scope = ImportScope::default();
    scope.ingest_items(&ast.items);
    let mut fns = Vec::new();
    collect_fn_profiles(&ast.items, &scope, &mut fns);

    if !rel.starts_with("packages/core/codegen/") {
        for f in &fns {
            let i2v: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (MirrorAtom::Int(n), MirrorAtom::Variant { qualifier, .. })
                        if !qualifier.as_ref().is_some_and(|q| q.is_external_sdk()) =>
                    {
                        Some(*n)
                    }
                    _ => None,
                })
                .collect();
            let v2i: Vec<i128> = f
                .pairs
                .iter()
                .filter_map(|(l, r)| match (l, r) {
                    (MirrorAtom::Variant { qualifier, .. }, MirrorAtom::Int(n))
                        if !qualifier.as_ref().is_some_and(|q| q.is_external_sdk()) =>
                    {
                        Some(*n)
                    }
                    _ => None,
                })
                .collect();
            let count = if is_compact_discriminant_set(&i2v, 3) {
                i2v.len()
            } else if is_compact_discriminant_set(&v2i, 3) {
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
                        "Manual integer-discriminant `match` in `fn {}` ({count} arms); derive via `#[derive(ReprEnum)]` / `#[derive(WordEnum)]` / `#[derive(NveCodec)]`",
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
                .filter(|(l, r)| matches!((l, r), (MirrorAtom::Variant { .. }, MirrorAtom::Str(_))))
                .count();
            let s2v = f
                .pairs
                .iter()
                .filter(|(l, r)| matches!((l, r), (MirrorAtom::Str(_), MirrorAtom::Variant { .. })))
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

fn collect_fn_profiles(items: &[Item], scope: &ImportScope, out: &mut Vec<FnMirrorProfile>) {
    for item in items {
        match item {
            Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                if let Some((_, sub)) = &m.content {
                    let child = ImportScope::child_for_inline_mod(scope, sub);
                    collect_fn_profiles(sub, &child, out);
                }
            }
            Item::Impl(imp) if !is_cfg_test_attr(&imp.attrs) => {
                let skip_trait = imp.trait_.as_ref().is_some_and(|(_, p, _)| {
                    p.segments
                        .last()
                        .is_some_and(|s| s.ident == "Display" || s.ident == "Debug")
                });
                let impl_ty = last_type_ident(&imp.self_ty);
                let skip_err = impl_ty.as_deref().is_some_and(|n| n.ends_with("Error"));
                if skip_trait || skip_err {
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(f) = ii
                        && !is_cfg_test_attr(&f.attrs)
                        && f.sig.ident != "fmt"
                    {
                        record_fn_profile(&f.sig, &f.block, scope, impl_ty.as_deref(), out);
                    }
                }
            }
            Item::Fn(f) if !is_cfg_test_attr(&f.attrs) && f.sig.ident != "fmt" => {
                record_fn_profile(&f.sig, &f.block, scope, None, out);
            }
            _ => {}
        }
    }
}

fn record_fn_profile(
    sig: &syn::Signature,
    block: &syn::Block,
    scope: &ImportScope,
    self_ty: Option<&str>,
    out: &mut Vec<FnMirrorProfile>,
) {
    struct MatchScan<'a> {
        scope: &'a ImportScope,
        self_ty: Option<&'a str>,
        pairs: BTreeSet<(MirrorAtom, MirrorAtom)>,
    }
    impl<'ast> Visit<'ast> for MatchScan<'_> {
        fn visit_expr_match(&mut self, m: &'ast syn::ExprMatch) {
            if is_cfg_test_attr(&m.attrs) {
                return;
            }
            let mut match_pairs = Vec::new();
            let mut overflowed = false;
            for arm in &m.arms {
                if arm.guard.is_none()
                    && !is_cfg_test_attr(&arm.attrs)
                    && let Some(rhs) = extract_body_mirror_atom(&arm.body, self.scope, self.self_ty)
                {
                    match extract_arm_patterns_checked(&arm.pat, self.scope, self.self_ty) {
                        PatternExpansion::Ok(pats) => {
                            for pat in pats {
                                if let Some(lhs) = pat.mirror_atom {
                                    match_pairs.push((lhs, rhs.clone()));
                                }
                            }
                        }
                        PatternExpansion::Overflow => {
                            overflowed = true;
                            break;
                        }
                    }
                }
            }
            if !overflowed {
                self.pairs.extend(match_pairs);
            }
            visit::visit_expr_match(self, m);
        }
    }
    let mut scan = MatchScan {
        scope,
        self_ty,
        pairs: BTreeSet::new(),
    };
    scan.visit_block(block);
    if !scan.pairs.is_empty() {
        out.push(FnMirrorProfile {
            name: sig.ident.to_string(),
            line: sig.ident.span().start().line,
            pairs: scan.pairs,
        });
    }
}
