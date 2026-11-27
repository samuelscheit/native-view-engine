//! Active hot-path allocation/UTF-8 guards and repo-wide FNV-1a ban (code + text/docs/scripts).

use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ExprCall, ExprMacro, ExprMethodCall, ImplItemFn, ItemFn, ItemMod, Macro};

use crate::{
    is_binary_ext, is_cfg_test_attr, is_self_invariant_file, is_test_or_bench_rs, line_at,
    RepoIndex, Violation, DOC_EXCLUDES,
};

/// Real existing hot-path production files always scanned today (plus `packages/core/store/src`).
/// Every listed file MUST exist so this scanner can never silently no-op.
pub const EXISTING_HOT_PATH_FILES: &[&str] = &[
    "packages/core/engine/src/layout.rs",
    "packages/core/wire-core/src/frame.rs",
    "packages/core/store/src/store.rs",
    "packages/core/store/src/tx.rs",
    "packages/core/store/src/span_pool.rs",
    "packages/backends/headless/src/taffy_store/mod.rs",
];

fn is_allowlisted_cold_fn(rel: &str, name: &str) -> bool {
    ((rel == "packages/core/store/src/store.rs"
        || rel == "packages/backends/headless/src/taffy_store/mod.rs")
        && name == "new")
        || (rel == "packages/core/store/src/span_pool.rs" && name == "try_str")
        || (rel == "packages/core/wire-core/src/frame.rs" && name == "to_owned")
}

/// Scans all `EXISTING_HOT_PATH_FILES` plus hot modules in `packages/core/store/src/` for banned
/// hot-path UTF-8 re-validation (`from_utf8(`) and heap allocations/sorting.
pub fn check_hot_path_perf_and_utf8(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for &candidate in EXISTING_HOT_PATH_FILES {
        let parsed = idx.parse_rust(candidate).unwrap_or_else(|| {
            #[expect(clippy::panic, reason = "hot-path inventory guard must fail loudly if file moved")]
            {
                panic!(
                    "Expected hot-path file `{candidate}` to exist — update EXISTING_HOT_PATH_FILES if moved"
                );
            }
        });
        scanned += 1;
        scan_hot_path_ast(candidate, &parsed.raw, &parsed.ast, true, &mut violations);
    }

    for rel in idx.rs_files_under("packages/core/store/src") {
        if is_test_or_bench_rs(&rel) || EXISTING_HOT_PATH_FILES.contains(&rel.as_str()) {
            continue;
        }
        let Some(parsed) = idx.parse_rust(&rel) else {
            continue;
        };
        scanned += 1;
        let is_hot = rel.contains("row_writer")
            || rel.contains("transaction")
            || rel.contains("str_arena")
            || rel.contains("string_arena")
            || rel.contains("span_pool")
            || rel.contains("dirty");
        scan_hot_path_ast(&rel, &parsed.raw, &parsed.ast, is_hot, &mut violations);
    }

    assert!(
        scanned >= EXISTING_HOT_PATH_FILES.len(),
        "hot_path_perf_and_utf8_guards must scan real files"
    );
    violations
}

/// AST-based hot-path scanner skipping `#![cfg(test)]`, `#[cfg(test)]`, and cold helper functions
/// (`fn new`, `fn try_str`, `fn to_owned`).
pub fn scan_hot_path_ast(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    is_hot_file: bool,
    out: &mut Vec<Violation>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    let check_utf8 = (rel.starts_with("packages/core/store/src/")
        && !rel.contains("wire")
        && !rel.contains("ingress"))
        || is_hot_file;
    if !check_utf8 && !is_hot_file {
        return;
    }

    struct HotScan<'a> {
        rel: &'a str,
        raw: &'a str,
        check_utf8: bool,
        is_hot_file: bool,
        out: &'a mut Vec<Violation>,
    }

    impl HotScan<'_> {
        fn push(&mut self, line: usize, reason: impl Into<String>) {
            self.out
                .push(Violation::new(self.rel, line, line_at(self.raw, line), reason));
        }

        fn check_macro(&mut self, mac: &Macro) {
            let Some(last) = mac.path.segments.last() else {
                return;
            };
            let name = last.ident.to_string();
            if name.starts_with("debug_assert") {
                return;
            }
            if self.is_hot_file {
                let ln = last.ident.span().start().line;
                match name.as_str() {
                    "format" => self.push(ln, "Hot-path ban `format!(`: allocates String on hot path"),
                    "vec" => self.push(
                        ln,
                        "Hot-path ban `vec![`: heap allocation on hot path; reuse arena/scratch buffer",
                    ),
                    _ => {}
                }
            }
            if let Ok(args) = mac.parse_body_with(
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
            ) {
                for expr in &args {
                    self.visit_expr(expr);
                }
            }
        }
    }

    impl<'ast> Visit<'ast> for HotScan<'_> {
        fn visit_item_mod(&mut self, node: &'ast ItemMod) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_mod(self, node);
            }
        }

        fn visit_item_fn(&mut self, node: &'ast ItemFn) {
            if is_cfg_test_attr(&node.attrs)
                || is_allowlisted_cold_fn(self.rel, &node.sig.ident.to_string())
            {
                return;
            }
            visit::visit_item_fn(self, node);
        }

        fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
            if is_cfg_test_attr(&node.attrs)
                || is_allowlisted_cold_fn(self.rel, &node.sig.ident.to_string())
            {
                return;
            }
            visit::visit_impl_item_fn(self, node);
        }

        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_impl(self, node);
            }
        }

        fn visit_expr_macro(&mut self, node: &'ast ExprMacro) {
            self.check_macro(&node.mac);
        }

        fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
            self.check_macro(&node.mac);
        }

        fn visit_expr_call(&mut self, node: &'ast ExprCall) {
            if let syn::Expr::Path(ep) = &*node.func {
                let segs: Vec<String> = ep.path.segments.iter().map(|s| s.ident.to_string()).collect();
                let last = segs.last().map(String::as_str);
                let prev = segs.len().checked_sub(2).map(|i| segs[i].as_str());
                let ln = ep.span().start().line;
                if self.check_utf8 && last == Some("from_utf8") {
                    self.push(
                        ln,
                        "`from_utf8` is banned in hot read/commit/layout paths; seal UTF-8 at ingress and use `from_utf8_unchecked` on reads",
                    );
                }
                if self.is_hot_file && prev == Some("String") && last == Some("new") {
                    self.push(
                        ln,
                        "Hot-path ban `String::new()`: heap String allocation on hot path",
                    );
                }
            }
            visit::visit_expr_call(self, node);
        }

        fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
            if self.is_hot_file {
                let name = node.method.to_string();
                let ln = node.method.span().start().line;
                match name.as_str() {
                    "to_ascii_lowercase" => self.push(
                        ln,
                        "Hot-path ban `.to_ascii_lowercase()`: allocates String on hot path; use `eq_ignore_ascii_case`",
                    ),
                    "to_vec" => self.push(ln, "Hot-path ban `.to_vec()`: heap clone on hot path"),
                    "to_string" => {
                        self.push(ln, "Hot-path ban `.to_string()`: heap String allocation on hot path")
                    }
                    "sort_unstable" => self.push(
                        ln,
                        "Hot-path ban `.sort_unstable(`: sorting dirty list on hot path; use bitset/generation stamp",
                    ),
                    "dedup" => self.push(
                        ln,
                        "Hot-path ban `.dedup(`: deduping dirty list on hot path; use bitset/generation stamp",
                    ),
                    _ => {}
                }
            }
            visit::visit_expr_method_call(self, node);
        }
    }

    HotScan {
        rel,
        raw,
        check_utf8,
        is_hot_file,
        out,
    }
    .visit_file(ast);
}

/// Scans every git-tracked non-binary text file (code, `.md`/`.mdx` docs, scripts, configs) for
/// FNV-1a identifiers (`fnv1a`, `fnv-1a`, `FnvHashMap`, `FnvHasher`) and offset/prime constants.
pub fn check_zero_fnv1a_repo_wide(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;

    for rel in &idx.tracked {
        if is_binary_ext(rel)
            || rel == "Cargo.lock"
            || rel == "AGENTS.md"
            || DOC_EXCLUDES.contains(&rel.as_str())
            || is_self_invariant_file(rel)
        {
            continue;
        }
        let Some(content) = idx.read_text(rel) else {
            continue;
        };
        scanned += 1;
        for (line_idx, line) in content.lines().enumerate() {
            if line_has_fnv1a(line) {
                violations.push(Violation::new(
                    rel,
                    line_idx + 1,
                    line,
                    "FNV-1a is banned; standardize on `const fn xxh64` (`nve-hash` / `nve-abi`)",
                ));
            }
        }
    }
    assert!(scanned > 0, "zero_fnv1a_repo_wide must scan > 0 files");
    violations
}

pub fn line_has_fnv1a(line: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "fnv1a",
        "fnv-1a",
        "fnvhashmap",
        "fnvhasher",
        "cbf29ce484222325",
        "00000100000001b3",
        "0x100000001b3",
        "811c9dc5",
        "01000193",
        "14695981039346656037",
        "1099511628211",
        "2166136261",
        "16777619",
    ];
    let compact: Vec<u8> = line
        .bytes()
        .filter(|&b| b != b'_')
        .map(|b| b.to_ascii_lowercase())
        .collect();
    NEEDLES
        .iter()
        .any(|needle| compact.windows(needle.len()).any(|w| w == needle.as_bytes()))
}

fn is_per_node_downcast_scope(rel: &str) -> bool {
    rel.starts_with("packages/backends/")
        || rel.starts_with("packages/core/abi/")
        || rel.starts_with("packages/core/components/")
        || rel.starts_with("packages/core/tables/")
        || rel == "packages/core/engine/src/layout.rs"
        || matches!(
            rel,
            "packages/core/store/src/store.rs"
                | "packages/core/store/src/tx.rs"
                | "packages/core/store/src/table.rs"
                | "packages/core/store/src/commit.rs"
                | "packages/core/store/src/view.rs"
                | "packages/core/store/src/span_pool.rs"
        )
}

/// Scans production Rust files in per-node hot scopes (`backends/**`, `core/{abi,components,tables}`,
/// `engine/src/layout.rs`, and hot store files) for banned `Any::downcast_ref` / `.downcast_ref`.
pub fn check_zero_per_node_downcast_ref(idx: &RepoIndex) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for rel in idx.rs_files_under("packages") {
        if is_test_or_bench_rs(&rel)
            || is_self_invariant_file(&rel)
            || !is_per_node_downcast_scope(&rel)
        {
            continue;
        }
        let Some(parsed) = idx.parse_rust(&rel) else {
            continue;
        };
        scanned += 1;
        scan_downcast_ref_ast(&rel, &parsed.raw, &parsed.ast, &mut violations);
    }
    assert!(
        scanned > 0,
        "zero_per_node_downcast_ref must scan > 0 files in per-node scope"
    );
    violations
}

/// AST visitor catching `.downcast_ref(...)`, `.downcast_ref::<T>(...)`, and `Any::downcast_ref(...)`
/// while skipping `#![cfg(test)]` and `#[cfg(test)]` items.
pub fn scan_downcast_ref_ast(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    struct DowncastScan<'a> {
        rel: &'a str,
        raw: &'a str,
        out: &'a mut Vec<Violation>,
    }
    impl<'ast> Visit<'ast> for DowncastScan<'_> {
        fn visit_item_mod(&mut self, node: &'ast ItemMod) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_mod(self, node);
            }
        }
        fn visit_item_fn(&mut self, node: &'ast ItemFn) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_fn(self, node);
            }
        }
        fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_impl_item_fn(self, node);
            }
        }
        fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
            if !is_cfg_test_attr(&node.attrs) {
                visit::visit_item_impl(self, node);
            }
        }
        fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
            if node.method == "downcast_ref" {
                let ln = node.method.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Per-node `Any::downcast_ref` is banned; use typed tables/handles",
                ));
            }
            visit::visit_expr_method_call(self, node);
        }
        fn visit_expr_call(&mut self, node: &'ast ExprCall) {
            if let syn::Expr::Path(ep) = &*node.func
                && ep
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "downcast_ref")
            {
                let ln = ep.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Per-node `Any::downcast_ref` is banned; use typed tables/handles",
                ));
            }
            visit::visit_expr_call(self, node);
        }
    }
    DowncastScan { rel, raw, out }.visit_file(ast);
}
