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
        let mut h = FNV1A_64_OFFSET_BASIS;
        h = hash_str(h, "nve.hybrid.file.v1");
        for c in &self.consts {
            h = hash_str(h, "const");
            h = hash_str(h, &c.name);
            h = hash_str(h, &c.ty.canonical_signature());
            h = fnv1a_64_update(h, &c.value.to_le_bytes());
        }
        for a in &self.aliases {
            h = hash_str(h, "alias");
            h = hash_str(h, &a.name);
            h = hash_str(h, &a.target.canonical_signature());
        }
        for e in &self.enums {
            h = hash_enum(h, e);
        }
        for s in &self.structs {
            h = hash_struct(h, s);
        }
        for m in &self.modules {
            h = fnv1a_64_update(h, &m.schema_hash(self).to_le_bytes());
        }
        for v in &self.views {
            h = fnv1a_64_update(h, &v.schema_hash(self).to_le_bytes());
        }
        for f in &self.c_functions {
            let mut rs = BTreeSet::new();
            let mut re = BTreeSet::new();
            h = hash_method(h, f, &mut rs, &mut re);
        }
        h
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
            if let HybridType::Struct(dep) = &f.ty
                && let Some(dep_spec) = self.find_struct(dep)
            {
                self.visit_struct_topo(dep_spec, visited, out);
            }
        }
        out.push(s);
    }

    /// Collects all distinct POD inner types wrapped in `HybridType::Optional(T)` across all
    /// module and view method signatures (used to emit exact `NveHybridOpt<T>` C-ABI structs).
    pub fn collect_pod_option_types(&self) -> Vec<HybridType> {
        let mut by_suffix: BTreeMap<String, HybridType> = BTreeMap::new();
        let mut record_ty = |ty: &HybridType| {
            collect_pod_opts_in_type(ty, self, &mut by_suffix);
        };
        for m in &self.modules {
            for method in &m.methods {
                for p in &method.params {
                    record_ty(&p.ty);
                }
                record_ty(&method.return_type);
            }
        }
        for v in &self.views {
            for method in &v.methods {
                for p in &method.params {
                    record_ty(&p.ty);
                }
                record_ty(&method.return_type);
            }
        }
        by_suffix.into_values().collect()
    }
}

fn collect_pod_opts_in_type(
    ty: &HybridType,
    file: &HybridFileSpec,
    out: &mut BTreeMap<String, HybridType>,
) {
    match ty {
        HybridType::Optional(inner) if inner.is_pod(file) => {
            if let Some(suffix) = inner.pod_opt_suffix() {
                out.entry(suffix).or_insert_with(|| (**inner).clone());
            }
        }
        HybridType::Result { ok, .. } => collect_pod_opts_in_type(ok, file, out),
        _ => {}
    }
}


pub(super) fn hash_str(mut h: u64, s: &str) -> u64 {
    h = fnv1a_64_update(h, &(s.len() as u32).to_le_bytes());
    fnv1a_64_update(h, s.as_bytes())
}

pub(super) fn hash_method(
    mut h: u64,
    m: &HybridMethodSpec,
    ref_structs: &mut BTreeSet<String>,
    ref_enums: &mut BTreeSet<String>,
) -> u64 {
    h = hash_str(h, "method");
    h = hash_str(h, &m.name);
    h = fnv1a_64_update(h, &(m.params.len() as u32).to_le_bytes());
    for p in &m.params {
        h = hash_str(h, &p.name);
        h = hash_str(h, &p.ty.canonical_signature());
        p.ty.collect_refs(ref_structs, ref_enums);
    }
    h = hash_str(h, &m.return_type.canonical_signature());
    m.return_type.collect_refs(ref_structs, ref_enums);
    h
}

fn hash_enum(mut h: u64, e: &HybridEnumSpec) -> u64 {
    h = hash_str(h, "enum");
    h = hash_str(h, &e.name);
    for v in &e.variants {
        h = hash_str(h, &v.name);
        h = fnv1a_64_update(h, &v.value.to_le_bytes());
    }
    h
}

fn hash_struct(mut h: u64, s: &HybridStructSpec) -> u64 {
    h = hash_str(h, if s.is_union { "union" } else { "struct" });
    h = hash_str(h, &s.name);
    if let Some(align) = s.align {
        h = hash_str(h, "align");
        h = fnv1a_64_update(h, &(align as u64).to_le_bytes());
    }
    if let Some(sz) = s.expected_size {
        h = hash_str(h, "size");
        h = fnv1a_64_update(h, &(sz as u64).to_le_bytes());
    }
    if let Some(sz64) = s.expected_size_64 {
        h = hash_str(h, "size64");
        h = fnv1a_64_update(h, &(sz64 as u64).to_le_bytes());
    }
    for f in &s.fields {
        h = hash_str(h, &f.name);
        h = hash_str(h, &f.ty.canonical_signature());
        if let Some(off) = f.offset {
            h = hash_str(h, "offset");
            h = fnv1a_64_update(h, &(off as u64).to_le_bytes());
        }
        if let Some(off64) = f.offset_64 {
            h = hash_str(h, "offset64");
            h = fnv1a_64_update(h, &(off64 as u64).to_le_bytes());
        }
    }
    h
}

pub(super) fn hash_transitive_types(
    mut h: u64,
    file: &HybridFileSpec,
    mut pending_structs: BTreeSet<String>,
    mut seen_enums: BTreeSet<String>,
) -> u64 {
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
            h = hash_enum(h, e);
        }
    }
    for s_name in seen_structs {
        if let Some(s) = file.find_struct(&s_name) {
            h = hash_struct(h, s);
        }
    }
    h
}
