//! Lexical `use` & `type` alias scope and crate/module-aware enum qualifiers for `ast_enums`.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Item, Stmt, Type, UseTree};

use crate::is_cfg_test_attr;

pub const EXTERNAL_SDK_ROOTS: &[&str] = &[
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
/// while local definitions (`origin_crate` matching current crate or `None` in synthetic tests)
/// still match external imports of the same workspace crate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EnumQualifier {
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

    pub fn is_external_sdk(&self) -> bool {
        self.origin_crate
            .as_deref()
            .is_some_and(|c| EXTERNAL_SDK_ROOTS.contains(&c))
    }

    /// Canonical string key used in normalized expressions, paths, and site qualifiers.
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
pub struct ImportScope {
    /// Current workspace crate name (snake_case, e.g. `"nve_components"`), if known.
    pub current_crate: Option<String>,
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

    pub fn has_scope_items(items: &[Item]) -> bool {
        items.iter().any(|item| match item {
            Item::Use(u) => !is_cfg_test_attr(&u.attrs),
            Item::Type(t) => !is_cfg_test_attr(&t.attrs),
            Item::Mod(m) => !is_cfg_test_attr(&m.attrs),
            _ => false,
        })
    }

    pub fn has_block_scope_items(stmts: &[Stmt]) -> bool {
        stmts.iter().any(|stmt| match stmt {
            Stmt::Item(Item::Use(u)) => !is_cfg_test_attr(&u.attrs),
            Stmt::Item(Item::Type(t)) => !is_cfg_test_attr(&t.attrs),
            Stmt::Item(Item::Mod(m)) => !is_cfg_test_attr(&m.attrs),
            _ => false,
        })
    }

    pub fn ingest_items(&mut self, items: &[Item]) {
        for item in items {
            if let Item::Mod(m) = item
                && !is_cfg_test_attr(&m.attrs)
            {
                self.mod_imports
                    .insert(m.ident.to_string(), self.current_crate.clone());
            }
        }
        for item in items {
            if let Item::Use(u) = item
                && !is_cfg_test_attr(&u.attrs)
            {
                let mut prefix = Vec::new();
                self.ingest_tree(&u.tree, &mut prefix);
            }
        }
        // Fixed-point passes for `type Alias = Target;` so chained local `type` aliases resolve cleanly
        // without ever letting a `use other_crate::Foo as Bar;` alias chain into a local `use this::Foo;`.
        for _ in 0..4 {
            let mut changed = false;
            for item in items {
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
    }

    pub fn ingest_block_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let Stmt::Item(Item::Mod(m)) = stmt
                && !is_cfg_test_attr(&m.attrs)
            {
                self.mod_imports
                    .insert(m.ident.to_string(), self.current_crate.clone());
            }
        }
        for stmt in stmts {
            if let Stmt::Item(Item::Use(u)) = stmt
                && !is_cfg_test_attr(&u.attrs)
            {
                let mut prefix = Vec::new();
                self.ingest_tree(&u.tree, &mut prefix);
            }
        }
        for _ in 0..4 {
            let mut changed = false;
            for stmt in stmts {
                if let Stmt::Item(Item::Type(t)) = stmt
                    && !is_cfg_test_attr(&t.attrs)
                {
                    changed |= self.ingest_type_alias(t);
                }
            }
            if !changed {
                break;
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

    fn ingest_tree(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.ingest_tree(&p.tree, prefix);
                prefix.pop();
            }
            UseTree::Group(g) => {
                for item in &g.items {
                    self.ingest_tree(item, prefix);
                }
            }
            UseTree::Name(n) => {
                let ident = n.ident.to_string();
                self.record_import(prefix, &ident, &ident);
            }
            UseTree::Rename(r) => {
                let orig = r.ident.to_string();
                let alias = r.rename.to_string();
                self.record_import(prefix, &orig, &alias);
            }
            UseTree::Glob(_) => {
                if let Some(last) = prefix.last()
                    && is_type_ident(last)
                {
                    let qual = self.qualifier_from_prefix(prefix);
                    self.glob_enums.insert(qual);
                } else {
                    self.has_unscoped_glob = true;
                }
            }
        }
    }

    fn record_import(&mut self, prefix: &[String], orig: &str, alias: &str) {
        if alias == "_" {
            return;
        }
        if orig == "self" {
            if !prefix.is_empty() && !is_type_ident(alias) {
                let (origin_crate, _) =
                    extract_origin_crate_and_mod(prefix, self.current_crate.as_deref(), &self.mod_imports);
                self.mod_imports.insert(alias.to_string(), origin_crate);
            }
            return;
        }
        if !is_type_ident(orig) && !is_type_ident(alias) {
            let mut full = prefix.to_vec();
            full.push(orig.to_string());
            let origin_crate = if full.len() == 1 {
                if matches!(orig, "crate" | "self" | "super") {
                    self.current_crate.clone()
                } else if let Some(mapped) = self.mod_imports.get(orig) {
                    mapped.clone()
                } else {
                    Some(orig.to_string())
                }
            } else {
                extract_origin_crate_and_mod(&full, self.current_crate.as_deref(), &self.mod_imports).0
            };
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
            let enum_qual = self.qualifier_from_prefix(prefix);
            self.variant_imports
                .insert(alias.to_string(), (enum_qual, orig.to_string()));
            return;
        }
        let mut full = prefix.to_vec();
        full.push(orig.to_string());
        let qual = self.qualifier_from_prefix(&full);
        self.type_aliases.insert(alias.to_string(), qual);
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
    pub fn resolve_type_path(&self, path: &syn::Path, self_ty: Option<&str>) -> Option<EnumQualifier> {
        let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.resolve_type_segments(&segs, self_ty)
    }

    pub fn resolve_type_segments(&self, segs: &[String], self_ty: Option<&str>) -> Option<EnumQualifier> {
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
        EnumQualifier::local(ident, self.current_crate.as_deref())
    }

    pub fn is_external_sdk_path(&self, path: &syn::Path) -> bool {
        if path
            .segments
            .iter()
            .any(|s| EXTERNAL_SDK_ROOTS.contains(&s.ident.to_string().as_str()))
        {
            return true;
        }
        if let Some(first) = path.segments.first() {
            let name = first.ident.to_string();
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
        }
        if let Some(last) = path.segments.last() {
            let name = last.ident.to_string();
            if self
                .type_aliases
                .get(&name)
                .is_some_and(EnumQualifier::is_external_sdk)
            {
                return true;
            }
        }
        false
    }

    pub fn glob_matches(&self, target: &EnumQualifier) -> bool {
        self.glob_enums.iter().any(|g| g.compatible_with(target))
    }
}

fn extract_origin_crate_and_mod(
    segs: &[String],
    current_crate: Option<&str>,
    mod_imports: &BTreeMap<String, Option<String>>,
) -> (Option<String>, Option<String>) {
    if segs.len() <= 1 {
        return (current_crate.map(ToString::to_string), None);
    }
    let prefix = &segs[..segs.len() - 1];
    if let Some(sdk) = prefix.iter().find(|s| EXTERNAL_SDK_ROOTS.contains(&s.as_str())) {
        return (Some(sdk.clone()), None);
    }
    let first = prefix[0].as_str();
    let origin_crate = if matches!(first, "crate" | "self" | "super") {
        current_crate.map(ToString::to_string)
    } else if let Some(mapped) = mod_imports.get(first) {
        mapped.clone()
    } else if !is_type_ident(first) {
        Some(first.to_string())
    } else {
        current_crate.map(ToString::to_string)
    };
    let origin_mod = prefix
        .last()
        .filter(|s| !matches!(s.as_str(), "crate" | "self" | "super") && !is_type_ident(s))
        .filter(|s| origin_crate.as_deref() != Some(s.as_str()))
        .cloned();
    (origin_crate, origin_mod)
}

pub fn is_type_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_uppercase())
}

/// True for `SCREAMING_SNAKE_CASE` constants containing `_` (e.g. `MAX_LEN`, `KEY_ENTER`).
pub fn is_screaming_snake_const(s: &str) -> bool {
    s.len() >= 2
        && s.contains('_')
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// True for any ALL_CAPS constant name (including single-word `MAX`, `RED`, `ZERO`),
/// used when disambiguating unqualified `Pat::Ident` (where PascalCase variants contain a lowercase char).
pub fn is_unqualified_const_ident(s: &str) -> bool {
    s.len() >= 2
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
