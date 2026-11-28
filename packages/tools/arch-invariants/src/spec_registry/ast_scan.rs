//! Full `syn::parse_file` AST verifier for `[INV:<id>]` test guards: walks `Item::Fn` and
//! `Item::Mod`, rejects disabled/ignored/`#[should_panic]` tests, skips dead/unexecuted scopes,
//! rejects literal tautologies, and downgrades mixed strict + ratchet tests to `Ratcheted`.

use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;

use super::{collect_valid_inv_tags, GuardStrength};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyGuardKind {
    Enforced,
    Ratcheted,
    Unenforced,
}

/// Extracts `[INV:<id>]` tags attached to active, non-ignored, non-`#[should_panic]` `#[test]`
/// functions parsed via `syn::parse_file` whose body contains a reachable, non-tautological
/// enforcing assertion, combining multiple guards per ID with weakest-link `.min(strength)`
/// (`Ratcheted` dominates `Enforced`).
pub fn extract_test_invariant_guards(src: &str) -> BTreeMap<String, GuardStrength> {
    let Ok(file) = syn::parse_file(src) else {
        return BTreeMap::new();
    };
    if has_disqualifying_cfg(&file.attrs) {
        return BTreeMap::new();
    }

    let lines: Vec<&str> = src.lines().collect();
    let mut line_starts = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        line_starts.push(offset);
        offset += line.len();
    }
    if line_starts.len() <= lines.len() {
        line_starts.push(offset);
    }

    let mut guards = BTreeMap::new();
    collect_item_guards(src, &lines, &line_starts, &file.items, &mut guards);
    guards
}

fn collect_item_guards(
    src: &str,
    lines: &[&str],
    line_starts: &[usize],
    items: &[syn::Item],
    guards: &mut BTreeMap<String, GuardStrength>,
) {
    for item in items {
        match item {
            syn::Item::Mod(item_mod) => {
                if has_disqualifying_cfg(&item_mod.attrs) {
                    continue;
                }
                if let Some((_, inner_items)) = &item_mod.content {
                    collect_item_guards(src, lines, line_starts, inner_items, guards);
                }
            }
            syn::Item::Fn(item_fn) => {
                if !is_active_test_fn(item_fn) {
                    continue;
                }
                let strength = match classify_test_block(&item_fn.block) {
                    BodyGuardKind::Enforced => GuardStrength::Enforced,
                    BodyGuardKind::Ratcheted => GuardStrength::Ratcheted,
                    BodyGuardKind::Unenforced => continue,
                };
                let mut tags = BTreeSet::new();
                collect_fn_inv_tags(src, lines, line_starts, item_fn, &mut tags);
                for id in tags {
                    guards
                        .entry(id)
                        .and_modify(|cur| *cur = (*cur).min(strength))
                        .or_insert(strength);
                }
            }
            _ => {}
        }
    }
}

/// Inspects a parsed `#[test]` `syn::Block` directly:
/// - Skips unreachable/unexecuted scopes (nested `Item::Fn`/items, `Expr::Closure`, disqualifying
///   `#[cfg(...)]` attributes like `#[cfg(any())]`, and dead `if false` / `while false` branches).
/// - Rejects trivial literal tautologies (`assert!(true)`, `assert_eq!(1, 1)`,
///   `assert_no_violations(_, &[])`, `assert_staged_ratchet(..., &[])`, etc.).
/// - Rejects any test calling unforced `assert_staged_violations(...)`.
/// - Downgrades mixed strict + ratchet tests to `BodyGuardKind::Ratcheted` so only pure strict
///   tests with zero ratchets qualify as `GuardStrength::Enforced`.
fn classify_test_block(block: &syn::Block) -> BodyGuardKind {
    struct AssertScan {
        has_strict: bool,
        has_ratchet: bool,
        has_unforced_staged: bool,
    }

    impl<'ast> syn::visit::Visit<'ast> for AssertScan {
        fn visit_item(&mut self, _node: &'ast syn::Item) {
            // Nested fn/mod/impl/trait declarations inside a test body do not execute on their own.
        }

        fn visit_expr_closure(&mut self, _node: &'ast syn::ExprClosure) {
            // Uncalled closures (e.g. `let _ = || assert!(false);`) must not spoof assertions.
        }

        fn visit_arm(&mut self, node: &'ast syn::Arm) {
            if has_disqualifying_cfg(&node.attrs) {
                return;
            }
            syn::visit::visit_arm(self, node);
        }

        fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
            match node {
                syn::Stmt::Local(local) if has_disqualifying_cfg(&local.attrs) => return,
                syn::Stmt::Macro(mac) if has_disqualifying_cfg(&mac.attrs) => return,
                syn::Stmt::Item(_) => return,
                _ => {}
            }
            syn::visit::visit_stmt(self, node);
        }

        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if has_disqualifying_cfg(expr_attrs(node)) {
                return;
            }
            syn::visit::visit_expr(self, node);
        }

        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            match const_bool_expr(&node.cond) {
                Some(false) => {
                    if let Some((_, else_expr)) = &node.else_branch {
                        self.visit_expr(else_expr);
                    }
                }
                Some(true) => self.visit_block(&node.then_branch),
                None => syn::visit::visit_expr_if(self, node),
            }
        }

        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            if const_bool_expr(&node.cond) == Some(false) {
                return;
            }
            syn::visit::visit_expr_while(self, node);
        }

        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(ep) = strip_parens(&node.func)
                && let Some(seg) = ep.path.segments.last()
            {
                match seg.ident.to_string().as_str() {
                    "assert_no_violations" => {
                        if node
                            .args
                            .iter()
                            .nth(1)
                            .is_some_and(|arg| !is_trivially_empty_slice(arg))
                        {
                            self.has_strict = true;
                        }
                    }
                    "assert_staged_ratchet" => {
                        if node
                            .args
                            .iter()
                            .nth(3)
                            .is_some_and(|arg| !is_trivially_empty_slice(arg))
                        {
                            self.has_ratchet = true;
                        }
                    }
                    "assert_staged_violations" => self.has_unforced_staged = true,
                    _ => {}
                }
            }
            syn::visit::visit_expr_call(self, node);
        }

        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if let Some(seg) = node.path.segments.last() {
                match seg.ident.to_string().as_str() {
                    "assert" if is_non_tautological_assert(&node.tokens) => self.has_strict = true,
                    "assert_eq" if is_non_tautological_assert_eq(&node.tokens) => {
                        self.has_strict = true;
                    }
                    "assert_ne" if is_non_tautological_assert_ne(&node.tokens) => {
                        self.has_strict = true;
                    }
                    "panic" => self.has_strict = true,
                    _ => {}
                }
            }
            syn::visit::visit_macro(self, node);
        }
    }

    let mut scan = AssertScan {
        has_strict: false,
        has_ratchet: false,
        has_unforced_staged: false,
    };
    syn::visit::visit_block(&mut scan, block);
    if scan.has_unforced_staged {
        BodyGuardKind::Unenforced
    } else if scan.has_ratchet {
        // Mixed strict + ratchet tests still carry a non-zero staged ceiling; only pure strict
        // tests with zero ratchets qualify as `GuardStrength::Enforced`.
        BodyGuardKind::Ratcheted
    } else if scan.has_strict {
        BodyGuardKind::Enforced
    } else {
        BodyGuardKind::Unenforced
    }
}

fn is_active_test_fn(item_fn: &syn::ItemFn) -> bool {
    let mut has_test = false;
    for attr in &item_fn.attrs {
        if is_disqualifying_test_attr(attr) {
            return false;
        }
        if matches!(attr.style, syn::AttrStyle::Outer)
            && path_is_any(
                attr.path(),
                &[
                    &["test"],
                    &["core", "prelude", "v1", "test"],
                    &["std", "prelude", "v1", "test"],
                ],
            )
            && matches!(attr.meta, syn::Meta::Path(_))
        {
            has_test = true;
        }
    }
    has_test
}

/// Recursively inspects `meta` through any nested `cfg_attr(pred, inner...)` wrappers, returning
/// `true` if `check` matches `meta` (or any nested inner meta), or if `cfg_attr` fails to parse.
fn meta_matches_through_cfg_attr(meta: &syn::Meta, check: &dyn Fn(&syn::Meta) -> bool) -> bool {
    if check(meta) {
        return true;
    }
    if meta.path().is_ident("cfg_attr") {
        let syn::Meta::List(list) = meta else {
            return true;
        };
        let Ok(metas) = list.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        ) else {
            return true;
        };
        return metas
            .iter()
            .skip(1)
            .any(|inner| meta_matches_through_cfg_attr(inner, check));
    }
    false
}

fn is_disqualifying_cfg_meta(meta: &syn::Meta) -> bool {
    if !meta.path().is_ident("cfg") {
        return false;
    }
    let syn::Meta::List(list) = meta else {
        return true;
    };
    !list
        .parse_args::<syn::Meta>()
        .is_ok_and(|inner| is_pure_cfg_test(&inner))
}

fn is_disqualifying_test_meta(meta: &syn::Meta) -> bool {
    if is_disqualifying_cfg_meta(meta) {
        return true;
    }
    let is_target = |id: &syn::Ident| id == "ignore" || id == "should_panic";
    let mut segs = meta.path().segments.iter().map(|s| &s.ident);
    match (segs.next(), segs.next(), segs.next(), segs.next(), segs.next()) {
        (Some(a), None, _, _, _) => is_target(a),
        (Some(root), Some(prelude), Some(edition), Some(leaf), None) => {
            (root == "core" || root == "std")
                && prelude == "prelude"
                && (edition == "v1"
                    || edition == "rust_2015"
                    || edition == "rust_2018"
                    || edition == "rust_2021"
                    || edition == "rust_2024")
                && is_target(leaf)
        }
        _ => false,
    }
}

fn is_disqualifying_test_attr(attr: &syn::Attribute) -> bool {
    meta_matches_through_cfg_attr(&attr.meta, &is_disqualifying_test_meta)
}

/// Returns `true` if any `#[cfg(...)]` (including inside recursive `#[cfg_attr(..., ...)]`) is
/// anything other than pure `#[cfg(test)]` (rejecting `#[cfg(any())]`, `#[cfg(not(test))]`,
/// `#[cfg(feature = "...")]`, platform cfgs, and `#[cfg(all(test, ...))]` with extra conditions).
fn has_disqualifying_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| meta_matches_through_cfg_attr(&attr.meta, &is_disqualifying_cfg_meta))
}

fn is_pure_cfg_test(meta: &syn::Meta) -> bool {
    match meta {
        syn::Meta::Path(p) => p.is_ident("test"),
        syn::Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| !items.is_empty() && items.iter().all(is_pure_cfg_test)),
        _ => false,
    }
}

fn path_is_any(path: &syn::Path, allowed: &[&[&str]]) -> bool {
    allowed.iter().any(|expected| {
        path.segments.len() == expected.len()
            && path
                .segments
                .iter()
                .zip(expected.iter())
                .all(|(seg, &name)| seg.ident == name)
    })
}

fn collect_fn_inv_tags(
    src: &str,
    lines: &[&str],
    line_starts: &[usize],
    item_fn: &syn::ItemFn,
    tags: &mut BTreeSet<String>,
) {
    for attr in &item_fn.attrs {
        if attr.path().is_ident("doc")
            && let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(el) = &nv.value
            && let syn::Lit::Str(lit) = &el.lit
        {
            collect_valid_inv_tags(&lit.value(), tags);
        }
    }
    let first_line = item_fn
        .attrs
        .first()
        .map(|a| a.span().start().line)
        .unwrap_or_else(|| item_fn.sig.span().start().line);
    let mut idx = first_line.saturating_sub(1);
    while idx > 0 {
        let prev = lines[idx - 1].trim();
        if prev.starts_with("//") {
            collect_valid_inv_tags(prev, tags);
            idx -= 1;
        } else {
            break;
        }
    }
    let body_start_line = item_fn.block.span().start().line;
    let hdr_start = first_line.saturating_sub(1);
    let hdr_end = body_start_line.min(lines.len());
    for &line in lines.get(hdr_start..hdr_end).unwrap_or(&[]) {
        if line.trim_start().starts_with("//") {
            collect_valid_inv_tags(line, tags);
        }
    }
    collect_valid_inv_tags(span_to_slice(src, line_starts, item_fn.block.span()), tags);
}

fn span_to_slice<'a>(
    src: &'a str,
    line_starts: &[usize],
    span: proc_macro2::Span,
) -> &'a str {
    let start = line_col_to_byte(src, line_starts, span.start());
    let end = line_col_to_byte(src, line_starts, span.end());
    if start <= end && end <= src.len() {
        &src[start..end]
    } else {
        ""
    }
}

fn line_col_to_byte(src: &str, line_starts: &[usize], lc: proc_macro2::LineColumn) -> usize {
    let Some(&line_start) = line_starts.get(lc.line.saturating_sub(1)) else {
        return src.len();
    };
    match src[line_start..].char_indices().nth(lc.column) {
        Some((off, _)) => line_start + off,
        None => src.len(),
    }
}

fn strip_parens(mut expr: &syn::Expr) -> &syn::Expr {
    loop {
        match expr {
            syn::Expr::Paren(p) => expr = &p.expr,
            syn::Expr::Group(g) => expr = &g.expr,
            _ => return expr,
        }
    }
}

fn const_bool_expr(expr: &syn::Expr) -> Option<bool> {
    match strip_parens(expr) {
        syn::Expr::Lit(el) => match &el.lit {
            syn::Lit::Bool(b) => Some(b.value),
            _ => None,
        },
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => {
            const_bool_expr(&u.expr).map(|b| !b)
        }
        syn::Expr::Binary(b) => match b.op {
            syn::BinOp::And(_) => match (const_bool_expr(&b.left), const_bool_expr(&b.right)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            syn::BinOp::Or(_) => match (const_bool_expr(&b.left), const_bool_expr(&b.right)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            syn::BinOp::Eq(_) => compare_const_exprs(&b.left, &b.right),
            syn::BinOp::Ne(_) => compare_const_exprs(&b.left, &b.right).map(|eq| !eq),
            _ => None,
        },
        _ => None,
    }
}

fn compare_const_exprs(lhs: &syn::Expr, rhs: &syn::Expr) -> Option<bool> {
    let (l, r) = (strip_parens(lhs), strip_parens(rhs));
    if let (syn::Expr::Lit(la), syn::Expr::Lit(lb)) = (l, r) {
        return Some(lit_eq(&la.lit, &lb.lit));
    }
    if let (Some(a), Some(b)) = (const_bool_expr(l), const_bool_expr(r)) {
        return Some(a == b);
    }
    None
}

fn suffix_compatible(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || a == b
}

fn lit_eq(a: &syn::Lit, b: &syn::Lit) -> bool {
    match (a, b) {
        (syn::Lit::Bool(x), syn::Lit::Bool(y)) => x.value == y.value,
        (syn::Lit::Int(x), syn::Lit::Int(y)) => {
            suffix_compatible(x.suffix(), y.suffix())
                && match (x.base10_parse::<u128>(), y.base10_parse::<u128>()) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => x.base10_digits() == y.base10_digits(),
                }
        }
        (syn::Lit::Float(x), syn::Lit::Float(y)) => {
            suffix_compatible(x.suffix(), y.suffix())
                && match (x.base10_parse::<f64>(), y.base10_parse::<f64>()) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => x.base10_digits() == y.base10_digits(),
                }
        }
        (syn::Lit::Int(i), syn::Lit::Float(f)) | (syn::Lit::Float(f), syn::Lit::Int(i)) => {
            matches!(i.suffix(), "f32" | "f64")
                && suffix_compatible(i.suffix(), f.suffix())
                && matches!(
                    (i.base10_digits().parse::<f64>(), f.base10_parse::<f64>()),
                    (Ok(a), Ok(b)) if a == b
                )
        }
        (syn::Lit::Str(x), syn::Lit::Str(y)) => x.value() == y.value(),
        (syn::Lit::ByteStr(x), syn::Lit::ByteStr(y)) => x.value() == y.value(),
        (syn::Lit::Char(x), syn::Lit::Char(y)) => x.value() == y.value(),
        (syn::Lit::Byte(x), syn::Lit::Byte(y)) => x.value() == y.value(),
        _ => false,
    }
}

fn same_simple_path(a: &syn::Expr, b: &syn::Expr) -> bool {
    let (syn::Expr::Path(pa), syn::Expr::Path(pb)) = (strip_parens(a), strip_parens(b)) else {
        return false;
    };
    pa.qself.is_none()
        && pb.qself.is_none()
        && pa.path.leading_colon.is_some() == pb.path.leading_colon.is_some()
        && pa.path.segments.len() == pb.path.segments.len()
        && pa.path.segments.iter().zip(pb.path.segments.iter()).all(|(sa, sb)| {
            sa.ident == sb.ident && sa.arguments.is_none() && sb.arguments.is_none()
        })
}

fn is_trivially_empty_slice(expr: &syn::Expr) -> bool {
    match strip_parens(expr) {
        syn::Expr::Reference(r) => is_trivially_empty_slice(&r.expr),
        syn::Expr::Cast(c) => is_trivially_empty_slice(&c.expr),
        syn::Expr::Array(a) => a.elems.is_empty(),
        syn::Expr::Index(idx) => is_trivially_empty_slice(&idx.expr),
        syn::Expr::MethodCall(mc) => {
            matches!(mc.method.to_string().as_str(), "as_slice" | "as_ref")
                && is_trivially_empty_slice(&mc.receiver)
        }
        syn::Expr::Call(c) => {
            c.args.is_empty()
                && matches!(
                    strip_parens(&c.func),
                    syn::Expr::Path(ep)
                        if path_is_any(&ep.path, &[&["Vec", "new"], &["std", "vec", "Vec", "new"], &["alloc", "vec", "Vec", "new"]])
                )
        }
        syn::Expr::Macro(m) => m.mac.path.is_ident("vec") && m.mac.tokens.is_empty(),
        syn::Expr::Block(b) => {
            b.block.stmts.len() == 1
                && matches!(&b.block.stmts[0], syn::Stmt::Expr(e, None) if is_trivially_empty_slice(e))
        }
        _ => false,
    }
}

fn parse_macro_args(
    tokens: &proc_macro2::TokenStream,
) -> Option<syn::punctuated::Punctuated<syn::Expr, syn::Token![,]>> {
    syn::parse::Parser::parse2(
        syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        tokens.clone(),
    )
    .ok()
}

fn is_non_tautological_assert(tokens: &proc_macro2::TokenStream) -> bool {
    let Some(args) = parse_macro_args(tokens) else {
        return false;
    };
    let Some(cond) = args.first() else {
        return false;
    };
    const_bool_expr(cond) != Some(true)
}

fn is_non_tautological_assert_eq(tokens: &proc_macro2::TokenStream) -> bool {
    let Some(args) = parse_macro_args(tokens) else {
        return false;
    };
    if args.len() < 2 {
        return false;
    }
    let (lhs, rhs) = (&args[0], &args[1]);
    !same_simple_path(lhs, rhs) && compare_const_exprs(lhs, rhs) != Some(true)
}

fn is_non_tautological_assert_ne(tokens: &proc_macro2::TokenStream) -> bool {
    let Some(args) = parse_macro_args(tokens) else {
        return false;
    };
    if args.len() < 2 {
        return false;
    }
    compare_const_exprs(&args[0], &args[1]) != Some(false)
}

fn expr_attrs(expr: &syn::Expr) -> &[syn::Attribute] {
    match expr {
        syn::Expr::Array(e) => &e.attrs,
        syn::Expr::Assign(e) => &e.attrs,
        syn::Expr::Async(e) => &e.attrs,
        syn::Expr::Await(e) => &e.attrs,
        syn::Expr::Binary(e) => &e.attrs,
        syn::Expr::Block(e) => &e.attrs,
        syn::Expr::Break(e) => &e.attrs,
        syn::Expr::Call(e) => &e.attrs,
        syn::Expr::Cast(e) => &e.attrs,
        syn::Expr::Closure(e) => &e.attrs,
        syn::Expr::Const(e) => &e.attrs,
        syn::Expr::Continue(e) => &e.attrs,
        syn::Expr::Field(e) => &e.attrs,
        syn::Expr::ForLoop(e) => &e.attrs,
        syn::Expr::Group(e) => &e.attrs,
        syn::Expr::If(e) => &e.attrs,
        syn::Expr::Index(e) => &e.attrs,
        syn::Expr::Infer(e) => &e.attrs,
        syn::Expr::Let(e) => &e.attrs,
        syn::Expr::Lit(e) => &e.attrs,
        syn::Expr::Loop(e) => &e.attrs,
        syn::Expr::Macro(e) => &e.attrs,
        syn::Expr::Match(e) => &e.attrs,
        syn::Expr::MethodCall(e) => &e.attrs,
        syn::Expr::Paren(e) => &e.attrs,
        syn::Expr::Path(e) => &e.attrs,
        syn::Expr::Range(e) => &e.attrs,
        syn::Expr::RawAddr(e) => &e.attrs,
        syn::Expr::Reference(e) => &e.attrs,
        syn::Expr::Repeat(e) => &e.attrs,
        syn::Expr::Return(e) => &e.attrs,
        syn::Expr::Struct(e) => &e.attrs,
        syn::Expr::Try(e) => &e.attrs,
        syn::Expr::TryBlock(e) => &e.attrs,
        syn::Expr::Tuple(e) => &e.attrs,
        syn::Expr::Unary(e) => &e.attrs,
        syn::Expr::Unsafe(e) => &e.attrs,
        syn::Expr::While(e) => &e.attrs,
        syn::Expr::Yield(e) => &e.attrs,
        _ => &[],
    }
}
