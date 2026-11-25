//! Intermediate Representation (IR), deterministic FNV-1a schema hashing, interface ID
//! assignment, and strict semantic validation for NVE's Nitro-inspired Rust-first Hybrid FFI
//! code generator.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::CodegenError;
use crate::naming::{to_pascal_case, to_snake_case};

/// Base interface ID for generated hybrid vtables (`>= 0x1000_0000`), keeping the low range
/// (`1..=4`) permanently reserved for core NVE interfaces (`NVE_IFACE_SURFACE..NVE_IFACE_DEV`).
pub const NVE_HYBRID_INTERFACE_ID_BASE: u32 = 0x1000_0000;

/// Bitmask for the 28-bit deterministic hash payload inside a hybrid `interface_id`.
pub const NVE_HYBRID_INTERFACE_ID_MASK: u32 = 0x0FFF_FFFF;

/// Current Hybrid FFI vtable ABI version stamped into every generated vtable.
pub const NVE_HYBRID_ABI_VERSION: u32 = 1;

/// Deterministic 64-bit FNV-1a hash helper.
pub const fn fnv1a_64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        i += 1;
    }
    hash
}

/// FNV-1a 64-bit offset basis.
pub const FNV1A_64_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// Computes the deterministic FNV-1a 64-bit hash of a byte slice.
pub const fn fnv1a_64(bytes: &[u8]) -> u64 {
    fnv1a_64_update(FNV1A_64_OFFSET_BASIS, bytes)
}

/// Computes a deterministic hybrid `interface_id >= 0x1000_0000` for a module or view.
pub fn compute_hybrid_interface_id(kind: &str, name: &str) -> u32 {
    let mut h = FNV1A_64_OFFSET_BASIS;
    h = fnv1a_64_update(h, b"nve.hybrid.iface.v1:");
    h = fnv1a_64_update(h, kind.as_bytes());
    h = fnv1a_64_update(h, b":");
    h = fnv1a_64_update(h, name.as_bytes());
    let folded = ((h as u32) ^ ((h >> 32) as u32)) & NVE_HYBRID_INTERFACE_ID_MASK;
    NVE_HYBRID_INTERFACE_ID_BASE | folded
}

/// Target languages emitted by the Hybrid FFI generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridTarget {
    Rust,
    Cpp,
    Swift,
    Kotlin,
    Dart,
    TypeScript,
}

impl HybridTarget {
    pub const ALL: &'static [Self] = &[
        Self::Rust,
        Self::Cpp,
        Self::Swift,
        Self::Kotlin,
        Self::Dart,
        Self::TypeScript,
    ];

    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Cpp => "cpp",
            Self::Swift => "swift",
            Self::Kotlin => "kotlin",
            Self::Dart => "dart",
            Self::TypeScript => "ts",
        }
    }

    pub fn from_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "rust" | "rs" => Some(Self::Rust),
            "cpp" | "c++" | "c" | "cxx" => Some(Self::Cpp),
            "swift" => Some(Self::Swift),
            "kotlin" | "kt" | "android" | "jni" => Some(Self::Kotlin),
            "dart" | "flutter" => Some(Self::Dart),
            "ts" | "typescript" | "wasm" | "web" => Some(Self::TypeScript),
            _ => None,
        }
    }
}

/// Rich typed system shared across Hybrid modules, views, structs, enums, and events.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HybridType {
    Void,
    Bool,
    U8,
    U16,
    Usize,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    String,
    Bytes,
    Optional(Box<HybridType>),
    Vec(Box<HybridType>),
    Result {
        ok: Box<HybridType>,
        err: Box<HybridType>,
    },
    Struct(String),
    Enum(String),
    Value,
    ConstPtr(Box<HybridType>),
    MutPtr(Box<HybridType>),
    Handle(String),
}

impl HybridType {
    /// Canonical deterministic type signature used for FNV-1a schema hashing.
    pub fn canonical_signature(&self) -> String {
        match self {
            Self::Void => "void".to_string(),
            Self::Bool => "bool".to_string(),
            Self::U8 => "u8".to_string(),
            Self::U16 => "u16".to_string(),
            Self::Usize => "usize".to_string(),
            Self::I32 => "i32".to_string(),
            Self::U32 => "u32".to_string(),
            Self::I64 => "i64".to_string(),
            Self::U64 => "u64".to_string(),
            Self::F32 => "f32".to_string(),
            Self::F64 => "f64".to_string(),
            Self::String => "string".to_string(),
            Self::Bytes => "bytes".to_string(),
            Self::Optional(inner) => format!("opt<{}>", inner.canonical_signature()),
            Self::Vec(inner) => format!("vec<{}>", inner.canonical_signature()),
            Self::Result { ok, err } => {
                format!(
                    "result<{},{}>",
                    ok.canonical_signature(),
                    err.canonical_signature()
                )
            }
            Self::Struct(name) => format!("struct:{name}"),
            Self::Enum(name) => format!("enum:{name}"),
            Self::Value => "nve_value".to_string(),
            Self::ConstPtr(inner) => format!("*const {}", inner.canonical_signature()),
            Self::MutPtr(inner) => format!("*mut {}", inner.canonical_signature()),
            Self::Handle(name) => format!("handle:{name}"),
        }
    }

    /// True if this type is a fixed-size C-ABI scalar or POD struct that can be passed
    /// directly by value across C11/C++20/Rust/Swift/Dart FFI without serialization.
    pub fn is_pod(&self, file: &HybridFileSpec) -> bool {
        match self {
            Self::Bool
            | Self::U8
            | Self::U16
            | Self::Usize
            | Self::I32
            | Self::U32
            | Self::I64
            | Self::U64
            | Self::F32
            | Self::F64
            | Self::Enum(_) => true,
            Self::Struct(name) => file
                .find_struct(name)
                .is_some_and(|s| s.is_pod(file)),
            Self::Void
            | Self::String
            | Self::Bytes
            | Self::Optional(_)
            | Self::Vec(_)
            | Self::Result { .. }
            | Self::Value
            | Self::ConstPtr(_)
            | Self::MutPtr(_)
            | Self::Handle(_) => false,
        }
    }

    /// True if this type is a `Result<_, _>`.
    pub fn is_result(&self) -> bool {
        matches!(self, Self::Result { .. })
    }

    /// Collects all referenced user struct and enum names into `structs` and `enums`.
    pub fn collect_refs(&self, structs: &mut BTreeSet<String>, enums: &mut BTreeSet<String>) {
        match self {
            Self::Struct(name) => {
                structs.insert(name.clone());
            }
            Self::Enum(name) => {
                enums.insert(name.clone());
            }
            Self::Optional(inner)
            | Self::Vec(inner)
            | Self::ConstPtr(inner)
            | Self::MutPtr(inner) => inner.collect_refs(structs, enums),
            Self::Result { ok, err } => {
                ok.collect_refs(structs, enums);
                err.collect_refs(structs, enums);
            }
            _ => {}
        }
    }

    /// Deterministic C-ABI option wrapper suffix for POD optional types (`Optional<T>`).
    /// Returns `Some("I64")`, `Some("Vec2")`, etc. when `self` is a POD inner type.
    pub fn pod_opt_suffix(&self) -> Option<String> {
        match self {
            Self::Bool => Some("Bool".to_string()),
            Self::U8 => Some("U8".to_string()),
            Self::U16 => Some("U16".to_string()),
            Self::Usize => Some("Usize".to_string()),
            Self::I32 => Some("I32".to_string()),
            Self::U32 => Some("U32".to_string()),
            Self::I64 => Some("I64".to_string()),
            Self::U64 => Some("U64".to_string()),
            Self::F32 => Some("F32".to_string()),
            Self::F64 => Some("F64".to_string()),
            Self::Enum(name) | Self::Struct(name) => Some(to_pascal_case(name)),
            _ => None,
        }
    }
}

/// Typed literal default value for a `#[prop(default = ...)]` on a `HybridView`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum HybridDefaultValue {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
    EnumVariant { enum_name: String, variant: String },
    None,
}

impl HybridDefaultValue {
    pub fn canonical_signature(&self) -> String {
        match self {
            Self::Bool(v) => format!("bool:{v}"),
            Self::Int(v) => format!("int:{v}"),
            Self::UInt(v) => format!("uint:{v}"),
            Self::Float(v) => format!("float:{:016x}", v.to_bits()),
            Self::String(v) => format!("str:{v}"),
            Self::EnumVariant { enum_name, variant } => format!("enum:{enum_name}::{variant}"),
            Self::None => "none".to_string(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Named constant declared with `#[nve_hybrid_const] pub const ...`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridConstSpec {
    pub name: String,
    pub ty: HybridType,
    pub value: u64,
    #[serde(default)]
    pub hex: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Opaque handle or typedef alias declared with `#[nve_hybrid_alias] pub type ... = ...;`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridAliasSpec {
    pub name: String,
    pub target: HybridType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// One variant of a `#[repr(u32)] #[nve_hybrid_enum]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridEnumVariant {
    pub name: String,
    pub value: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// C-ABI enum declared with `#[nve_hybrid_enum]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HybridEnumSpec {
    pub name: String,
    pub variants: Vec<HybridEnumVariant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Optional explicit C enumerator prefix (e.g. `"NVE_OP_"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_prefix: Option<String>,
    /// Optional explicit Rust `#[repr(...)]` width (e.g. `"u8"`, `"u16"`, `"u32"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repr: Option<String>,
    /// Optional fallback variant name for non-nullable Dart `fromCode(int)` decoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dart_fallback: Option<String>,
}

/// Named field of a `#[nve_hybrid_struct]` or `#[nve_hybrid_union]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridFieldSpec {
    pub name: String,
    pub ty: HybridType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Optional fixed byte offset across all targets (`#[offset(N)]` -> `NVE_STATIC_ASSERT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// Optional fixed byte offset on 64-bit targets (`#[offset64(N)]` -> `sizeof(void*) != 8 || offsetof(...) == N`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_64: Option<usize>,
}

/// Structured record declared with `#[nve_hybrid_struct]` or `#[nve_hybrid_union]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HybridStructSpec {
    pub name: String,
    pub fields: Vec<HybridFieldSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// True when declared as `#[nve_hybrid_union] pub union ...`.
    #[serde(default)]
    pub is_union: bool,
    /// Optional explicit `#[repr(C, align(N))]` alignment descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<usize>,
    /// Optional exact target-independent byte size (`#[nve_hybrid_struct(size = N)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_size: Option<usize>,
    /// Optional exact 64-bit byte size (`#[nve_hybrid_struct(size_64 = N)]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_size_64: Option<usize>,
}

impl HybridStructSpec {
    /// Returns true if all fields (transitively) are fixed-size C-ABI scalars/enums/POD structs.
    pub fn is_pod(&self, file: &HybridFileSpec) -> bool {
        if self.is_union {
            return false;
        }
        let mut visited = BTreeSet::new();
        self.is_pod_inner(file, &mut visited)
    }

    fn is_pod_inner(&self, file: &HybridFileSpec, visited: &mut BTreeSet<String>) -> bool {
        if self.is_union || !visited.insert(self.name.clone()) {
            return false;
        }
        let ok = !self.fields.is_empty()
            && self.fields.iter().all(|f| match &f.ty {
                HybridType::Bool
                | HybridType::U8
                | HybridType::U16
                | HybridType::Usize
                | HybridType::I32
                | HybridType::U32
                | HybridType::I64
                | HybridType::U64
                | HybridType::F32
                | HybridType::F64
                | HybridType::Enum(_) => true,
                HybridType::Struct(other) => file
                    .find_struct(other)
                    .is_some_and(|s| s.is_pod_inner(file, visited)),
                _ => false,
            });
        visited.remove(&self.name);
        ok
    }
}

/// Named parameter of a method or view event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridParamSpec {
    pub name: String,
    pub ty: HybridType,
}

/// Method declared on a `HybridModuleSpec`, imperative `#[method]` on a `HybridViewSpec`, or C API function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridMethodSpec {
    pub name: String,
    #[serde(default)]
    pub params: Vec<HybridParamSpec>,
    pub return_type: HybridType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Whether the trait method takes a `self` receiver (`true` for instance methods, `false` for static/create fns).
    #[serde(default = "default_true")]
    pub has_self: bool,
    /// Whether this vtable slot may be NULL (`#[optional]`).
    #[serde(default)]
    pub optional: bool,
}

/// Typed prop declared with `#[prop]` on a `HybridViewSpec`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HybridPropSpec {
    pub name: String,
    pub ty: HybridType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<HybridDefaultValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Typed event declared with `#[event]` on a `HybridViewSpec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HybridEventSpec {
    pub name: String,
    #[serde(default)]
    pub params: Vec<HybridParamSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

/// Nitro-style stateful native module declared with `#[nve_hybrid_module(name = "...")]`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HybridModuleSpec {
    pub name: String,
    pub trait_name: String,
    #[serde(default)]
    pub methods: Vec<HybridMethodSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Optional explicit fixed interface ID override (e.g. `interface_id = 1` for core engine
    /// interfaces `NVE_IFACE_SURFACE..NVE_IFACE_DEV`). When `None`, defaults to deterministic
    /// FNV-1a ID `>= 0x1000_0000`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_interface_id: Option<u32>,
    /// Optional explicit C vtable struct name (e.g. `"NveSurfaceApi"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_name: Option<String>,
}

impl HybridModuleSpec {
    /// Interface ID: returns `explicit_interface_id` when set, otherwise a deterministic ID
    /// in the reserved hybrid range (`>= 0x1000_0000`).
    pub fn interface_id(&self) -> u32 {
        self.explicit_interface_id
            .unwrap_or_else(|| compute_hybrid_interface_id("module", &self.name))
    }

    /// Deterministic 64-bit FNV-1a schema hash covering this module's methods and all
    /// transitively referenced structs and enums.
    pub fn schema_hash(&self, file: &HybridFileSpec) -> u64 {
        let mut h = FNV1A_64_OFFSET_BASIS;
        h = hash_str(h, "nve.hybrid.module.v1");
        h = hash_str(h, &self.name);
        if let Some(id) = self.explicit_interface_id {
            h = hash_str(h, "iface_id");
            h = fnv1a_64_update(h, &id.to_le_bytes());
        }
        let mut ref_structs = BTreeSet::new();
        let mut ref_enums = BTreeSet::new();
        for m in &self.methods {
            h = hash_method(h, m, &mut ref_structs, &mut ref_enums);
        }
        hash_transitive_types(h, file, ref_structs, ref_enums)
    }
}

/// Nitro-style native view component declared with `#[nve_hybrid_view(name = "...")]`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HybridViewSpec {
    pub name: String,
    pub trait_name: String,
    #[serde(default)]
    pub props: Vec<HybridPropSpec>,
    #[serde(default)]
    pub events: Vec<HybridEventSpec>,
    #[serde(default)]
    pub methods: Vec<HybridMethodSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Optional explicit fixed interface ID override. Defaults to deterministic FNV-1a ID
    /// `>= 0x1000_0000` when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_interface_id: Option<u32>,
}

impl HybridViewSpec {
    /// Interface ID: returns `explicit_interface_id` when set, otherwise a deterministic ID
    /// in the reserved hybrid range (`>= 0x1000_0000`).
    pub fn interface_id(&self) -> u32 {
        self.explicit_interface_id
            .unwrap_or_else(|| compute_hybrid_interface_id("view", &self.name))
    }

    /// Deterministic 64-bit FNV-1a schema hash covering this view's props, events, methods,
    /// and all transitively referenced structs and enums.
    pub fn schema_hash(&self, file: &HybridFileSpec) -> u64 {
        let mut h = FNV1A_64_OFFSET_BASIS;
        h = hash_str(h, "nve.hybrid.view.v1");
        h = hash_str(h, &self.name);
        if let Some(id) = self.explicit_interface_id {
            h = hash_str(h, "iface_id");
            h = fnv1a_64_update(h, &id.to_le_bytes());
        }
        let mut ref_structs = BTreeSet::new();
        let mut ref_enums = BTreeSet::new();
        for p in &self.props {
            h = hash_str(h, "prop");
            h = hash_str(h, &p.name);
            h = hash_str(h, &p.ty.canonical_signature());
            if let Some(def) = &p.default_value {
                h = hash_str(h, &def.canonical_signature());
            }
            p.ty.collect_refs(&mut ref_structs, &mut ref_enums);
        }
        for ev in &self.events {
            h = hash_str(h, "event");
            h = hash_str(h, &ev.name);
            for param in &ev.params {
                h = hash_str(h, &param.name);
                h = hash_str(h, &param.ty.canonical_signature());
                param.ty.collect_refs(&mut ref_structs, &mut ref_enums);
            }
        }
        for m in &self.methods {
            h = hash_method(h, m, &mut ref_structs, &mut ref_enums);
        }
        hash_transitive_types(h, file, ref_structs, ref_enums)
    }
}

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

    /// Strict semantic validation of the entire Hybrid FFI specification.
    pub fn validate(&self) -> Result<(), CodegenError> {
        if self.consts.is_empty()
            && self.aliases.is_empty()
            && self.enums.is_empty()
            && self.structs.is_empty()
            && self.modules.is_empty()
            && self.views.is_empty()
            && self.c_functions.is_empty()
        {
            return Err(CodegenError::validation(
                "hybrid spec must declare at least one enum, struct, module, or view",
            ));
        }

        if let Some(ns) = &self.namespace {
            validate_ident("namespace", ns)?;
        }

        let mut type_names = BTreeSet::new();
        let mut const_names = BTreeSet::new();
        let mut alias_names = BTreeSet::new();
        let mut enum_map: BTreeMap<&str, &HybridEnumSpec> = BTreeMap::new();
        let mut struct_map: BTreeMap<&str, &HybridStructSpec> = BTreeMap::new();

        for c in &self.consts {
            validate_ident("const", &c.name)?;
            if !const_names.insert(c.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate const name '{}'",
                    c.name
                )));
            }
        }

        for a in &self.aliases {
            validate_ident("type alias", &a.name)?;
            if !type_names.insert(a.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    a.name
                )));
            }
            alias_names.insert(a.name.as_str());
        }

        for e in &self.enums {
            validate_ident("enum", &e.name)?;
            if !type_names.insert(e.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    e.name
                )));
            }
            if e.variants.is_empty() {
                return Err(CodegenError::validation(format!(
                    "enum '{}' must declare at least one variant",
                    e.name
                )));
            }
            let mut variant_names = BTreeSet::new();
            let mut variant_values = BTreeSet::new();
            for v in &e.variants {
                validate_ident("enum variant", &v.name)?;
                if !variant_names.insert(v.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate variant '{}' in enum '{}'",
                        v.name, e.name
                    )));
                }
                if !variant_values.insert(v.value) {
                    return Err(CodegenError::validation(format!(
                        "duplicate discriminant value {} in enum '{}'",
                        v.value, e.name
                    )));
                }
            }
            enum_map.insert(e.name.as_str(), e);
        }

        for s in &self.structs {
            validate_ident("struct", &s.name)?;
            if !type_names.insert(s.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate type name '{}'",
                    s.name
                )));
            }
            if let Some(align) = s.align
                && (align == 0 || !align.is_power_of_two())
            {
                return Err(CodegenError::validation(format!(
                    "struct '{}' align {} must be a power of two",
                    s.name, align
                )));
            }
            if s.fields.is_empty() {
                return Err(CodegenError::validation(format!(
                    "struct '{}' must declare at least one field",
                    s.name
                )));
            }
            let mut field_names = BTreeSet::new();
            for f in &s.fields {
                validate_ident("struct field", &f.name)?;
                if !field_names.insert(f.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate field '{}' in struct '{}'",
                        f.name, s.name
                    )));
                }
            }
            struct_map.insert(s.name.as_str(), s);
        }

        for a in &self.aliases {
            validate_type(
                &a.target,
                true,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("type alias '{}'", a.name),
            )?;
        }

        // Validate all struct field types and check for infinite-size recursive cycles.
        for s in &self.structs {
            for f in &s.fields {
                validate_type(
                    &f.ty,
                    false,
                    &enum_map,
                    &struct_map,
                    &alias_names,
                    &format!("struct '{}' field '{}'", s.name, f.name),
                )?;
            }
        }
        detect_recursive_value_structs(&struct_map)?;

        let mut item_names = BTreeSet::new();
        let mut iface_ids: BTreeMap<u32, String> = BTreeMap::new();

        for m in &self.modules {
            validate_ident("module", &m.name)?;
            validate_ident("module trait", &m.trait_name)?;
            if !item_names.insert(m.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate hybrid module/view name '{}'",
                    m.name
                )));
            }
            if type_names.contains(m.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "hybrid module '{}' collides with a struct or enum of the same name",
                    m.name
                )));
            }
            let iface_id = m.interface_id();
            if let Some(explicit) = m.explicit_interface_id {
                if explicit == 0 {
                    return Err(CodegenError::validation(format!(
                        "module '{}' explicit interface_id cannot be 0",
                        m.name
                    )));
                }
            } else {
                assert!(iface_id >= NVE_HYBRID_INTERFACE_ID_BASE);
            }
            if let Some(prev) = iface_ids.insert(iface_id, format!("module '{}'", m.name)) {
                return Err(CodegenError::validation(format!(
                    "hybrid interface_id collision 0x{iface_id:08X} between {prev} and module '{}'",
                    m.name
                )));
            }
            validate_methods(
                &m.methods,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("module '{}'", m.name),
            )?;
        }

        for v in &self.views {
            validate_ident("view", &v.name)?;
            validate_ident("view trait", &v.trait_name)?;
            if !item_names.insert(v.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate hybrid module/view name '{}'",
                    v.name
                )));
            }
            if type_names.contains(v.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "hybrid view '{}' collides with a struct or enum of the same name",
                    v.name
                )));
            }
            let iface_id = v.interface_id();
            if let Some(explicit) = v.explicit_interface_id {
                if explicit == 0 {
                    return Err(CodegenError::validation(format!(
                        "view '{}' explicit interface_id cannot be 0",
                        v.name
                    )));
                }
            } else {
                assert!(iface_id >= NVE_HYBRID_INTERFACE_ID_BASE);
            }
            if let Some(prev) = iface_ids.insert(iface_id, format!("view '{}'", v.name)) {
                return Err(CodegenError::validation(format!(
                    "hybrid interface_id collision 0x{iface_id:08X} between {prev} and view '{}'",
                    v.name
                )));
            }

            let mut member_names = BTreeSet::new();
            for p in &v.props {
                validate_ident("prop", &p.name)?;
                if !member_names.insert(p.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        p.name, v.name
                    )));
                }
                validate_type(
                    &p.ty,
                    false,
                    &enum_map,
                    &struct_map,
                    &alias_names,
                    &format!("view '{}' prop '{}'", v.name, p.name),
                )?;
                if let Some(def) = &p.default_value {
                    validate_default_value(def, &p.ty, &enum_map, &v.name, &p.name)?;
                }
            }
            for ev in &v.events {
                validate_ident("event", &ev.name)?;
                if !member_names.insert(ev.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        ev.name, v.name
                    )));
                }
                let mut param_names = BTreeSet::new();
                for param in &ev.params {
                    validate_ident("event parameter", &param.name)?;
                    if !param_names.insert(param.name.as_str()) {
                        return Err(CodegenError::validation(format!(
                            "duplicate parameter '{}' in view '{}' event '{}'",
                            param.name, v.name, ev.name
                        )));
                    }
                    validate_type(
                        &param.ty,
                        false,
                        &enum_map,
                        &struct_map,
                        &alias_names,
                        &format!("view '{}' event '{}' param '{}'", v.name, ev.name, param.name),
                    )?;
                }
            }
            for m in &v.methods {
                validate_ident("method", &m.name)?;
                if !member_names.insert(m.name.as_str()) {
                    return Err(CodegenError::validation(format!(
                        "duplicate member '{}' in view '{}'",
                        m.name, v.name
                    )));
                }
            }
            validate_methods(
                &v.methods,
                &enum_map,
                &struct_map,
                &alias_names,
                &format!("view '{}'", v.name),
            )?;
        }

        if !self.c_functions.is_empty() {
            validate_methods(
                &self.c_functions,
                &enum_map,
                &struct_map,
                &alias_names,
                "c_functions",
            )?;
        }

        Ok(())
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

fn validate_ident(kind: &str, name: &str) -> Result<(), CodegenError> {
    if name.is_empty() {
        return Err(CodegenError::validation(format!("{kind} name cannot be empty")));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}': must start with ASCII letter or '_'"
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(CodegenError::validation(format!(
            "invalid {kind} identifier '{name}': must contain only ASCII alphanumeric or '_'"
        )));
    }
    Ok(())
}

fn validate_methods(
    methods: &[HybridMethodSpec],
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
    alias_names: &BTreeSet<&str>,
    owner: &str,
) -> Result<(), CodegenError> {
    let mut names = BTreeSet::new();
    for m in methods {
        validate_ident("method", &m.name)?;
        if !names.insert(m.name.as_str()) {
            return Err(CodegenError::validation(format!(
                "duplicate method '{}' in {owner}",
                m.name
            )));
        }
        let mut param_names = BTreeSet::new();
        for p in &m.params {
            validate_ident("parameter", &p.name)?;
            if !param_names.insert(p.name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "duplicate parameter '{}' in {owner} method '{}'",
                    p.name, m.name
                )));
            }
            validate_type(
                &p.ty,
                false,
                enum_map,
                struct_map,
                alias_names,
                &format!("{owner} method '{}' param '{}'", m.name, p.name),
            )?;
            if p.ty.is_result() {
                return Err(CodegenError::validation(format!(
                    "{owner} method '{}' param '{}' cannot use Result<T, E> as input parameter",
                    m.name, p.name
                )));
            }
        }
        validate_type(
            &m.return_type,
            true,
            enum_map,
            struct_map,
            alias_names,
            &format!("{owner} method '{}' return type", m.name),
        )?;
    }
    Ok(())
}

fn validate_type(
    ty: &HybridType,
    allow_void: bool,
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
    alias_names: &BTreeSet<&str>,
    ctx: &str,
) -> Result<(), CodegenError> {
    match ty {
        HybridType::Void => {
            if !allow_void {
                return Err(CodegenError::validation(format!(
                    "{ctx}: void/'()' is only permitted as a method return or Result<(), String> ok type"
                )));
            }
            Ok(())
        }
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64
        | HybridType::String
        | HybridType::Bytes
        | HybridType::Value => Ok(()),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            validate_type(inner, true, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Handle(name) => {
            if !alias_names.contains(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown handle alias '{name}'"
                )));
            }
            Ok(())
        }
        HybridType::Optional(inner) => {
            if matches!(inner.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Option<Result<...>> is not supported across FFI"
                )));
            }
            validate_type(inner, false, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Vec(inner) => {
            if matches!(inner.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Vec<Result<...>> is not supported across FFI"
                )));
            }
            validate_type(inner, false, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Result { ok, err } => {
            if !allow_void {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Result<T, String> is only supported as a top-level method return type"
                )));
            }
            if matches!(ok.as_ref(), HybridType::Result { .. }) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: nested Result<Result<...>> is not supported"
                )));
            }
            if !matches!(err.as_ref(), HybridType::String) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: Result error type must be String (found '{}')",
                    err.canonical_signature()
                )));
            }
            validate_type(ok, true, enum_map, struct_map, alias_names, ctx)
        }
        HybridType::Struct(name) => {
            if !struct_map.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown struct or type '{name}'"
                )));
            }
            Ok(())
        }
        HybridType::Enum(name) => {
            if !enum_map.contains_key(name.as_str()) {
                return Err(CodegenError::validation(format!(
                    "{ctx}: unknown enum '{name}'"
                )));
            }
            Ok(())
        }
    }
}

fn validate_default_value(
    def: &HybridDefaultValue,
    ty: &HybridType,
    enum_map: &BTreeMap<&str, &HybridEnumSpec>,
    view_name: &str,
    prop_name: &str,
) -> Result<(), CodegenError> {
    let ok = match (def, ty) {
        (HybridDefaultValue::None, HybridType::Optional(_)) => true,
        (other, HybridType::Optional(inner)) => {
            return validate_default_value(other, inner, enum_map, view_name, prop_name);
        }
        (HybridDefaultValue::Bool(_), HybridType::Bool) => true,
        (
            HybridDefaultValue::Int(_),
            HybridType::I32 | HybridType::I64 | HybridType::F32 | HybridType::F64,
        ) => true,
        (
            HybridDefaultValue::Int(v),
            HybridType::U32 | HybridType::U64,
        ) => *v >= 0,
        (
            HybridDefaultValue::UInt(_),
            HybridType::U32 | HybridType::U64 | HybridType::I32 | HybridType::I64 | HybridType::F32 | HybridType::F64,
        ) => true,
        (HybridDefaultValue::Float(_), HybridType::F32 | HybridType::F64) => true,
        (HybridDefaultValue::String(_), HybridType::String) => true,
        (
            HybridDefaultValue::EnumVariant { enum_name, variant },
            HybridType::Enum(expected_enum),
        ) => {
            enum_name == expected_enum
                && enum_map
                    .get(expected_enum.as_str())
                    .is_some_and(|e| e.variants.iter().any(|v| v.name == *variant))
        }
        _ => false,
    };
    if !ok {
        return Err(CodegenError::validation(format!(
            "view '{view_name}' prop '{prop_name}': default value {:?} does not match type '{}'",
            def,
            ty.canonical_signature()
        )));
    }
    Ok(())
}

/// Rejects cyclic value-embedded structs (`struct A { b: B }`, `struct B { a: A }`, or through
/// `Option<B>`), which have infinite size in Rust and C11/C++20.
fn detect_recursive_value_structs(
    struct_map: &BTreeMap<&str, &HybridStructSpec>,
) -> Result<(), CodegenError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Visiting,
        Visited,
    }

    fn dfs<'a>(
        name: &'a str,
        struct_map: &BTreeMap<&'a str, &'a HybridStructSpec>,
        marks: &mut BTreeMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
    ) -> Result<(), CodegenError> {
        if let Some(mark) = marks.get(name) {
            if *mark == Mark::Visiting {
                stack.push(name);
                return Err(CodegenError::validation(format!(
                    "recursive infinite-size value struct cycle detected: {}",
                    stack.join(" -> ")
                )));
            }
            return Ok(());
        }
        marks.insert(name, Mark::Visiting);
        stack.push(name);
        if let Some(spec) = struct_map.get(name) {
            for field in &spec.fields {
                collect_direct_struct_edges(&field.ty, &mut |target| {
                    dfs(target, struct_map, marks, stack)
                })?;
            }
        }
        stack.pop();
        marks.insert(name, Mark::Visited);
        Ok(())
    }

    fn collect_direct_struct_edges<'a, F>(ty: &'a HybridType, f: &mut F) -> Result<(), CodegenError>
    where
        F: FnMut(&'a str) -> Result<(), CodegenError>,
    {
        match ty {
            HybridType::Struct(name) => f(name.as_str()),
            HybridType::Optional(inner) => collect_direct_struct_edges(inner, f),
            // `Vec<T>` introduces heap indirection and breaks direct value-size cycles.
            _ => Ok(()),
        }
    }

    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for &name in struct_map.keys() {
        dfs(name, struct_map, &mut marks, &mut stack)?;
    }
    Ok(())
}

fn hash_str(mut h: u64, s: &str) -> u64 {
    h = fnv1a_64_update(h, &(s.len() as u32).to_le_bytes());
    fnv1a_64_update(h, s.as_bytes())
}

fn hash_method(
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

fn hash_transitive_types(
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
