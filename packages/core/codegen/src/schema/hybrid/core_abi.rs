//! Unified NVE Core ABI + Scene ABI schema specs and deterministic hashes consuming
//! single-source-of-truth colocated `#[derive(NveAbi)]` descriptors directly from
//! `packages/core/abi/src/schema.rs`.

use super::schema_bridge::from_abi_descriptor;
use super::{
    fnv1a_64_update, HybridEnumSpec, HybridFileSpec, HybridModuleSpec, HybridType,
    FNV1A_64_OFFSET_BASIS,
};
use crate::naming::{to_pascal_case, to_snake_case};

/// Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).
pub const NVE_RENDER_NODE_SIZE: usize = nve_abi::NVE_RENDER_NODE_SIZE;

/// Required cache-line alignment of `NveRenderNode`.
pub const NVE_RENDER_NODE_ALIGN: usize = nve_abi::NVE_RENDER_NODE_ALIGN;

/// Exact byte stride of one fixed-stride `NveNodeMeta` record (64 bytes).
pub const NVE_NODE_META_SIZE: usize = nve_abi::NVE_NODE_META_SIZE;

/// Natural alignment of `NveNodeMeta` (4 bytes).
pub const NVE_NODE_META_ALIGN: usize = nve_abi::NVE_NODE_META_ALIGN;

/// Deterministic FNV-1a 64-bit Core ABI schema hash (must match `packages/core/abi/src/ffi.rs`
/// and `core_abi_schema_hash()` verbatim).
pub const NVE_CORE_ABI_SCHEMA_HASH: u64 = nve_abi::NVE_CORE_ABI_SCHEMA_HASH;

/// Deterministic FNV-1a 64-bit scene schema hash (must match `packages/core/abi/src/scene/types.rs`
/// and `compute_scene_schema_hash(&scene_abi_spec())` verbatim).
pub const NVE_SCENE_SCHEMA_HASH: u64 = nve_abi::NVE_SCENE_SCHEMA_HASH;

/// Builds the validated Core ABI [`HybridFileSpec`] directly from the real compiled
/// `#[derive(NveAbi)]` types in `packages/core/abi/src/schema.rs`.
pub fn core_abi_spec() -> HybridFileSpec {
    from_abi_descriptor(&nve_abi::schema::core_abi_descriptor())
}

/// Builds the validated Scene ABI [`HybridFileSpec`] directly from the real compiled
/// `NveRenderNode` and `NveSceneView` types in `packages/core/abi/src/schema.rs`.
pub fn scene_abi_spec() -> HybridFileSpec {
    from_abi_descriptor(&nve_abi::schema::scene_abi_descriptor())
}

/// Deterministic FNV-1a 64-bit schema hash computed from the colocated Core ABI `HybridFileSpec`.
pub fn core_abi_schema_hash() -> u64 {
    core_abi_spec().file_schema_hash()
}

/// Computes the exact FNV-1a 64-bit Scene ABI schema hash from the colocated Scene ABI
/// `HybridFileSpec` using the identical byte sequence verified by `packages/core/abi/src/scene/types.rs`.
pub fn compute_scene_schema_hash(spec: &HybridFileSpec) -> u64 {
    let render_node = spec
        .find_struct("NveRenderNode")
        .expect("scene_abi_descriptor must declare NveRenderNode");
    let scene_view = spec
        .find_struct("NveSceneView")
        .expect("scene_abi_descriptor must declare NveSceneView");
    let node_meta = spec
        .find_struct("NveNodeMeta")
        .expect("scene_abi_descriptor must declare NveNodeMeta");
    let node_size = render_node
        .expected_size
        .expect("NveRenderNode must declare size = 64");
    let meta_size = node_meta
        .expected_size
        .expect("NveNodeMeta must declare size = 64");

    let mut h = FNV1A_64_OFFSET_BASIS;
    h = fnv1a_64_update(h, b"NveRenderNode");
    h = fnv1a_64_update(h, &(node_size as u64).to_le_bytes());

    for f in &render_node.fields {
        h = fnv1a_64_update(h, b"|");
        h = fnv1a_64_update(h, f.name.as_bytes());
        h = fnv1a_64_update(h, b":");
        h = fnv1a_64_update(h, scene_canonical_type_name(&f.ty).as_bytes());
        h = fnv1a_64_update(h, b"@");
        let off = f
            .offset
            .expect("NveRenderNode fields must declare explicit #[offset(N)]");
        h = fnv1a_64_update(h, &(off as u64).to_le_bytes());
    }

    h = fnv1a_64_update(h, b";NveSceneView");
    for (idx, f) in scene_view.fields.iter().enumerate() {
        h = fnv1a_64_update(h, b"|");
        h = fnv1a_64_update(h, f.name.as_bytes());
        h = fnv1a_64_update(h, b":");
        h = fnv1a_64_update(h, scene_canonical_type_name(&f.ty).as_bytes());
        if let Some(off) = f.offset {
            h = fnv1a_64_update(h, b"@");
            h = fnv1a_64_update(h, &(off as u64).to_le_bytes());
        } else {
            h = fnv1a_64_update(h, b"#");
            h = fnv1a_64_update(h, &(idx as u64).to_le_bytes());
        }
    }

    h = fnv1a_64_update(h, b";NveNodeMeta");
    h = fnv1a_64_update(h, &(meta_size as u64).to_le_bytes());
    for f in &node_meta.fields {
        h = fnv1a_64_update(h, b"|");
        h = fnv1a_64_update(h, f.name.as_bytes());
        h = fnv1a_64_update(h, b":");
        h = fnv1a_64_update(h, scene_canonical_type_name(&f.ty).as_bytes());
        h = fnv1a_64_update(h, b"@");
        let off = f
            .offset
            .expect("NveNodeMeta fields must declare explicit #[offset(N)]");
        h = fnv1a_64_update(h, &(off as u64).to_le_bytes());
    }
    h
}

fn scene_canonical_type_name(ty: &HybridType) -> String {
    match ty {
        HybridType::U8 => "u8".to_string(),
        HybridType::U16 => "u16".to_string(),
        HybridType::U32 => "u32".to_string(),
        HybridType::I32 => "i32".to_string(),
        HybridType::U64 => "u64".to_string(),
        HybridType::F32 => "f32".to_string(),
        HybridType::ConstPtr(inner) => match inner.as_ref() {
            HybridType::Struct(name) => format!("*const {name}"),
            other => format!("*const {}", scene_canonical_type_name(other)),
        },
        other => other.canonical_signature(),
    }
}

/// Signed status codes shared by `NveHostCallbacks::invoke` and `channel_publish`.
pub(crate) const HOST_STATUS_VARIANTS: &[(&str, i32)] = &[
    ("Ok", 0),
    ("Pending", 1),
    ("BufferTooSmall", -1),
    ("Unsupported", -2),
    ("PermissionDenied", -3),
    ("InvalidArg", -4),
    ("NotFound", -5),
    ("Cancelled", -6),
    ("IoError", -7),
    ("InternalError", -8),
];

pub(crate) fn snake_to_camel_preserve_leading(s: &str) -> String {
    let trimmed = s.trim_start_matches('_');
    let pascal = to_pascal_case(trimmed);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::with_capacity(pascal.len());
            out.push(first.to_ascii_lowercase());
            out.push_str(chars.as_str());
            if s.starts_with('_') {
                format!("_{out}")
            } else {
                out
            }
        }
        None => s.to_owned(),
    }
}

pub(crate) fn snake_to_screaming(s: &str) -> String {
    s.trim_start_matches('_').to_ascii_uppercase()
}

pub(crate) fn format_enum_value(v: u32) -> String {
    if v >= 0x1000 {
        format!("0x{v:X}")
    } else {
        v.to_string()
    }
}

pub(crate) fn format_c_enum_prefix(e: &HybridEnumSpec) -> String {
    e.c_prefix
        .clone()
        .unwrap_or_else(|| format!("{}_", to_snake_case(&e.name).to_ascii_uppercase()))
}

pub(crate) fn module_c_struct_name(m: &HybridModuleSpec) -> String {
    m.c_name
        .clone()
        .unwrap_or_else(|| format!("Nve{}Api", to_pascal_case(&m.name)))
}
