//! Unified NVE Core ABI + Scene ABI generator dogfooding the Nitro-style Rust-first
//! `hybrid` parser & IR (`crates/nve-codegen/specs/nve_core_abi.nve.rs` and
//! `crates/nve-codegen/specs/nve_scene_abi.nve.rs`).
//!
//! Emits all 10 checked-in ABI artifacts from ONE pipeline:
//!
//! **Core ABI (5 files):**
//! 1. `cxx/nve-fabric-bridge/include/nve_abi.generated.h`
//! 2. `hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.generated.h`
//! 3. `hosts/flutter-plugin/lib/src/nve_core_abi.generated.dart`
//! 4. `hosts/android-shell/android-app/src/dev/nve/android/generated/NveCoreAbi.java`
//! 5. `hosts/web-shell/nve_core_abi.generated.ts`
//!
//! **Scene ABI (5 files):**
//! 6. `cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h`
//! 7. `hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.generated.h`
//! 8. `hosts/flutter-plugin/lib/src/nve_scene_abi.generated.dart`
//! 9. `hosts/android-shell/android-app/src/dev/nve/android/generated/NveSceneBuffer.java`
//! 10. `hosts/web-shell/nve_scene_view.generated.ts`

mod c_emitter;
mod dart_emitter;
mod java_emitter;
mod ts_emitter;

pub use c_emitter::{generate_c_core_abi_h, generate_c_scene_abi_h};
pub use dart_emitter::{generate_dart_core_abi, generate_dart_scene_abi};
pub use java_emitter::{generate_java_core_abi, generate_java_scene_buffer};
pub use ts_emitter::{generate_ts_core_abi, generate_ts_scene_view};

use crate::hybrid::ir::{
    fnv1a_64_update, HybridEnumSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec,
    HybridStructSpec, HybridType, FNV1A_64_OFFSET_BASIS,
};
use crate::hybrid::parser::parse_hybrid_rust_spec;
use crate::ir::GeneratedFile;
use crate::naming::{to_camel_case, to_pascal_case, to_snake_case};

/// Embedded declarative Rust spec for NVE's Core ABI.
pub const NVE_CORE_ABI_SPEC_SOURCE: &str = include_str!("../../../specs/nve_core_abi.nve.rs");

/// Embedded declarative Rust spec for NVE's 64-byte Scene ABI.
pub const NVE_SCENE_ABI_SPEC_SOURCE: &str = include_str!("../../../specs/nve_scene_abi.nve.rs");

/// Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).
pub const NVE_RENDER_NODE_SIZE: usize = 64;

/// Required cache-line alignment of `NveRenderNode`.
pub const NVE_RENDER_NODE_ALIGN: usize = 64;

/// Deterministic FNV-1a 64-bit Core ABI schema hash (must match `crates/nve-abi/src/ffi.rs`
/// and `core_abi_schema_hash()` verbatim).
pub const NVE_CORE_ABI_SCHEMA_HASH: u64 = 0x5BE5_D06C_C7C0_1FEF;

/// Deterministic FNV-1a 64-bit scene schema hash (must match `crates/nve-abi/src/scene.rs`
/// and `compute_scene_schema_hash(&scene_abi_spec())` verbatim).
pub const NVE_SCENE_SCHEMA_HASH: u64 = 0x00F2_CF03_7DB5_0CC8;

/// Parses and validates `crates/nve-codegen/specs/nve_core_abi.nve.rs` via the unified
/// `hybrid` Rust spec parser.
pub fn core_abi_spec() -> HybridFileSpec {
    parse_hybrid_rust_spec(NVE_CORE_ABI_SPEC_SOURCE)
        .expect("embedded specs/nve_core_abi.nve.rs must parse and validate cleanly")
}

/// Parses and validates `crates/nve-codegen/specs/nve_scene_abi.nve.rs` via the unified
/// `hybrid` Rust spec parser.
pub fn scene_abi_spec() -> HybridFileSpec {
    parse_hybrid_rust_spec(NVE_SCENE_ABI_SPEC_SOURCE)
        .expect("embedded specs/nve_scene_abi.nve.rs must parse and validate cleanly")
}

/// Deterministic FNV-1a 64-bit schema hash computed from the parsed Core ABI `HybridFileSpec`.
pub fn core_abi_schema_hash() -> u64 {
    core_abi_spec().file_schema_hash()
}

/// Computes the exact FNV-1a 64-bit Scene ABI schema hash from the parsed `nve_scene_abi.nve.rs`
/// `HybridFileSpec` using the identical byte sequence verified by `crates/nve-abi/src/scene.rs`.
pub fn compute_scene_schema_hash(spec: &HybridFileSpec) -> u64 {
    let render_node = spec
        .find_struct("NveRenderNode")
        .expect("nve_scene_abi.nve.rs must declare NveRenderNode");
    let scene_view = spec
        .find_struct("NveSceneView")
        .expect("nve_scene_abi.nve.rs must declare NveSceneView");
    let node_size = render_node
        .expected_size
        .expect("NveRenderNode must declare size = 64");

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

// ============================================================================
// Shared Low-Level Type Formatters
// ============================================================================

fn c_core_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "void".to_string(),
        HybridType::Bool => "bool".to_string(),
        HybridType::U8 => "uint8_t".to_string(),
        HybridType::U16 => "uint16_t".to_string(),
        HybridType::Usize => "size_t".to_string(),
        HybridType::I32 => "int32_t".to_string(),
        HybridType::U32 => "uint32_t".to_string(),
        HybridType::I64 => "int64_t".to_string(),
        HybridType::U64 => "uint64_t".to_string(),
        HybridType::F32 => "float".to_string(),
        HybridType::F64 => "double".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) | HybridType::Enum(name) | HybridType::Handle(name) => {
            name.clone()
        }
        HybridType::ConstPtr(inner) => format!("const {}*", c_core_type(inner)),
        HybridType::MutPtr(inner) => format!("{}*", c_core_type(inner)),
        other => unreachable!("unsupported core ABI C type: {other:?}"),
    }
}

fn dart_core_ffi_annotation(ty: &HybridType) -> Option<&'static str> {
    match ty {
        HybridType::Bool => Some("@ffi.Bool()"),
        HybridType::U8 => Some("@ffi.Uint8()"),
        HybridType::U16 => Some("@ffi.Uint16()"),
        HybridType::Usize => Some("@ffi.Size()"),
        HybridType::I32 => Some("@ffi.Int32()"),
        HybridType::U32 => Some("@ffi.Uint32()"),
        HybridType::I64 => Some("@ffi.Int64()"),
        HybridType::U64 => Some("@ffi.Uint64()"),
        HybridType::F32 => Some("@ffi.Float()"),
        HybridType::F64 => Some("@ffi.Double()"),
        _ => None,
    }
}

/// Signed status codes shared by `NveHostCallbacks::invoke`, `channel_publish`, and `NVH1`.
const HOST_STATUS_VARIANTS: &[(&str, i32)] = &[
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

fn dart_core_native_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Void => "ffi.Void".to_string(),
        HybridType::Bool => "ffi.Bool".to_string(),
        HybridType::U8 => "ffi.Uint8".to_string(),
        HybridType::U16 => "ffi.Uint16".to_string(),
        HybridType::Usize => "ffi.Size".to_string(),
        HybridType::I32 => "ffi.Int32".to_string(),
        HybridType::U32 => "ffi.Uint32".to_string(),
        HybridType::I64 => "ffi.Int64".to_string(),
        HybridType::U64 => "ffi.Uint64".to_string(),
        HybridType::F32 => "ffi.Float".to_string(),
        HybridType::F64 => "ffi.Double".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) => name.clone(),
        HybridType::Handle(name) if name == "NveHostInvokeFn" => {
            "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_string()
        }
        HybridType::Handle(name) if name == "NveHostChannelPublishFn" => {
            "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_string()
        }
        HybridType::Handle(_) => "ffi.Pointer<ffi.Void>".to_string(),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            format!("ffi.Pointer<{}>", dart_core_native_type(inner))
        }
        other => unreachable!("unsupported core ABI Dart native type: {other:?}"),
    }
}

fn dart_core_struct_field_type(ty: &HybridType) -> String {
    match ty {
        HybridType::Bool => "bool".to_string(),
        HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64 => "int".to_string(),
        HybridType::F32 | HybridType::F64 => "double".to_string(),
        HybridType::Value => "NveValue".to_string(),
        HybridType::Struct(name) => name.clone(),
        HybridType::Handle(name) if name == "NveHostInvokeFn" => {
            "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_string()
        }
        HybridType::Handle(name) if name == "NveHostChannelPublishFn" => {
            "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_string()
        }
        HybridType::Handle(_) => "ffi.Pointer<ffi.Void>".to_string(),
        HybridType::ConstPtr(inner) | HybridType::MutPtr(inner) => {
            format!("ffi.Pointer<{}>", dart_core_native_type(inner))
        }
        other => unreachable!("unsupported core ABI Dart struct field type: {other:?}"),
    }
}

fn snake_to_camel_preserve_leading(s: &str) -> String {
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

fn snake_to_screaming(s: &str) -> String {
    s.trim_start_matches('_').to_ascii_uppercase()
}

fn dart_enum_variant_ident(variant_name: &str) -> String {
    let camel = to_camel_case(variant_name);
    match camel.as_str() {
        "null" => "nullValue".to_string(),
        "bool" => "boolValue".to_string(),
        "default" => "defaultValue".to_string(),
        _ => camel,
    }
}

fn format_enum_value(v: u32) -> String {
    if v >= 0x1000 {
        format!("0x{v:X}")
    } else {
        v.to_string()
    }
}

fn format_c_enum_prefix(e: &HybridEnumSpec) -> String {
    e.c_prefix
        .clone()
        .unwrap_or_else(|| format!("{}_", to_snake_case(&e.name).to_ascii_uppercase()))
}

fn module_c_struct_name(m: &HybridModuleSpec) -> String {
    m.c_name
        .clone()
        .unwrap_or_else(|| format!("Nve{}Api", to_pascal_case(&m.name)))
}


// ============================================================================
// Unified File Bundle Entrypoints (5 Core + 5 Scene = 10 Checked-in Artifacts)
// ============================================================================

/// Generates all 5 checked-in Core ABI files keyed by workspace-relative path.
pub fn generate_all_core_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_core_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_abi.generated.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.generated.h"
                .to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_core_abi.generated.dart".to_owned(),
            content: generate_dart_core_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveCoreAbi.java"
                    .to_owned(),
            content: generate_java_core_abi(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_core_abi.generated.ts".to_owned(),
            content: generate_ts_core_abi(),
        },
    ]
}

/// Generates all 5 checked-in Scene ABI files keyed by workspace-relative path.
pub fn generate_all_scene_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_scene_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.generated.h"
                .to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_scene_abi.generated.dart".to_owned(),
            content: generate_dart_scene_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/generated/NveSceneBuffer.java"
                    .to_owned(),
            content: generate_java_scene_buffer(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_scene_view.generated.ts".to_owned(),
            content: generate_ts_scene_view(),
        },
    ]
}

/// Generates ALL 10 checked-in Core + Scene ABI artifacts from the unified `hybrid` pipeline.
pub fn generate_all_abi_files() -> Vec<GeneratedFile> {
    let mut files = generate_all_core_abi_files();
    files.extend(generate_all_scene_abi_files());
    files
}
