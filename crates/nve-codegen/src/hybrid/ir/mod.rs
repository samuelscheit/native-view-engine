//! Intermediate Representation (IR), deterministic FNV-1a schema hashing, interface ID
//! assignment, and strict semantic validation for NVE's Nitro-inspired Rust-first Hybrid FFI
//! code generator.

mod file_spec;
mod validate;

pub use file_spec::HybridFileSpec;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::CodegenError;
use crate::naming::{to_pascal_case, to_snake_case};
use file_spec::{hash_method, hash_str, hash_transitive_types};

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
    FixedArray {
        elem: Box<HybridType>,
        len: usize,
    },
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
            Self::FixedArray { elem, len } => format!("[{};{len}]", elem.canonical_signature()),
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
            Self::FixedArray { elem, .. } => elem.is_pod(file),
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
            | Self::MutPtr(inner)
            | Self::FixedArray { elem: inner, .. } => inner.collect_refs(structs, enums),
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
                HybridType::FixedArray { elem, .. } => elem.is_pod(file),
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
