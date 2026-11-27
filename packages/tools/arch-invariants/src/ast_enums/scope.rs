//! Lexical `use` & `type` alias scope and crate/module-aware enum qualifiers for `ast_enums`.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Item, Stmt, Type, UseTree};

use crate::is_cfg_test_attr;

pub(super) const EXTERNAL_SDK_ROOTS: &[&str] = &[
    "taffy",
    "skia_safe",
    "web_sys",
    "accesskit",
    "winit",
    "parley",
    "cosmic_text",
    "vk",
    "ash",
    "ash_vk",
    "windows",
    "objc2",
    "gtk4",
];

/// Canonical identity of an enum type across files and crates.
///
/// Preserves crate/module origin so same-name enums from distinct crates (e.g.
/// `nve_components::tables::FlexDirection` vs `nve_abi::FlexDirection`) remain distinct,
/// while locally defined types get `origin_crate = current_crate` and unresolved single-segment
/// types (e.g. from `use super::*;` or `prelude::*`) keep `origin_crate = None` so they still
/// match explicitly qualified workspace imports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct EnumQualifier {
    pub origin_crate: Option<String>,
    pub origin_mod: Option<String>,
    pub type_name: String,
}

impl EnumQualifier {
    pub fn local(type_name: impl Into<String>, current_crate: Option<&str>) -> Self {
        Self {
            origin_crate: current_crate.map(ToString::to_string),
            origin_mod: None,
            type_name: type_name.into(),
        }
    }

    pub fn unresolved(type_name: impl Into<String>) -> Self {
        Self {
            origin_crate: None,
            origin_mod: None,
            type_name: type_name.into(),
        }
    }

    pub fn is_external_sdk(&self) -> bool {
        self.origin_crate
            .as_deref()
            .is_some_and(|c| EXTERNAL_SDK_ROOTS.contains(&c))
    }

    /// Canonical string key used in normalized expressions and paths.
    pub fn canonical_key(&self) -> String {
        match &self.origin_crate {
            Some(c) => format!("{c}::{}", self.type_name),
            None => match &self.origin_mod {
                Some(m) => format!("{m}::{}", self.type_name),
                None => self.type_name.clone(),
            },
        }
    }

    /// Human-readable short display name for reports & allowlist keys.
    pub fn display_name(&self) -> String {
        if self.is_external_sdk()
            && let Some(c) = &self.origin_crate
        {
            return format!("{c}::{}", self.type_name);
        }
        self.type_name.clone()
    }

    /// True if two qualifiers can refer to the exact same enum type across two sites.
    pub fn compatible_with(&self, other: &Self) -> bool {
        if self.type_name != other.type_name {
            return false;
        }
        match (&self.origin_crate, &other.origin_crate) {
            (Some(a), Some(b)) => {
                if a != b {
                    return false;
                }
            }
            (Some(c), None) | (None, Some(c)) => {
                if EXTERNAL_SDK_ROOTS.contains(&c.as_str()) {
                    return false;
                }
            }
            (None, None) => {}
        }
        match (&self.origin_mod, &other.origin_mod) {
            (Some(a), Some(b)) if self.origin_crate.is_none() && other.origin_crate.is_none() => {
                a == b
            }
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ImportScope {
    /// Current workspace crate name (snake_case, e.g. `"nve_components"`), if known.
    pub current_crate: Option<String>,
    /// Types defined locally in this file/module (`enum`, `struct`, `type`).
    pub local_types: BTreeSet<String>,
    /// Local child `mod` declarations or imported module aliases -> resolved origin crate.
    pub mod_imports: BTreeMap<String, Option<String>>,
    /// Imported or `type`-aliased type identifier -> canonical `EnumQualifier`.
    pub type_aliases: BTreeMap<String, EnumQualifier>,
    /// Explicitly imported enum variant -> `(EnumQualifier, original_variant_name)`.
    pub variant_imports: BTreeMap<String, (EnumQualifier, String)>,
    /// `use path::Enum::*;` -> `EnumQualifier`.
    pub glob_enums: BTreeSet<EnumQualifier>,
    /// True if scope has a wildcard import from a module (`use super::*;`, `use crate::foo::*;`).
    pub has_unscoped_glob: bool,
}

impl ImportScope {
    pub fn with_crate(current_crate: Option<&str>) -> Self {
        Self {
            current_crate: current_crate.map(ToString::to_string),
            ..Self::default()
        }
    }

    /// Creates a fresh lexical import scope for an inline `mod foo { ... }` block (Rust inline
    /// modules do not inherit parent `use` imports, while remaining in the same crate).
    pub fn child_for_inline_mod(parent: &Self, items: &[Item]) -> Self {
        let mut child = Self::with_crate(parent.current_crate.as_deref());
        child.ingest_items(items);
        child
    }

    pub fn has_block_scope_items(stmts: &[Stmt]) -> bool {
        stmts.iter().any(|stmt| match stmt {
            Stmt::Item(item) => is_scope_item(item),
            _ => false,
        })
    }

    pub fn ingest_items(&mut self, items: &[Item]) {
        self.ingest_item_refs(items.iter());
    }

    pub fn ingest_block_stmts(&mut self, stmts: &[Stmt]) {
        let items: Vec<&Item> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Item(it) => Some(it),
                _ => None,
            })
            .collect();
        self.ingest_item_refs(items.into_iter());
    }

    fn ingest_item_refs<'a>(&mut self, items: impl Iterator<Item = &'a Item> + Clone) {
        // Pass 1: record local module and type declarations.
        for item in items.clone() {
            match item {
                Item::Mod(m) if !is_cfg_test_attr(&m.attrs) => {
                    self.mod_imports
                        .insert(m.ident.to_string(), self.current_crate.clone());
                }
                Item::Enum(e) if !is_cfg_test_attr(&e.attrs) => {
                    self.local_types.insert(e.ident.to_string());
                }
                Item::Struct(s) if !is_cfg_test_attr(&s.attrs) => {
                    self.local_types.insert(s.ident.to_string());
                }
                Item::Type(t) if !is_cfg_test_attr(&t.attrs) => {
                    self.local_types.insert(t.ident.to_string());
                }
                _ => {}
            }
        }
        // Pass 2: non-glob `use` imports (two passes so module aliases resolve before dependents).
        for _ in 0..2 {
            for item in items.clone() {
                if let Item::Use(u) = item
                    && !is_cfg_test_attr(&u.attrs)
                {
                    let mut prefix = Vec::new();
                    self.ingest_tree(&u.tree, &mut prefix, false);
                }
            }
        }
        // Pass 3: fixed-point `type Alias = Target;` resolution without crossing `use` boundaries.
        for _ in 0..4 {
            let mut changed = false;
            for item in items.clone() {
                if let Item::Type(t) = item
                    && !is_cfg_test_attr(&t.attrs)
                {
                    changed |= self.ingest_type_alias(t);
                }
            }
            if !changed {
                break;
            }
        }
        // Pass 4: `UseTree::Glob` AFTER all aliases/types resolve so `use Al::*; use crate::Align as Al;`
        // and `type Al = Align; use Al::*;` never pollute `glob_enums` with stale alias names.
        for item in items {
            if let Item::Use(u) = item
                && !is_cfg_test_attr(&u.attrs)
            {
                let mut prefix = Vec::new();
                self.ingest_tree(&u.tree, &mut prefix, true);
            }
        }
    }

    fn ingest_type_alias(&mut self, item: &syn::ItemType) -> bool {
        let alias = item.ident.to_string();
        if !is_type_ident(&alias) {
            return false;
        }
        if let Type::Path(tp) = &*item.ty
            && tp.qself.is_none()
            && let Some(qual) = self.resolve_type_path(&tp.path, None)
            && self.type_aliases.get(&alias) != Some(&qual)
        {
            self.type_aliases.insert(alias, qual);
            return true;
        }
        false
    }

    fn ingest_tree(&mut self, tree: &UseTree, prefix: &mut Vec<String>, globs_only: bool) {
        match tree {
            UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.ingest_tree(&p.tree, prefix, globs_only);
                prefix.pop();
            }
            UseTree::Group(g) => {
                for item in &g.items {
                    self.ingest_tree(item, prefix, globs_only);
                }
            }
            UseTree::Name(n) if !globs_only => {
                let ident = n.ident.to_string();
                self.record_import(prefix, &ident, &ident);
            }
            UseTree::Rename(r) if !globs_only => {
                let orig = r.ident.to_string();
                let alias = r.rename.to_string();
                self.record_import(prefix, &orig, &alias);
            }
            UseTree::Glob(_) if globs_only => {
                if let Some(last) = prefix.last()
                    && is_type_ident(last)
                {
                    let qual = if prefix.len() == 1
                        && let Some(aliased) = self.type_aliases.get(last)
                    {
                        aliased.clone()
                    } else {
                        self.qualifier_from_prefix(prefix)
                    };
                    self.glob_enums.insert(qual);
                } else {
                    self.has_unscoped_glob = true;
                }
            }
            _ => {}
        }
    }

    fn record_import(&mut self, prefix: &[String], orig: &str, alias: &str) {
        if alias == "_" {
            return;
        }
        if orig == "self" {
            let Some(last) = prefix.last() else {
                return;
            };
            let bound = if alias == "self" { last.as_str() } else { alias };
            if is_type_ident(bound) {
                let qual = self.qualifier_from_prefix(prefix);
                self.type_aliases.insert(bound.to_string(), qual);
            } else {
                let origin_crate = self.resolve_module_origin_crate(prefix);
                self.mod_imports.insert(bound.to_string(), origin_crate);
            }
            return;
        }
        if !is_type_ident(orig) && !is_type_ident(alias) {
            let mut full = prefix.to_vec();
            full.push(orig.to_string());
            let origin_crate = self.resolve_module_origin_crate(&full);
            self.mod_imports.insert(alias.to_string(), origin_crate);
            return;
        }
        if !is_type_ident(orig) || !is_type_ident(alias) {
            return;
        }
        if let Some(parent) = prefix.last()
            && is_type_ident(parent)
            && !is_screaming_snake_const(orig)
        {
            let enum_qual = if prefix.len() == 1
                && let Some(aliased) = self.type_aliases.get(parent)
            {
                aliased.clone()
            } else {
                self.qualifier_from_prefix(prefix)
            };
            self.variant_imports
                .insert(alias.to_string(), (enum_qual, orig.to_string()));
            return;
        }
        let mut full = prefix.to_vec();
        full.push(orig.to_string());
        let qual = self.qualifier_from_prefix(&full);
        self.type_aliases.insert(alias.to_string(), qual);
    }

    fn resolve_module_origin_crate(&self, segs: &[String]) -> Option<String> {
        let first = segs.first()?.as_str();
        resolve_root_crate(first, self.current_crate.as_deref(), &self.mod_imports)
    }

    /// Builds an `EnumQualifier` for a path whose final segment is the type name.
    pub fn qualifier_from_prefix(&self, segs: &[String]) -> EnumQualifier {
        let type_name = segs.last().cloned().unwrap_or_default();
        let (origin_crate, origin_mod) =
            extract_origin_crate_and_mod(segs, self.current_crate.as_deref(), &self.mod_imports);
        EnumQualifier {
            origin_crate,
            origin_mod,
            type_name,
        }
    }

    /// Resolves a type path (such as `Align`, `Al`, `Self`, `nve_abi::FlexDirection`,
    /// `crate::a::Mode`, or `winit::window::Theme`) into its canonical `EnumQualifier`.
    pub fn resolve_type_path(
        &self,
        path: &syn::Path,
        self_ty: Option<&str>,
    ) -> Option<EnumQualifier> {
        let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.resolve_type_segments(&segs, self_ty)
    }

    pub fn resolve_type_segments(
        &self,
        segs: &[String],
        self_ty: Option<&str>,
    ) -> Option<EnumQualifier> {
        if segs.is_empty() {
            return None;
        }
        if segs.len() == 1 {
            let raw = &segs[0];
            if raw == "Self" {
                let ty = self_ty?;
                return Some(self.resolve_single_type_ident(ty));
            }
            if !is_type_ident(raw) {
                return None;
            }
            return Some(self.resolve_single_type_ident(raw));
        }
        let last = segs.last()?;
        if !is_type_ident(last) {
            return None;
        }
        let (origin_crate, origin_mod) =
            extract_origin_crate_and_mod(segs, self.current_crate.as_deref(), &self.mod_imports);
        Some(EnumQualifier {
            origin_crate,
            origin_mod,
            type_name: last.clone(),
        })
    }

    pub fn resolve_single_type_ident(&self, ident: &str) -> EnumQualifier {
        if let Some(q) = self.type_aliases.get(ident) {
            return q.clone();
        }
        if self.local_types.contains(ident) {
            return EnumQualifier::local(ident, self.current_crate.as_deref());
        }
        EnumQualifier::unresolved(ident)
    }

    /// Unified resolver for unqualified enum variants across `Pat::Ident`, `Expr::Path`,
    /// `Expr::Call`, and `Expr::Struct`. Only resolves when an explicit variant import or glob
    /// import is active (never tags ordinary structs/newtypes as enum variants).
    pub fn resolve_unqualified_variant(
        &self,
        name: &str,
    ) -> Option<(Option<EnumQualifier>, String, bool)> {
        if matches!(name, "Some" | "None" | "Ok" | "Err") {
            return None;
        }
        if let Some((q, v)) = self.variant_imports.get(name) {
            return Some((Some(q.clone()), v.clone(), false));
        }
        if !is_type_ident(name) || is_unqualified_const_ident(name) {
            return None;
        }
        if self.type_aliases.contains_key(name) || self.local_types.contains(name) {
            return None;
        }
        if self.glob_enums.len() == 1
            && let Some(q) = self.glob_enums.iter().next()
        {
            return Some((Some(q.clone()), name.to_string(), false));
        }
        if !self.glob_enums.is_empty() || self.has_unscoped_glob {
            return Some((None, name.to_string(), true));
        }
        None
    }

    pub fn is_external_sdk_path(&self, path: &syn::Path) -> bool {
        let Some(first) = path.segments.first() else {
            return false;
        };
        let name = first.ident.to_string();
        if EXTERNAL_SDK_ROOTS.contains(&name.as_str()) {
            return true;
        }
        if self
            .type_aliases
            .get(&name)
            .is_some_and(EnumQualifier::is_external_sdk)
        {
            return true;
        }
        if let Some(Some(crate_name)) = self.mod_imports.get(&name)
            && EXTERNAL_SDK_ROOTS.contains(&crate_name.as_str())
        {
            return true;
        }
        false
    }
}

fn is_scope_item(item: &Item) -> bool {
    match item {
        Item::Use(u) => !is_cfg_test_attr(&u.attrs),
        Item::Type(t) => !is_cfg_test_attr(&t.attrs),
        Item::Mod(m) => !is_cfg_test_attr(&m.attrs),
        Item::Enum(e) => !is_cfg_test_attr(&e.attrs),
        Item::Struct(s) => !is_cfg_test_attr(&s.attrs),
        _ => false,
    }
}

fn resolve_root_crate(
    first: &str,
    current_crate: Option<&str>,
    mod_imports: &BTreeMap<String, Option<String>>,
) -> Option<String> {
    if EXTERNAL_SDK_ROOTS.contains(&first) {
        return Some(first.to_string());
    }
    if matches!(first, "crate" | "self" | "super") {
        current_crate.map(ToString::to_string)
    } else if let Some(mapped) = mod_imports.get(first) {
        mapped.clone()
    } else if !is_type_ident(first) {
        Some(first.to_string())
    } else {
        current_crate.map(ToString::to_string)
    }
}

fn extract_origin_crate_and_mod(
    segs: &[String],
    current_crate: Option<&str>,
    mod_imports: &BTreeMap<String, Option<String>>,
) -> (Option<String>, Option<String>) {
    if segs.len() <= 1 {
        return (None, None);
    }
    let prefix = &segs[..segs.len() - 1];
    let origin_crate = resolve_root_crate(prefix[0].as_str(), current_crate, mod_imports);
    if origin_crate
        .as_deref()
        .is_some_and(|c| EXTERNAL_SDK_ROOTS.contains(&c))
    {
        return (origin_crate, None);
    }
    let origin_mod = prefix
        .last()
        .filter(|s| !matches!(s.as_str(), "crate" | "self" | "super") && !is_type_ident(s))
        .filter(|s| origin_crate.as_deref() != Some(s.as_str()))
        .cloned();
    (origin_crate, origin_mod)
}

pub(super) fn is_type_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase())
}

/// True for `SCREAMING_SNAKE_CASE` constants containing `_` (e.g. `MAX_LEN`, `KEY_ENTER`).
pub(super) fn is_screaming_snake_const(s: &str) -> bool {
    s.len() >= 2
        && s.contains('_')
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// True for any ALL_CAPS constant name (including single-word `MAX`, `RED`, `ZERO`),
/// used when disambiguating unqualified `Pat::Ident` (where PascalCase variants contain a lowercase char).
pub(super) fn is_unqualified_const_ident(s: &str) -> bool {
    s.len() >= 2
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
