use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::*;

/// Complete parsed or programmatically constructed Hybrid FFI specification file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HybridFileSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default)]
    pub consts: Vec<HybridConstSpec>,
    #[serde(default)]
    pub aliases: Vec<HybridAliasSpec>,
    #[serde(default)]
    pub enums: Vec<HybridEnumSpec>,
    #[serde(default)]
    pub structs: Vec<HybridStructSpec>,
    #[serde(default)]
    pub modules: Vec<HybridModuleSpec>,
    #[serde(default)]
    pub views: Vec<HybridViewSpec>,
    #[serde(default)]
    pub c_functions: Vec<HybridMethodSpec>,
}

impl HybridFileSpec {
    /// Rewrites `Struct(name)` references that name a declared enum into `Enum(name)` and a
    /// declared alias into `Handle(name)`. Frontends call it once every declaration is known, since
    /// a type can be used before it is declared.
    pub(crate) fn resolve_named_types(&mut self) {
        let Self {
            consts,
            aliases,
            enums,
            structs,
            modules,
            views,
            c_functions,
            ..
        } = self;
        if enums.is_empty() && aliases.is_empty() {
            return;
        }
        let enum_names: BTreeSet<&str> = enums.iter().map(|e| e.name.as_str()).collect();
        let alias_names: BTreeSet<String> = aliases.iter().map(|a| a.name.clone()).collect();
        let rewrite = |ty: &mut HybridType| rewrite_named_type(ty, &enum_names, &alias_names);
        consts.iter_mut().for_each(|c| rewrite(&mut c.ty));
        aliases.iter_mut().for_each(|a| rewrite(&mut a.target));
        for f in structs.iter_mut().flat_map(|s| s.fields.iter_mut()) {
            rewrite(&mut f.ty);
        }
        let module_methods = modules.iter_mut().flat_map(|m| m.methods.iter_mut());
        for v in views.iter_mut() {
            v.props.iter_mut().for_each(|p| rewrite(&mut p.ty));
            for p in v.events.iter_mut().flat_map(|e| e.params.iter_mut()) {
                rewrite(&mut p.ty);
            }
            for m in &mut v.methods {
                m.params.iter_mut().for_each(|p| rewrite(&mut p.ty));
                rewrite(&mut m.return_type);
            }
        }
        for m in module_methods.chain(c_functions.iter_mut()) {
            m.params.iter_mut().for_each(|p| rewrite(&mut p.ty));
            rewrite(&mut m.return_type);
        }
    }

    pub fn find_struct(&self, name: &str) -> Option<&HybridStructSpec> {
        self.structs.iter().find(|s| s.name == name)
    }

    pub fn find_enum(&self, name: &str) -> Option<&HybridEnumSpec> {
        self.enums.iter().find(|e| e.name == name)
    }

    /// PascalCase stem used for generated filenames (`<Stem>HybridAbi.h`, `<Stem>HybridSpec.swift`, etc.).
    pub fn stem_pascal(&self) -> String {
        if let Some(ns) = &self.namespace
            && !ns.trim().is_empty()
        {
            return to_pascal_case(ns);
        }
        if let Some(m) = self.modules.first() {
            return to_pascal_case(&m.name);
        }
        if let Some(v) = self.views.first() {
            return to_pascal_case(&v.name);
        }
        if let Some(s) = self.structs.first() {
            return to_pascal_case(&s.name);
        }
        if let Some(e) = self.enums.first() {
            return to_pascal_case(&e.name);
        }
        "Nve".to_string()
    }

    /// snake_case stem used for Rust/Dart/TS generated filenames (`<stem>_hybrid.rs`, etc.).
    pub fn stem_snake(&self) -> String {
        to_snake_case(&self.stem_pascal())
    }

    /// Deterministic file-wide 64-bit schema hash covering all items in order.
    pub fn file_schema_hash(&self) -> u64 {
        let mut h = Xxh64Hasher::new(0);
        hash_str(&mut h, "nve.hybrid.file.v1");
        for c in &self.consts {
            hash_str(&mut h, "const");
            hash_str(&mut h, &c.name);
            hash_str(&mut h, &c.ty.canonical_signature());
            h.update(&c.value.to_le_bytes());
        }
        for a in &self.aliases {
            hash_str(&mut h, "alias");
            hash_str(&mut h, &a.name);
            hash_str(&mut h, &a.target.canonical_signature());
        }
        for e in &self.enums {
            hash_enum(&mut h, e);
        }
        for s in &self.structs {
            hash_struct(&mut h, s);
        }
        for m in &self.modules {
            h.update(&m.schema_hash(self).to_le_bytes());
        }
        for v in &self.views {
            h.update(&v.schema_hash(self).to_le_bytes());
        }
        for f in &self.c_functions {
            let mut rs = BTreeSet::new();
            let mut re = BTreeSet::new();
            hash_method(&mut h, f, &mut rs, &mut re);
        }
        h.digest()
    }

    /// Returns all structs in topological dependency order (dependencies emitted before dependents
    /// so C11/C++20 structs that embed POD structs by value always compile cleanly).
    pub fn structs_topological(&self) -> Vec<&HybridStructSpec> {
        let mut result = Vec::with_capacity(self.structs.len());
        let mut visited = BTreeSet::new();
        for s in &self.structs {
            self.visit_struct_topo(s, &mut visited, &mut result);
        }
        result
    }

    fn visit_struct_topo<'a>(
        &'a self,
        s: &'a HybridStructSpec,
        visited: &mut BTreeSet<&'a str>,
        out: &mut Vec<&'a HybridStructSpec>,
    ) {
        if !visited.insert(s.name.as_str()) {
            return;
        }
        for f in &s.fields {
            // A struct held by value, in an option, a `Vec`, or a `[T; N]` must be complete first.
            let mut ty = &f.ty;
            while let HybridType::Optional(inner)
            | HybridType::Vec(inner)
            | HybridType::FixedArray { elem: inner, .. } = ty
            {
                ty = inner;
            }
            if let HybridType::Struct(dep) = ty
                && let Some(dep_spec) = self.find_struct(dep)
            {
                self.visit_struct_topo(dep_spec, visited, out);
            }
        }
        out.push(s);
    }

    /// Collects all distinct `(suffix, inner)` POD types wrapped in `HybridType::Optional(T)`
    /// across every module, view, and static C function signature (used to emit exact
    /// `NveHybridOpt<T>` C-ABI structs).
    pub fn collect_pod_option_types(&self) -> Vec<(std::borrow::Cow<'static, str>, HybridType)> {
        let mut by_suffix: BTreeMap<std::borrow::Cow<'static, str>, HybridType> = BTreeMap::new();
        let interfaces = self
            .modules
            .iter()
            .map(|m| &m.methods)
            .chain(self.views.iter().map(|v| &v.methods));
        for method in interfaces.flatten().chain(&self.c_functions) {
            for ty in method
                .params
                .iter()
                .map(|p| &p.ty)
                .chain([&method.return_type])
            {
                collect_pod_opts_in_type(ty, self, &mut by_suffix);
            }
        }
        by_suffix.into_iter().collect()
    }
}

fn collect_pod_opts_in_type(
    ty: &HybridType,
    file: &HybridFileSpec,
    out: &mut BTreeMap<std::borrow::Cow<'static, str>, HybridType>,
) {
    match ty {
        HybridType::Optional(inner) => {
            if let Some(suffix) = inner.pod_opt_suffix_for(file) {
                out.entry(suffix).or_insert_with(|| (**inner).clone());
            }
        }
        HybridType::Result { ok, .. } => collect_pod_opts_in_type(ok, file, out),
        _ => {}
    }
}

pub(crate) fn hash_str(h: &mut Xxh64Hasher, s: &str) {
    h.update_u32(s.len() as u32);
    h.update(s.as_bytes());
}

pub(super) fn hash_method(
    h: &mut Xxh64Hasher,
    m: &HybridMethodSpec,
    ref_structs: &mut BTreeSet<String>,
    ref_enums: &mut BTreeSet<String>,
) {
    hash_str(h, "method");
    hash_str(h, &m.name);
    h.update_u32(m.params.len() as u32);
    for p in &m.params {
        hash_str(h, &p.name);
        hash_str(h, &p.ty.canonical_signature());
        p.ty.collect_refs(ref_structs, ref_enums);
    }
    hash_str(h, &m.return_type.canonical_signature());
    m.return_type.collect_refs(ref_structs, ref_enums);
}

fn hash_enum(h: &mut Xxh64Hasher, e: &HybridEnumSpec) {
    hash_str(h, "enum");
    hash_str(h, &e.name);
    for v in &e.variants {
        hash_str(h, &v.name);
        h.update(&v.value.to_le_bytes());
    }
}

fn hash_struct(h: &mut Xxh64Hasher, s: &HybridStructSpec) {
    hash_str(h, if s.is_union { "union" } else { "struct" });
    hash_str(h, &s.name);
    if let Some(align) = s.align {
        hash_str(h, "align");
        h.update_u64(align as u64);
    }
    if let Some(sz) = s.expected_size {
        hash_str(h, "size");
        h.update_u64(sz as u64);
    }
    if let Some(sz64) = s.expected_size_64 {
        hash_str(h, "size64");
        h.update_u64(sz64 as u64);
    }
    for f in &s.fields {
        hash_str(h, &f.name);
        hash_str(h, &f.ty.canonical_signature());
        if let Some(off) = f.offset {
            hash_str(h, "offset");
            h.update_u64(off as u64);
        }
        if let Some(off64) = f.offset_64 {
            hash_str(h, "offset64");
            h.update_u64(off64 as u64);
        }
    }
}

pub(super) fn hash_transitive_types(
    h: &mut Xxh64Hasher,
    file: &HybridFileSpec,
    mut pending_structs: BTreeSet<String>,
    mut seen_enums: BTreeSet<String>,
) {
    let mut seen_structs = BTreeSet::new();
    while let Some(name) = pending_structs.pop_first() {
        if !seen_structs.insert(name.clone()) {
            continue;
        }
        if let Some(s) = file.find_struct(&name) {
            for f in &s.fields {
                let mut child_structs = BTreeSet::new();
                f.ty.collect_refs(&mut child_structs, &mut seen_enums);
                for cs in child_structs {
                    if !seen_structs.contains(&cs) {
                        pending_structs.insert(cs);
                    }
                }
            }
        }
    }
    for e_name in seen_enums {
        if let Some(e) = file.find_enum(&e_name) {
            hash_enum(h, e);
        }
    }
    for s_name in seen_structs {
        if let Some(s) = file.find_struct(&s_name) {
            hash_struct(h, s);
        }
    }
}

fn rewrite_named_type(ty: &mut HybridType, enums: &BTreeSet<&str>, aliases: &BTreeSet<String>) {
    match ty {
        HybridType::Struct(name) if enums.contains(name.as_str()) => {
            *ty = HybridType::Enum(std::mem::take(name))
        }
        HybridType::Struct(name) if aliases.contains(name) => {
            *ty = HybridType::Handle(std::mem::take(name))
        }
        HybridType::Optional(inner)
        | HybridType::Vec(inner)
        | HybridType::ConstPtr(inner)
        | HybridType::MutPtr(inner)
        | HybridType::FixedArray { elem: inner, .. } => rewrite_named_type(inner, enums, aliases),
        HybridType::Result { ok, err } => {
            rewrite_named_type(ok, enums, aliases);
            rewrite_named_type(err, enums, aliases);
        }
        _ => {}
    }
}
