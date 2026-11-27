//! Typed handle hygiene & bare-handle-parameter AST scanners (`[INV:typed-domain-handles]`).
//!
//! - Enforces private tuple fields (`Visibility::Inherited`) on `NodeId` and `SurfaceId` in
//!   `packages/core/abi/src/id.rs` and verifies both struct definitions are present.
//! - Bans leftover `StoreNodeId` aliases (`use ... as StoreNodeId`, `use ...::StoreNodeId`,
//!   `type StoreNodeId = ...`) anywhere in the AST, including inside function bodies.
//! - Bans raw `NodeId(...)`/`SurfaceId(...)` tuple-call and `NodeId { 0: ... }` struct-literal
//!   construction outside `id.rs` while ignoring enum variants like `MyEnum::NodeId(...)`.
//! - Bans bare primitive `surface_id`/`node_id`/`frame_id`/`interface_id` parameters (including
//!   wrapped `Option`/`&`/`&mut`/`[]`/`Vec`/`*const`/`*mut`, `extern "C"`, `fn(...)`, and nested
//!   functions) across all `packages/core/*` crates (skipping `#[wasm_bindgen]` JS boundaries).

use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Fields, ForeignItem, Item, Type, Visibility};

use crate::{is_cfg_test_attr, line_at, Violation};

const BARE_PRIMITIVES: &[&str] = &[
    "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize", "f32", "f64",
];

const BANNED_HANDLE_PARAMS: &[(&str, &str)] = &[
    ("surface_id", "u32"),
    ("node_id", "u32"),
    ("frame_id", "u64"),
    ("interface_id", "u32"),
];

const ABI_ID_PATH: &str = "packages/core/abi/src/id.rs";

/// Enforces typed handle hygiene across Rust sources (`[INV:typed-domain-handles]`).
pub fn scan_handle_hygiene(rel: &str, raw: &str, ast: &syn::File, out: &mut Vec<Violation>) {
    let is_id_rs = rel == ABI_ID_PATH;
    let mut visitor = HandleHygieneVisitor {
        rel,
        raw,
        is_id_rs,
        saw_node_id: false,
        saw_surface_id: false,
        out,
    };
    visitor.visit_file(ast);

    if is_id_rs {
        if !visitor.saw_node_id {
            visitor.out.push(Violation::new(
                rel,
                1,
                "missing `pub struct NodeId(...)`",
                "`packages/core/abi/src/id.rs` must directly declare `struct NodeId` so field privacy is auditable",
            ));
        }
        if !visitor.saw_surface_id {
            visitor.out.push(Violation::new(
                rel,
                1,
                "missing `pub struct SurfaceId(...)`",
                "`packages/core/abi/src/id.rs` must directly declare `struct SurfaceId` so field privacy is auditable",
            ));
        }
    }
}

struct HandleHygieneVisitor<'a> {
    rel: &'a str,
    raw: &'a str,
    is_id_rs: bool,
    saw_node_id: bool,
    saw_surface_id: bool,
    out: &'a mut Vec<Violation>,
}

impl HandleHygieneVisitor<'_> {
    fn check_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.check_use_tree(&p.tree),
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    self.check_use_tree(item);
                }
            }
            syn::UseTree::Rename(r) if r.rename == "StoreNodeId" => {
                let ln = r.rename.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Leftover `StoreNodeId` alias is banned; import unified `NodeId` directly",
                ));
            }
            syn::UseTree::Name(n) if n.ident == "StoreNodeId" => {
                let ln = n.ident.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    "Leftover `StoreNodeId` import is banned; import unified `NodeId` directly",
                ));
            }
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for HandleHygieneVisitor<'_> {
    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        self.check_use_tree(&node.tree);
        visit::visit_item_use(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        if node.ident == "StoreNodeId" {
            let ln = node.ident.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                ln,
                line_at(self.raw, ln),
                "Leftover `type StoreNodeId` alias is banned; use unified `NodeId` directly",
            ));
        }
        visit::visit_item_type(self, node);
    }

    fn visit_item_struct(&mut self, st: &'ast syn::ItemStruct) {
        if self.is_id_rs && !is_cfg_test_attr(&st.attrs) {
            let name = st.ident.to_string();
            if name == "NodeId" {
                self.saw_node_id = true;
            } else if name == "SurfaceId" {
                self.saw_surface_id = true;
            }
            if matches!(name.as_str(), "NodeId" | "SurfaceId")
                && let Fields::Unnamed(unnamed) = &st.fields
                && unnamed
                    .unnamed
                    .iter()
                    .any(|f| !matches!(f.vis, Visibility::Inherited))
            {
                let ln = st.ident.span().start().line;
                self.out.push(Violation::new(
                    self.rel,
                    ln,
                    line_at(self.raw, ln),
                    format!(
                        "`{name}` tuple field must remain private (`Visibility::Inherited`) so handles can only be minted via validated constructors"
                    ),
                ));
            }
        }
        visit::visit_item_struct(self, st);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if !self.is_id_rs
            && let Expr::Path(ep) = &*call.func
            && let Some(ident) = direct_handle_ctor_ident(&ep.path)
        {
            let ln = ident.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                ln,
                line_at(self.raw, ln),
                format!(
                    "Raw `{ident}(...)` tuple construction is banned outside `{ABI_ID_PATH}`; use `::ROOT`, `::NULL`, `::new`, `::try_new`, or `::from_raw`"
                ),
            ));
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, es: &'ast syn::ExprStruct) {
        if !self.is_id_rs
            && let Some(ident) = direct_handle_ctor_ident(&es.path)
        {
            let ln = ident.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                ln,
                line_at(self.raw, ln),
                format!(
                    "Raw `{ident} {{ ... }}` struct construction is banned outside `{ABI_ID_PATH}`; use `::ROOT`, `::NULL`, `::new`, `::try_new`, or `::from_raw`"
                ),
            ));
        }
        visit::visit_expr_struct(self, es);
    }
}

/// Returns `Some(last_ident)` when `path` directly names the `NodeId` or `SurfaceId` struct
/// (e.g. `NodeId`, `nve_abi::NodeId`, `crate::id::SurfaceId`), while ignoring enum variants such
/// as `MyEnum::NodeId(...)` whose preceding path segment starts with an uppercase type name.
fn direct_handle_ctor_ident(path: &syn::Path) -> Option<&syn::Ident> {
    let last = path.segments.last()?;
    if !matches!(last.ident.to_string().as_str(), "NodeId" | "SurfaceId") {
        return None;
    }
    if path.segments.len() >= 2 {
        let prev = &path.segments[path.segments.len() - 2].ident.to_string();
        if prev.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return None;
        }
    }
    Some(&last.ident)
}

/// Scans core Rust API function signatures (skipping `#[wasm_bindgen]` JS boundaries) for bare
/// or wrapped primitive `surface_id`/`node_id`/`frame_id`/`interface_id` parameters across
/// top-level/nested fns, impl methods, trait methods, `extern` blocks, and bare `fn(...)` types.
pub fn scan_bare_handle_params(
    rel: &str,
    raw: &str,
    ast: &syn::File,
    out: &mut Vec<Violation>,
) {
    if is_cfg_test_attr(&ast.attrs) {
        return;
    }
    BareHandleParamVisitor { rel, raw, out }.visit_file(ast);
}

fn has_wasm_bindgen_attr(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "wasm_bindgen"))
}

/// Recursively unwraps `Option<T>`, `Vec<T>`, `&T`, `&mut T`, `[T]`, `[T; N]`, `*const T`,
/// `*mut T`, parentheses, and groups to check whether the inner type is a bare primitive.
fn unwrap_bare_primitive(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::Reference(r) => unwrap_bare_primitive(&r.elem),
        Type::Ptr(p) => unwrap_bare_primitive(&p.elem),
        Type::Slice(s) => unwrap_bare_primitive(&s.elem),
        Type::Array(a) => unwrap_bare_primitive(&a.elem),
        Type::Paren(p) => unwrap_bare_primitive(&p.elem),
        Type::Group(g) => unwrap_bare_primitive(&g.elem),
        Type::Path(tp) if tp.qself.is_none() => {
            if tp.path.segments.len() == 1 {
                let s = tp.path.segments[0].ident.to_string();
                if let Some(&prim) = BARE_PRIMITIVES.iter().find(|&&p| p == s) {
                    return Some(prim);
                }
            }
            let last = tp.path.segments.last()?;
            if matches!(last.ident.to_string().as_str(), "Option" | "Vec")
                && let syn::PathArguments::AngleBracketed(args) = &last.arguments
                && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
            {
                return unwrap_bare_primitive(inner);
            }
            None
        }
        _ => None,
    }
}

struct BareHandleParamVisitor<'a> {
    rel: &'a str,
    raw: &'a str,
    out: &'a mut Vec<Violation>,
}

impl BareHandleParamVisitor<'_> {
    fn check_param(&mut self, fn_label: &str, ident: &syn::Ident, ty: &Type) {
        let pname = ident.to_string();
        if let Some((_, banned_ty)) = BANNED_HANDLE_PARAMS.iter().find(|&&(n, _)| n == pname)
            && unwrap_bare_primitive(ty) == Some(*banned_ty)
        {
            let ln = ident.span().start().line;
            self.out.push(Violation::new(
                self.rel,
                ln,
                line_at(self.raw, ln),
                format!(
                    "{fn_label} parameter `{pname}` wraps bare `{banned_ty}` instead of the typed `nve_abi` handle newtype"
                ),
            ));
        }
    }

    fn check_sig(&mut self, sig: &syn::Signature) {
        let label = format!("fn `{}`", sig.ident);
        for input in &sig.inputs {
            if let syn::FnArg::Typed(pat_ty) = input
                && let syn::Pat::Ident(pat_ident) = &*pat_ty.pat
            {
                self.check_param(&label, &pat_ident.ident, &pat_ty.ty);
            }
        }
    }
}

impl<'ast> Visit<'ast> for BareHandleParamVisitor<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        match item {
            Item::Mod(m) if is_cfg_test_attr(&m.attrs) => return,
            Item::Impl(imp)
                if is_cfg_test_attr(&imp.attrs) || has_wasm_bindgen_attr(&imp.attrs) =>
            {
                return;
            }
            Item::Trait(tr) if is_cfg_test_attr(&tr.attrs) => return,
            Item::Fn(f) if is_cfg_test_attr(&f.attrs) || has_wasm_bindgen_attr(&f.attrs) => return,
            Item::ForeignMod(fm) if is_cfg_test_attr(&fm.attrs) => return,
            Item::Struct(s) if is_cfg_test_attr(&s.attrs) => return,
            Item::Enum(e) if is_cfg_test_attr(&e.attrs) => return,
            Item::Type(t) if is_cfg_test_attr(&t.attrs) => return,
            Item::Const(c) if is_cfg_test_attr(&c.attrs) => return,
            Item::Static(s) if is_cfg_test_attr(&s.attrs) => return,
            _ => {}
        }
        visit::visit_item(self, item);
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if is_cfg_test_attr(&f.attrs) || has_wasm_bindgen_attr(&f.attrs) {
            return;
        }
        self.check_sig(&f.sig);
        visit::visit_item_fn(self, f);
    }

    fn visit_impl_item_fn(&mut self, m: &'ast syn::ImplItemFn) {
        if is_cfg_test_attr(&m.attrs) || has_wasm_bindgen_attr(&m.attrs) {
            return;
        }
        self.check_sig(&m.sig);
        visit::visit_impl_item_fn(self, m);
    }

    fn visit_trait_item_fn(&mut self, m: &'ast syn::TraitItemFn) {
        if is_cfg_test_attr(&m.attrs) || has_wasm_bindgen_attr(&m.attrs) {
            return;
        }
        self.check_sig(&m.sig);
        visit::visit_trait_item_fn(self, m);
    }

    fn visit_foreign_item_fn(&mut self, f: &'ast syn::ForeignItemFn) {
        if is_cfg_test_attr(&f.attrs) || has_wasm_bindgen_attr(&f.attrs) {
            return;
        }
        self.check_sig(&f.sig);
        visit::visit_foreign_item_fn(self, f);
    }

    fn visit_item_foreign_mod(&mut self, fm: &'ast syn::ItemForeignMod) {
        if is_cfg_test_attr(&fm.attrs) {
            return;
        }
        for fi in &fm.items {
            if let ForeignItem::Fn(f) = fi {
                self.visit_foreign_item_fn(f);
            }
        }
    }

    fn visit_type_bare_fn(&mut self, bare: &'ast syn::TypeBareFn) {
        for arg in &bare.inputs {
            if let Some((ident, _)) = &arg.name {
                self.check_param("bare `fn`", ident, &arg.ty);
            }
        }
        visit::visit_type_bare_fn(self, bare);
    }
}
