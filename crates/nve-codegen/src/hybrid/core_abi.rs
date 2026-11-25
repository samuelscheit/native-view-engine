//! Unified NVE Core ABI + Scene ABI generator dogfooding the Nitro-style Rust-first
//! `hybrid` parser & IR (`crates/nve-codegen/specs/nve_core_abi.nve.rs` and
//! `crates/nve-codegen/specs/nve_scene_abi.nve.rs`).
//!
//! Emits all 10 checked-in ABI artifacts from ONE pipeline:
//!
//! **Core ABI (5 files):**
//! 1. `cxx/nve-fabric-bridge/include/nve_abi.h`
//! 2. `hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.h`
//! 3. `hosts/flutter-plugin/lib/src/nve_core_abi_gen.dart`
//! 4. `hosts/android-shell/android-app/src/dev/nve/android/NveCoreAbi.java`
//! 5. `hosts/web-shell/nve_core_abi.ts`
//!
//! **Scene ABI (5 files):**
//! 6. `cxx/nve-fabric-bridge/include/nve_scene_abi.h`
//! 7. `hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.h`
//! 8. `hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart`
//! 9. `hosts/android-shell/android-app/src/dev/nve/android/NveSceneBuffer.java`
//! 10. `hosts/web-shell/nve_scene_view.ts`

use std::fmt::Write as _;

use crate::hybrid::ir::{
    fnv1a_64_update, HybridEnumSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec,
    HybridStructSpec, HybridType, FNV1A_64_OFFSET_BASIS,
};
use crate::hybrid::parser::parse_hybrid_rust_spec;
use crate::ir::GeneratedFile;
use crate::naming::{to_camel_case, to_pascal_case, to_snake_case};

/// Embedded declarative Rust spec for NVE's Core ABI.
pub const NVE_CORE_ABI_SPEC_SOURCE: &str = include_str!("../../specs/nve_core_abi.nve.rs");

/// Embedded declarative Rust spec for NVE's 64-byte Scene ABI.
pub const NVE_SCENE_ABI_SPEC_SOURCE: &str = include_str!("../../specs/nve_scene_abi.nve.rs");

/// Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).
pub const NVE_RENDER_NODE_SIZE: usize = 64;

/// Required cache-line alignment of `NveRenderNode`.
pub const NVE_RENDER_NODE_ALIGN: usize = 64;

/// Deterministic FNV-1a 64-bit Core ABI schema hash (must match `crates/nve-abi/src/ffi.rs`
/// and `core_abi_schema_hash()` verbatim).
pub const NVE_CORE_ABI_SCHEMA_HASH: u64 = 0x54A4_A32B_6F25_8107;

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
// Core ABI Target 1 & 2: C11 / C++20 / Swift (`nve_abi.h`)
// ============================================================================

/// Generates `nve_abi.h` (used identically by `cxx/nve-fabric-bridge/include/nve_abi.h`
/// and `hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.h`) directly from
/// `crates/nve-codegen/specs/nve_core_abi.nve.rs`.
pub fn generate_c_core_abi_h() -> String {
    let spec = core_abi_spec();
    let schema_hash = spec.file_schema_hash();
    let mut out = String::with_capacity(8 * 1024);

    let _ = writeln!(
        out,
        "/* @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY. */"
    );
    let _ = writeln!(
        out,
        "/* Single-source NVE Core C ABI generated from crates/nve-codegen/specs/nve_core_abi.nve.rs. */"
    );
    let _ = writeln!(out, "#pragma once");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <stddef.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out);
    let _ = writeln!(out, "#ifdef __cplusplus");
    let _ = writeln!(out, "extern \"C\" {{");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);
    let _ = writeln!(out, "#ifndef NVE_API");
    let _ = writeln!(out, "#if defined(NVE_BUILD_SHARED_LIB)");
    let _ = writeln!(out, "#if defined(_WIN32)");
    let _ = writeln!(out, "#define NVE_API __declspec(dllexport)");
    let _ = writeln!(out, "#elif defined(__GNUC__) || defined(__clang__)");
    let _ = writeln!(out, "#define NVE_API __attribute__((visibility(\"default\")))");
    let _ = writeln!(out, "#else");
    let _ = writeln!(out, "#define NVE_API");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out, "#else");
    let _ = writeln!(out, "#define NVE_API");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "#define NVE_CORE_ABI_SCHEMA_HASH 0x{schema_hash:016X}ULL"
    );
    for c in &spec.consts {
        if c.name == "NVE_BIN_VALUE_MAX_DEPTH" {
            continue;
        }
        if c.hex {
            let _ = writeln!(out, "#define {} 0x{:X}u", c.name, c.value);
        } else {
            let _ = writeln!(out, "#define {} {}u", c.name, c.value);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/* Modular interface IDs resolved once via `nve_get_interface(uint32_t)`. */"
    );
    for m in &spec.modules {
        let upper = to_snake_case(&m.name).to_ascii_uppercase();
        let c_name = module_c_struct_name(m);
        let _ = writeln!(
            out,
            "#define NVE_IFACE_{upper} {}u /* const {c_name}* */",
            m.interface_id()
        );
    }
    let _ = writeln!(out);

    // Core enums
    for e in &spec.enums {
        let prefix = format_c_enum_prefix(e);
        let _ = writeln!(out, "typedef enum {} {{", e.name);
        for v in &e.variants {
            let v_upper = to_snake_case(&v.name).to_ascii_uppercase();
            let _ = writeln!(
                out,
                "  {prefix}{v_upper} = {},",
                format_enum_value(v.value)
            );
        }
        let _ = writeln!(out, "}} {};", e.name);
        let _ = writeln!(out);
    }

    // Signed host-capability status enum (`NVE_IFACE_HOST = 5`)
    let _ = writeln!(out, "typedef enum NveHostStatus {{");
    for &(variant, code) in HOST_STATUS_VARIANTS {
        let v_upper = to_snake_case(variant).to_ascii_uppercase();
        let _ = writeln!(out, "  NVE_HOST_{v_upper} = {code},");
    }
    let _ = writeln!(out, "}} NveHostStatus;");
    let _ = writeln!(out);

    if let Some(depth_const) = spec.consts.iter().find(|c| c.name == "NVE_BIN_VALUE_MAX_DEPTH") {
        let _ = writeln!(
            out,
            "#define {} {}u",
            depth_const.name, depth_const.value
        );
        let _ = writeln!(out);
    }

    // Forward declarations for mutually referencing value structs
    let _ = writeln!(out, "typedef struct NveValue NveValue;");
    let _ = writeln!(out, "typedef struct NveMapEntry NveMapEntry;");
    let _ = writeln!(out);

    // Structs and unions in declaration order
    for s in &spec.structs {
        if s.name == "NveHostCallbacks" {
            let _ = writeln!(
                out,
                "typedef int32_t (*NveHostInvokeFn)(void* host_ctx, uint32_t service_id, uint32_t method_id, uint64_t request_id, NveBufferSlice request, uint8_t* out_buf, size_t out_cap, size_t* out_len);"
            );
            let _ = writeln!(
                out,
                "typedef int32_t (*NveHostChannelPublishFn)(void* host_ctx, uint32_t channel_id, uint32_t flags, NveBufferSlice payload);"
            );
            let _ = writeln!(out);
        }
        emit_c_core_struct_or_union(&mut out, s);
    }

    // Compile-time layout & offset assertions
    let _ = writeln!(out, "#ifndef NVE_STATIC_ASSERT");
    let _ = writeln!(out, "#if defined(__cplusplus)");
    let _ = writeln!(out, "#define NVE_STATIC_ASSERT(cond, msg) static_assert((cond), msg)");
    let _ = writeln!(
        out,
        "#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L"
    );
    let _ = writeln!(
        out,
        "#define NVE_STATIC_ASSERT(cond, msg) _Static_assert((cond), msg)"
    );
    let _ = writeln!(out, "#else");
    let _ = writeln!(out, "#define NVE_STATIC_ASSERT(cond, msg)");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);

    for s in &spec.structs {
        if let Some(sz) = s.expected_size {
            let _ = writeln!(
                out,
                "NVE_STATIC_ASSERT(sizeof({}) == {sz}u, \"{} size must be {sz}\");",
                s.name, s.name
            );
        }
        if let Some(sz64) = s.expected_size_64 {
            let _ = writeln!(
                out,
                "NVE_STATIC_ASSERT(sizeof(void*) != 8 || sizeof({}) == {sz64}u, \"{} size must be {sz64} on 64-bit\");",
                s.name, s.name
            );
        }
        for f in &s.fields {
            if let Some(off) = f.offset {
                let _ = writeln!(
                    out,
                    "NVE_STATIC_ASSERT(offsetof({}, {}) == {off}u, \"{}::{} offset must be {off}\");",
                    s.name, f.name, s.name, f.name
                );
            }
            if let Some(off64) = f.offset_64 {
                let _ = writeln!(
                    out,
                    "NVE_STATIC_ASSERT(sizeof(void*) != 8 || offsetof({}, {}) == {off64}u, \"{}::{} offset must be {off64}\");",
                    s.name, f.name, s.name, f.name
                );
            }
        }
    }
    let _ = writeln!(out);

    // Handle aliases
    for a in &spec.aliases {
        if matches!(
            a.name.as_str(),
            "NveHostInvokeFn" | "NveHostChannelPublishFn"
        ) {
            continue;
        }
        let _ = writeln!(out, "typedef {} {};", c_core_type(&a.target), a.name);
    }
    let _ = writeln!(out);

    // Modular interface vtables
    for m in &spec.modules {
        let c_name = module_c_struct_name(m);
        if let Some(doc) = &m.doc {
            let _ = writeln!(out, "/* {} */", doc.replace('\n', " "));
        }
        let _ = writeln!(out, "typedef struct {c_name} {{");
        for method in &m.methods {
            if let Some(doc) = &method.doc {
                let _ = writeln!(out, "  /* {} */", doc.replace('\n', " "));
            }
            let ret = c_core_type(&method.return_type);
            let mut params = Vec::new();
            if method.has_self {
                params.push("NveAppHandle handle".to_string());
            }
            for p in &method.params {
                params.push(format!("{} {}", c_core_type(&p.ty), p.name));
            }
            let param_list = if params.is_empty() {
                "void".to_string()
            } else {
                params.join(", ")
            };
            let _ = writeln!(out, "  {ret} (*{})({param_list});", method.name);
        }
        let _ = writeln!(out, "}} {c_name};");
        let _ = writeln!(out);
    }

    // Single exported symbol + NveModule resolver
    let _ = writeln!(
        out,
        "/* Single exported symbol per NVE module (`nve_get_interface`). */"
    );
    let _ = writeln!(
        out,
        "typedef const void* (*NveGetInterfaceFn)(uint32_t interface_id);"
    );
    let _ = writeln!(out, "NVE_API const void* nve_get_interface(uint32_t interface_id);");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/* Host-side resolved cache of modular interface pointers (not an ABI struct). */"
    );
    let _ = writeln!(out, "typedef struct NveModule {{");
    for m in &spec.modules {
        let c_name = module_c_struct_name(m);
        let field = to_snake_case(&m.name);
        let _ = writeln!(out, "  const {c_name}* {field};");
    }
    let _ = writeln!(out, "}} NveModule;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "static inline NveModule nve_resolve_module(NveGetInterfaceFn get_iface) {{"
    );
    let nulls = vec!["NULL"; spec.modules.len()].join(", ");
    let _ = writeln!(out, "  NveModule m = {{{nulls}}};");
    let _ = writeln!(out, "  if (!get_iface) {{");
    let _ = writeln!(out, "    return m;");
    let _ = writeln!(out, "  }}");
    for m in &spec.modules {
        let c_name = module_c_struct_name(m);
        let field = to_snake_case(&m.name);
        let upper = field.to_ascii_uppercase();
        let _ = writeln!(
            out,
            "  m.{field} = (const {c_name}*)get_iface(NVE_IFACE_{upper});"
        );
    }
    let _ = writeln!(out, "  return m;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Host C API functions (cxx/nve-fabric-bridge)
    let _ = writeln!(
        out,
        "/* C API implemented by cxx/nve-fabric-bridge for iOS, macOS, Android, and headless Fabric tests. */"
    );
    for f in &spec.c_functions {
        emit_c_function_decl(&mut out, f);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "#ifdef __cplusplus");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out, "#endif");
    out
}

fn emit_c_core_struct_or_union(out: &mut String, s: &HybridStructSpec) {
    let is_forward_typedef = matches!(s.name.as_str(), "NveValue" | "NveMapEntry");
    if s.is_union {
        let _ = writeln!(out, "typedef union {} {{", s.name);
    } else if is_forward_typedef {
        let _ = writeln!(out, "struct {} {{", s.name);
    } else {
        let _ = writeln!(out, "typedef struct {} {{", s.name);
    }
    for f in &s.fields {
        let _ = writeln!(out, "  {} {};", c_core_type(&f.ty), f.name);
    }
    if is_forward_typedef {
        let _ = writeln!(out, "}};");
    } else {
        let _ = writeln!(out, "}} {};", s.name);
    }
    let _ = writeln!(out);
}

fn emit_c_function_decl(out: &mut String, f: &HybridMethodSpec) {
    let ret = c_core_type(&f.return_type);
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{} {}", c_core_type(&p.ty), p.name))
        .collect();
    let param_list = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let _ = writeln!(out, "{ret} {}({param_list});", f.name);
}

// ============================================================================
// Core ABI Target 3: Dart `dart:ffi` (`hosts/flutter-plugin/lib/src/nve_core_abi_gen.dart`)
// ============================================================================

/// Generates `hosts/flutter-plugin/lib/src/nve_core_abi_gen.dart` directly from
/// `crates/nve-codegen/specs/nve_core_abi.nve.rs`.
pub fn generate_dart_core_abi() -> String {
    let spec = core_abi_spec();
    let schema_hash = spec.file_schema_hash();
    let signed_hash = schema_hash as i64;
    let mut out = String::with_capacity(8 * 1024);

    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source NVE Core C ABI `dart:ffi` bindings generated from specs/nve_core_abi.nve.rs."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "import 'dart:ffi' as ffi;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Deterministic FNV-1a 64-bit Core ABI schema hash (`0x{schema_hash:016X}`)."
    );
    let _ = writeln!(out, "const int nveCoreAbiSchemaHash = {signed_hash};");
    let _ = writeln!(
        out,
        "const String nveCoreAbiSchemaHashHex = '0x{schema_hash:016X}';"
    );
    let _ = writeln!(out);

    // Core constants
    for c in &spec.consts {
        let dart_name = to_camel_case(&c.name.to_ascii_lowercase());
        if c.hex {
            let _ = writeln!(out, "const int {dart_name} = 0x{:X};", c.value);
        } else {
            let _ = writeln!(out, "const int {dart_name} = {};", c.value);
        }
    }
    let _ = writeln!(out);

    // Modular interface IDs
    let _ = writeln!(
        out,
        "/// Modular interface IDs passed to `nve_get_interface(uint32_t)`."
    );
    for m in &spec.modules {
        let pascal = to_pascal_case(&m.name);
        let _ = writeln!(out, "const int nveIface{pascal} = {};", m.interface_id());
    }
    let _ = writeln!(out);

    // Core enums
    for e in &spec.enums {
        let _ = writeln!(out, "enum {} {{", e.name);
        for (i, v) in e.variants.iter().enumerate() {
            let vname = dart_enum_variant_ident(&v.name);
            let term = if i + 1 == e.variants.len() { ";" } else { "," };
            let _ = writeln!(out, "  {vname}({}){term}", format_enum_value(v.value));
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "  const {}(this.code);", e.name);
        let _ = writeln!(out, "  final int code;");
        let _ = writeln!(out);
        if let Some(fallback) = &e.dart_fallback {
            let fb_ident = dart_enum_variant_ident(fallback);
            let _ = writeln!(out, "  static {} fromCode(int code) {{", e.name);
            let _ = writeln!(out, "    return switch (code) {{");
            for v in &e.variants {
                let vname = dart_enum_variant_ident(&v.name);
                if vname == fb_ident {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "      {} => {}.{vname},",
                    format_enum_value(v.value),
                    e.name
                );
            }
            let _ = writeln!(out, "      _ => {}.{fb_ident},", e.name);
            let _ = writeln!(out, "    }};");
            let _ = writeln!(out, "  }}");
        } else {
            let _ = writeln!(out, "  static {}? fromCode(int code) {{", e.name);
            let _ = writeln!(out, "    return switch (code) {{");
            for v in &e.variants {
                let vname = dart_enum_variant_ident(&v.name);
                let _ = writeln!(
                    out,
                    "      {} => {}.{vname},",
                    format_enum_value(v.value),
                    e.name
                );
            }
            let _ = writeln!(out, "      _ => null,");
            let _ = writeln!(out, "    }};");
            let _ = writeln!(out, "  }}");
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    // Signed host-capability status enum (`NVE_IFACE_HOST = 5`)
    let _ = writeln!(out, "enum NveHostStatus {{");
    for (i, &(variant, code)) in HOST_STATUS_VARIANTS.iter().enumerate() {
        let vname = dart_enum_variant_ident(variant);
        let term = if i + 1 == HOST_STATUS_VARIANTS.len() {
            ";"
        } else {
            ","
        };
        let _ = writeln!(out, "  {vname}({code}){term}");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  const NveHostStatus(this.code);");
    let _ = writeln!(out, "  final int code;");
    let _ = writeln!(out);
    let _ = writeln!(out, "  static NveHostStatus? fromCode(int code) {{");
    let _ = writeln!(out, "    return switch (code) {{");
    for &(variant, code) in HOST_STATUS_VARIANTS {
        let vname = dart_enum_variant_ident(variant);
        let _ = writeln!(out, "      {code} => NveHostStatus.{vname},");
    }
    let _ = writeln!(out, "      _ => null,");
    let _ = writeln!(out, "    }};");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);

    // Core structs & union
    for s in &spec.structs {
        if s.name == "NveHostCallbacks" {
            let _ = writeln!(out, "typedef NveHostInvokeNative = ffi.Int32 Function(");
            let _ = writeln!(out, "  ffi.Pointer<ffi.Void> hostCtx,");
            let _ = writeln!(out, "  ffi.Uint32 serviceId,");
            let _ = writeln!(out, "  ffi.Uint32 methodId,");
            let _ = writeln!(out, "  ffi.Uint64 requestId,");
            let _ = writeln!(out, "  NveBufferSlice request,");
            let _ = writeln!(out, "  ffi.Pointer<ffi.Uint8> outBuf,");
            let _ = writeln!(out, "  ffi.Size outCap,");
            let _ = writeln!(out, "  ffi.Pointer<ffi.Size> outLen,");
            let _ = writeln!(out, ");");
            let _ = writeln!(
                out,
                "typedef NveHostChannelPublishNative = ffi.Int32 Function("
            );
            let _ = writeln!(out, "  ffi.Pointer<ffi.Void> hostCtx,");
            let _ = writeln!(out, "  ffi.Uint32 channelId,");
            let _ = writeln!(out, "  ffi.Uint32 flags,");
            let _ = writeln!(out, "  NveBufferSlice payload,");
            let _ = writeln!(out, ");");
            let _ = writeln!(out);
        }
        let base = if s.is_union { "ffi.Union" } else { "ffi.Struct" };
        let _ = writeln!(out, "final class {} extends {base} {{", s.name);
        for f in &s.fields {
            if let Some(ann) = dart_core_ffi_annotation(&f.ty) {
                let _ = writeln!(out, "  {ann}");
            }
            let dart_field = to_camel_case(&f.name);
            let _ = writeln!(
                out,
                "  external {} {dart_field};",
                dart_core_struct_field_type(&f.ty)
            );
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    // Native function typedefs for modular interfaces + nve_get_interface
    for m in &spec.modules {
        for method in &m.methods {
            let td_name = format!("Nve{}Native", to_pascal_case(&method.name));
            let ret = dart_core_native_type(&method.return_type);
            let mut params = Vec::new();
            if method.has_self {
                params.push("ffi.Pointer<ffi.Void> handle".to_string());
            }
            for p in &method.params {
                params.push(format!(
                    "{} {}",
                    dart_core_native_type(&p.ty),
                    to_camel_case(&p.name)
                ));
            }
            let _ = writeln!(out, "typedef {td_name} = {ret} Function(");
            for p in &params {
                let _ = writeln!(out, "  {p},");
            }
            let _ = writeln!(out, ");");
        }
    }
    let _ = writeln!(
        out,
        "typedef NveGetInterfaceNative = ffi.Pointer<ffi.Void> Function(ffi.Uint32 interfaceId);"
    );
    let _ = writeln!(out);

    // Modular interface vtable structs
    for m in &spec.modules {
        let c_name = module_c_struct_name(m);
        let _ = writeln!(out, "final class {c_name} extends ffi.Struct {{");
        for method in &m.methods {
            let td_name = format!("Nve{}Native", to_pascal_case(&method.name));
            let field_name = to_camel_case(&method.name);
            let _ = writeln!(
                out,
                "  external ffi.Pointer<ffi.NativeFunction<{td_name}>> {field_name};"
            );
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
    }

    out
}

// ============================================================================
// Core ABI Target 4: Android Java (`hosts/android-shell/.../NveCoreAbi.java`)
// ============================================================================

/// Generates `hosts/android-shell/android-app/src/dev/nve/android/NveCoreAbi.java`
/// directly from `crates/nve-codegen/specs/nve_core_abi.nve.rs`.
pub fn generate_java_core_abi() -> String {
    let spec = core_abi_spec();
    let schema_hash = spec.file_schema_hash();
    let mut out = String::with_capacity(4 * 1024);

    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source NVE Core ABI constants generated from specs/nve_core_abi.nve.rs."
    );
    let _ = writeln!(out, "package dev.nve.android;");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Generated NVE Core ABI constants, interface IDs, enums, and layout sizes."
    );
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "public final class NveCoreAbi {{");
    let _ = writeln!(out, "    private NveCoreAbi() {{}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "    public static final long CORE_ABI_SCHEMA_HASH = 0x{schema_hash:016X}L;"
    );
    let _ = writeln!(
        out,
        "    public static final String CORE_ABI_SCHEMA_HASH_HEX = \"0x{schema_hash:016X}\";"
    );
    let _ = writeln!(out);

    for c in &spec.consts {
        let jname = c.name.strip_prefix("NVE_").unwrap_or(&c.name);
        if c.hex {
            let _ = writeln!(out, "    public static final int {jname} = 0x{:X};", c.value);
        } else {
            let _ = writeln!(out, "    public static final int {jname} = {};", c.value);
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "    // Modular interface IDs (`nve_get_interface`)");
    for m in &spec.modules {
        let upper = to_snake_case(&m.name).to_ascii_uppercase();
        let _ = writeln!(
            out,
            "    public static final int IFACE_{upper} = {};",
            m.interface_id()
        );
    }
    let _ = writeln!(out);

    for e in &spec.enums {
        let prefix = format_c_enum_prefix(e);
        let jprefix = prefix.strip_prefix("NVE_").unwrap_or(&prefix);
        let _ = writeln!(out, "    // {}", e.name);
        for v in &e.variants {
            let v_upper = to_snake_case(&v.name).to_ascii_uppercase();
            let _ = writeln!(
                out,
                "    public static final int {jprefix}{v_upper} = {};",
                format_enum_value(v.value)
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "    // NveHostStatus");
    for &(variant, code) in HOST_STATUS_VARIANTS {
        let v_upper = to_snake_case(variant).to_ascii_uppercase();
        let _ = writeln!(out, "    public static final int HOST_{v_upper} = {code};");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "    // Fixed ABI struct sizes");
    for s in &spec.structs {
        let sup = to_snake_case(&s.name).to_ascii_uppercase();
        let short = sup.strip_prefix("NVE_").unwrap_or(&sup);
        if let Some(sz) = s.expected_size {
            let _ = writeln!(out, "    public static final int SIZEOF_{short} = {sz};");
        }
        if let Some(sz64) = s.expected_size_64 {
            let _ = writeln!(
                out,
                "    public static final int SIZEOF_{short}_64 = {sz64};"
            );
        }
    }
    let _ = writeln!(out, "}}");
    out
}

// ============================================================================
// Core ABI Target 5: Web TypeScript (`hosts/web-shell/nve_core_abi.ts`)
// ============================================================================

/// Generates `hosts/web-shell/nve_core_abi.ts` directly from
/// `crates/nve-codegen/specs/nve_core_abi.nve.rs`.
pub fn generate_ts_core_abi() -> String {
    let spec = core_abi_spec();
    let schema_hash = spec.file_schema_hash();
    let mut out = String::with_capacity(4 * 1024);

    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source NVE Core ABI constants generated from specs/nve_core_abi.nve.rs."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "export const NVE_CORE_ABI_SCHEMA_HASH: bigint = 0x{schema_hash:016X}n;"
    );
    for c in &spec.consts {
        if c.hex {
            let _ = writeln!(out, "export const {} = 0x{:X} as const;", c.name, c.value);
        } else {
            let _ = writeln!(out, "export const {} = {} as const;", c.name, c.value);
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "export const NVE_IFACE = {{");
    for m in &spec.modules {
        let upper = to_snake_case(&m.name).to_ascii_uppercase();
        let _ = writeln!(out, "  {upper}: {},", m.interface_id());
    }
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(
        out,
        "export type NveIfaceId = (typeof NVE_IFACE)[keyof typeof NVE_IFACE];"
    );
    let _ = writeln!(out);

    for e in &spec.enums {
        let obj_name = to_snake_case(&e.name).to_ascii_uppercase();
        let _ = writeln!(out, "export const {obj_name} = {{");
        for v in &e.variants {
            let v_upper = to_snake_case(&v.name).to_ascii_uppercase();
            let _ = writeln!(out, "  {v_upper}: {},", format_enum_value(v.value));
        }
        let _ = writeln!(out, "}} as const;");
        let _ = writeln!(
            out,
            "export type {} = (typeof {obj_name})[keyof typeof {obj_name}];",
            e.name
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "export const NVE_HOST_STATUS = {{");
    for &(variant, code) in HOST_STATUS_VARIANTS {
        let v_upper = to_snake_case(variant).to_ascii_uppercase();
        let _ = writeln!(out, "  {v_upper}: {code},");
    }
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(
        out,
        "export type NveHostStatus = (typeof NVE_HOST_STATUS)[keyof typeof NVE_HOST_STATUS];"
    );
    let _ = writeln!(out);

    out
}

// ============================================================================
// Scene ABI Target 1 & 2: C11 / C++20 / Swift (`nve_scene_abi.h`)
// ============================================================================

/// Generates `nve_scene_abi.h` (used identically by `cxx/nve-fabric-bridge/include/nve_scene_abi.h`
/// and `hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.h`) directly from
/// `crates/nve-codegen/specs/nve_scene_abi.nve.rs`.
pub fn generate_c_scene_abi_h() -> String {
    let spec = scene_abi_spec();
    let schema_hash = compute_scene_schema_hash(&spec);
    let render_node = spec.find_struct("NveRenderNode").unwrap();
    let scene_view = spec.find_struct("NveSceneView").unwrap();
    let node_size = render_node.expected_size.unwrap();
    let node_align = render_node.align.unwrap_or(NVE_RENDER_NODE_ALIGN);

    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "/* @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY. */"
    );
    let _ = writeln!(
        out,
        "/* Single-source 64-byte cache-line NveRenderNode + NveSceneView ABI. */"
    );
    let _ = writeln!(out, "#ifndef NVE_SCENE_ABI_H");
    let _ = writeln!(out, "#define NVE_SCENE_ABI_H");
    let _ = writeln!(out);
    let _ = writeln!(out, "#include <stddef.h>");
    let _ = writeln!(out, "#include <stdint.h>");
    let _ = writeln!(out);
    let _ = writeln!(out, "#ifdef __cplusplus");
    let _ = writeln!(out, "extern \"C\" {{");
    let _ = writeln!(
        out,
        "#define NVE_SCENE_STATIC_ASSERT(cond, msg) static_assert(cond, msg)"
    );
    let _ = writeln!(out, "#else");
    let _ = writeln!(
        out,
        "#define NVE_SCENE_STATIC_ASSERT(cond, msg) _Static_assert(cond, msg)"
    );
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);
    let _ = writeln!(out, "#define NVE_SCENE_SCHEMA_HASH 0x{schema_hash:016X}ULL");
    let _ = writeln!(out, "#define NVE_RENDER_NODE_SIZE {node_size}u");
    let _ = writeln!(out, "#define NVE_RENDER_NODE_ALIGN {node_align}u");
    let _ = writeln!(out);
    let _ = writeln!(out, "/* NveRenderNode::flags bits */");
    for c in &spec.consts {
        if matches!(
            c.name.as_str(),
            "NVE_NODE_LEAF_TEXT" | "NVE_NODE_INTERACTIVE" | "NVE_NODE_CLIP_BOUNDS"
        ) {
            let _ = writeln!(out, "#define {} {}u", c.name, c.value);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "/* Compact uint8_t NveRenderNode::kind discriminants */");
    for c in &spec.consts {
        if c.name.starts_with("NVE_NODE_KIND_") {
            let _ = writeln!(out, "#define {} {}u", c.name, c.value);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/* Naturally 4-byte aligned in C so Swift's ClangImporter imports it directly as a value type;"
    );
    let _ = writeln!(
        out,
        " * exact size is 64 bytes (1 cache line) and Rust aligns backing slabs to 64 bytes. */"
    );
    let _ = writeln!(out, "typedef struct NveRenderNode {{");
    for f in &render_node.fields {
        let _ = writeln!(
            out,
            "  {} {}; /* offset {} */",
            c_core_type(&f.ty),
            f.name,
            f.offset.unwrap()
        );
    }
    let _ = writeln!(out, "}} NveRenderNode;");
    let _ = writeln!(out);
    let _ = writeln!(out, "typedef struct NveSceneView {{");
    for f in &scene_view.fields {
        let _ = writeln!(out, "  {} {};", c_core_type(&f.ty), f.name);
    }
    let _ = writeln!(out, "}} NveSceneView;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "NVE_SCENE_STATIC_ASSERT(sizeof(NveRenderNode) == {node_size}u, \"NveRenderNode must be 64 bytes\");"
    );
    for f in &render_node.fields {
        let off = f.offset.unwrap();
        let _ = writeln!(
            out,
            "NVE_SCENE_STATIC_ASSERT(offsetof(NveRenderNode, {}) == {off}u, \"NveRenderNode.{} offset mismatch\");",
            f.name, f.name
        );
    }
    for f in &scene_view.fields {
        if let Some(off) = f.offset {
            let _ = writeln!(
                out,
                "NVE_SCENE_STATIC_ASSERT(offsetof(NveSceneView, {}) == {off}u, \"NveSceneView.{} offset mismatch\");",
                f.name, f.name
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "#undef NVE_SCENE_STATIC_ASSERT");
    let _ = writeln!(out);
    let _ = writeln!(out, "#ifdef __cplusplus");
    let _ = writeln!(out, "}} /* extern \"C\" */");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);
    let _ = writeln!(out, "#endif /* NVE_SCENE_ABI_H */");
    out
}

// ============================================================================
// Scene ABI Target 3: Dart `dart:ffi` (`hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart`)
// ============================================================================

/// Generates `hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart` directly from
/// `crates/nve-codegen/specs/nve_scene_abi.nve.rs`.
pub fn generate_dart_scene_abi() -> String {
    let spec = scene_abi_spec();
    let schema_hash = compute_scene_schema_hash(&spec);
    let signed_hash = schema_hash as i64;
    let render_node = spec.find_struct("NveRenderNode").unwrap();
    let scene_view = spec.find_struct("NveSceneView").unwrap();
    let node_size = render_node.expected_size.unwrap();

    let mut out = String::with_capacity(6144);
    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source 64-byte NveRenderNode + NveSceneView `dart:ffi` bindings."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "import 'dart:convert';");
    let _ = writeln!(out, "import 'dart:ffi' as ffi;");
    let _ = writeln!(out, "import 'dart:typed_data';");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Deterministic FNV-1a 64-bit schema hash (`0x{schema_hash:016X}`)."
    );
    let _ = writeln!(
        out,
        "/// Stored as 64-bit two's-complement `int` matching `@ffi.Uint64()` bit pattern on native VM."
    );
    let _ = writeln!(out, "const int nveSceneSchemaHash = {signed_hash};");
    let _ = writeln!(
        out,
        "const String nveSceneSchemaHashHex = '0x{schema_hash:016X}';"
    );
    let _ = writeln!(out, "const int nveRenderNodeStride = {node_size};");
    let _ = writeln!(out);
    let _ = writeln!(out, "// NveRenderNode::flags bit constants");
    let _ = writeln!(out, "const int nveNodeLeafText = 1 << 0;");
    let _ = writeln!(out, "const int nveNodeInteractive = 1 << 1;");
    let _ = writeln!(out, "const int nveNodeClipBounds = 1 << 2;");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Compact uint8 NveRenderNode::kind discriminants");
    for c in &spec.consts {
        if c.name.starts_with("NVE_NODE_KIND_") {
            let dart_name = to_camel_case(&c.name.to_ascii_lowercase());
            let _ = writeln!(out, "const int {dart_name} = {};", c.value);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "// Exact byte offsets within one 64-byte NveRenderNode");
    for f in &render_node.fields {
        let const_name = format!(
            "nveNodeOffset{}",
            to_pascal_case(f.name.trim_start_matches('_'))
        );
        let _ = writeln!(out, "const int {const_name} = {};", f.offset.unwrap());
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// 64-byte zero-decode render node (`NveRenderNode` in `nve_scene_abi.h`)."
    );
    let _ = writeln!(out, "final class NveRenderNode extends ffi.Struct {{");
    for f in &render_node.fields {
        let dart_name = if f.name == "_pad0" {
            "pad0".to_owned()
        } else {
            snake_to_camel_preserve_leading(&f.name)
        };
        if let Some(ann) = dart_core_ffi_annotation(&f.ty) {
            let _ = writeln!(out, "  {ann}");
        }
        let _ = writeln!(
            out,
            "  external {} {dart_name}; // offset {}",
            dart_core_struct_field_type(&f.ty),
            f.offset.unwrap()
        );
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Zero-decode scene snapshot header + slab pointers (`NveSceneView` in `nve_scene_abi.h`)."
    );
    let _ = writeln!(out, "final class NveSceneView extends ffi.Struct {{");
    for f in &scene_view.fields {
        let dart_name = if f.name == "_pad" {
            "pad".to_owned()
        } else {
            snake_to_camel_preserve_leading(&f.name)
        };
        if let Some(ann) = dart_core_ffi_annotation(&f.ty) {
            let _ = writeln!(out, "  {ann}");
        }
        let _ = writeln!(
            out,
            "  external {} {dart_name};",
            dart_core_struct_field_type(&f.ty)
        );
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Error thrown when an `NveSceneView` fails schema or bounds validation."
    );
    let _ = writeln!(out, "class NveSceneAbiException implements Exception {{");
    let _ = writeln!(out, "  const NveSceneAbiException(this.message);");
    let _ = writeln!(out, "  final String message;");
    let _ = writeln!(out, "  @override");
    let _ = writeln!(out, "  String toString() => 'NveSceneAbiException: $message';");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Zero-copy, zero-decode reader over a native `Pointer<NveSceneView>`."
    );
    let _ = writeln!(out, "class NveSceneReader {{");
    let _ = writeln!(out, "  NveSceneReader._({{");
    let _ = writeln!(out, "    required this.frameId,");
    let _ = writeln!(out, "    required this.nodeCount,");
    let _ = writeln!(out, "    required ffi.Pointer<NveRenderNode> nodesPtr,");
    let _ = writeln!(out, "    required this.textSlab,");
    let _ = writeln!(out, "    required this.childrenSlab,");
    let _ = writeln!(out, "    required this.dirtyIndices,");
    let _ = writeln!(out, "    required this.removedIds,");
    let _ = writeln!(out, "  }}) : _nodesPtr = nodesPtr;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  /// Validates `schemaHash` and non-null pointer invariants with zero data copies."
    );
    let _ = writeln!(
        out,
        "  factory NveSceneReader.fromPointer(ffi.Pointer<NveSceneView> viewPtr) {{"
    );
    let _ = writeln!(out, "    if (viewPtr == ffi.nullptr) {{");
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('NveSceneView pointer is null');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final NveSceneView view = viewPtr.ref;");
    let _ = writeln!(out, "    if (view.schemaHash != nveSceneSchemaHash) {{");
    let _ = writeln!(
        out,
        "      throw NveSceneAbiException('Schema hash mismatch: expected $nveSceneSchemaHashHex, got 0x${{view.schemaHash.toUnsigned(64).toRadixString(16).toUpperCase()}}');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int nodeCount = view.nodeCount;");
    let _ = writeln!(out, "    if (nodeCount > 0 && view.nodes == ffi.nullptr) {{");
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('nodes pointer is null with non-zero nodeCount');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int textLen = view.textSlabLen;");
    let _ = writeln!(out, "    if (textLen > 0 && view.textSlab == ffi.nullptr) {{");
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('textSlab pointer is null with non-zero textSlabLen');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int childrenLen = view.childrenSlabLen;");
    let _ = writeln!(
        out,
        "    if (childrenLen > 0 && view.childrenSlab == ffi.nullptr) {{"
    );
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('childrenSlab pointer is null with non-zero childrenSlabLen');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int dirtyCount = view.dirtyCount;");
    let _ = writeln!(
        out,
        "    if (dirtyCount > 0 && view.dirtyIndices == ffi.nullptr) {{"
    );
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('dirtyIndices pointer is null with non-zero dirtyCount');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int removedCount = view.removedCount;");
    let _ = writeln!(
        out,
        "    if (removedCount > 0 && view.removedIds == ffi.nullptr) {{"
    );
    let _ = writeln!(
        out,
        "      throw const NveSceneAbiException('removedIds pointer is null with non-zero removedCount');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    return NveSceneReader._(");
    let _ = writeln!(out, "      frameId: view.frameId,");
    let _ = writeln!(out, "      nodeCount: nodeCount,");
    let _ = writeln!(out, "      nodesPtr: view.nodes,");
    let _ = writeln!(
        out,
        "      textSlab: textLen == 0 ? Uint8List(0) : view.textSlab.asTypedList(textLen),"
    );
    let _ = writeln!(
        out,
        "      childrenSlab: childrenLen == 0 ? Uint32List(0) : view.childrenSlab.asTypedList(childrenLen),"
    );
    let _ = writeln!(
        out,
        "      dirtyIndices: dirtyCount == 0 ? Uint32List(0) : view.dirtyIndices.asTypedList(dirtyCount),"
    );
    let _ = writeln!(
        out,
        "      removedIds: removedCount == 0 ? Uint32List(0) : view.removedIds.asTypedList(removedCount),"
    );
    let _ = writeln!(out, "    );");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  final int frameId;");
    let _ = writeln!(out, "  final int nodeCount;");
    let _ = writeln!(out, "  final ffi.Pointer<NveRenderNode> _nodesPtr;");
    let _ = writeln!(out, "  final Uint8List textSlab;");
    let _ = writeln!(out, "  final Uint32List childrenSlab;");
    let _ = writeln!(out, "  final Uint32List dirtyIndices;");
    let _ = writeln!(out, "  final Uint32List removedIds;");
    let _ = writeln!(out);
    let _ = writeln!(out, "  /// Zero-copy indexed access to a 64-byte `NveRenderNode`.");
    let _ = writeln!(out, "  NveRenderNode nodeAt(int index) {{");
    let _ = writeln!(out, "    if (index < 0 || index >= nodeCount) {{");
    let _ = writeln!(
        out,
        "      throw RangeError.index(index, this, 'index', null, nodeCount);"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return _nodesPtr[index];");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  /// Zero-copy UTF-8 byte slice for `node` inside `textSlab`.");
    let _ = writeln!(out, "  Uint8List textBytesFor(NveRenderNode node) {{");
    let _ = writeln!(out, "    final int len = node.textLen;");
    let _ = writeln!(out, "    if (len == 0) {{");
    let _ = writeln!(out, "      return Uint8List(0);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int start = node.textOffset;");
    let _ = writeln!(out, "    final int end = start + len;");
    let _ = writeln!(out, "    if (start < 0 || end > textSlab.length) {{");
    let _ = writeln!(
        out,
        "      throw NveSceneAbiException('Node ${{node.nodeId}} text slice [$start..$end] out of bounds (${{textSlab.length}})');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return Uint8List.sublistView(textSlab, start, end);");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  /// Decodes `node`'s UTF-8 text directly from `textSlab`.");
    let _ = writeln!(out, "  String textFor(NveRenderNode node) {{");
    let _ = writeln!(out, "    final Uint8List slice = textBytesFor(node);");
    let _ = writeln!(out, "    if (slice.isEmpty) {{");
    let _ = writeln!(out, "      return '';");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return utf8.decode(slice, allowMalformed: false);");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  /// Zero-copy `Uint32List` view over `node`'s children in `childrenSlab`."
    );
    let _ = writeln!(out, "  Uint32List childrenFor(NveRenderNode node) {{");
    let _ = writeln!(out, "    final int count = node.childCount;");
    let _ = writeln!(out, "    if (count == 0) {{");
    let _ = writeln!(out, "      return Uint32List(0);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    final int start = node.childOffset;");
    let _ = writeln!(out, "    final int end = start + count;");
    let _ = writeln!(out, "    if (start < 0 || end > childrenSlab.length) {{");
    let _ = writeln!(
        out,
        "      throw NveSceneAbiException('Node ${{node.nodeId}} children slice [$start..$end] out of bounds (${{childrenSlab.length}})');"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return Uint32List.sublistView(childrenSlab, start, end);");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  /// Iterates only dirty nodes in `O(dirtyCount)`.");
    let _ = writeln!(
        out,
        "  void forEachDirty(void Function(int index, NveRenderNode node) visit) {{"
    );
    let _ = writeln!(out, "    for (int i = 0; i < dirtyIndices.length; i++) {{");
    let _ = writeln!(out, "      final int idx = dirtyIndices[i];");
    let _ = writeln!(out, "      if (idx < nodeCount) {{");
    let _ = writeln!(out, "        visit(idx, _nodesPtr[idx]);");
    let _ = writeln!(out, "      }}");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    out
}

// ============================================================================
// Scene ABI Target 4: Java / Android DirectByteBuffer (`NveSceneBuffer.java`)
// ============================================================================

/// Generates `hosts/android-shell/android-app/src/dev/nve/android/NveSceneBuffer.java`
/// directly from `crates/nve-codegen/specs/nve_scene_abi.nve.rs`.
pub fn generate_java_scene_buffer() -> String {
    let spec = scene_abi_spec();
    let schema_hash = compute_scene_schema_hash(&spec);
    let render_node = spec.find_struct("NveRenderNode").unwrap();
    let node_size = render_node.expected_size.unwrap();

    let mut out = String::with_capacity(6144);
    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source 64-byte NveRenderNode + NveSceneView zero-GC DirectByteBuffer reader."
    );
    let _ = writeln!(out, "package dev.nve.android;");
    let _ = writeln!(out);
    let _ = writeln!(out, "import java.nio.ByteBuffer;");
    let _ = writeln!(out, "import java.nio.ByteOrder;");
    let _ = writeln!(out, "import java.nio.charset.StandardCharsets;");
    let _ = writeln!(out);
    let _ = writeln!(out, "/**");
    let _ = writeln!(
        out,
        " * Zero-copy, zero-JSON reader over JNI `NewDirectByteBuffer` slabs for `NveSceneView`."
    );
    let _ = writeln!(out, " */");
    let _ = writeln!(out, "public final class NveSceneBuffer {{");
    let _ = writeln!(
        out,
        "    public static final long SCHEMA_HASH = 0x{schema_hash:016X}L;"
    );
    let _ = writeln!(out, "    public static final int NODE_STRIDE = {node_size};");
    let _ = writeln!(out);
    let _ = writeln!(out, "    // NveRenderNode::flags bits");
    let _ = writeln!(out, "    public static final int FLAG_LEAF_TEXT = 1 << 0;");
    let _ = writeln!(out, "    public static final int FLAG_INTERACTIVE = 1 << 1;");
    let _ = writeln!(out, "    public static final int FLAG_CLIP_BOUNDS = 1 << 2;");
    let _ = writeln!(out);
    let _ = writeln!(out, "    // Compact uint8 NveRenderNode::kind discriminants");
    for c in &spec.consts {
        if let Some(suffix) = c.name.strip_prefix("NVE_NODE_KIND_") {
            let _ = writeln!(out, "    public static final int KIND_{suffix} = {};", c.value);
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "    // Exact byte offsets within one 64-byte NveRenderNode");
    for f in &render_node.fields {
        let _ = writeln!(
            out,
            "    public static final int OFFSET_{} = {};",
            snake_to_screaming(&f.name),
            f.offset.unwrap()
        );
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "    private long frameId;");
    let _ = writeln!(out, "    private int nodeCount;");
    let _ = writeln!(out, "    private int dirtyCount;");
    let _ = writeln!(out, "    private int removedCount;");
    let _ = writeln!(out, "    private int textSlabLen;");
    let _ = writeln!(out, "    private int childrenSlabLen;");
    let _ = writeln!(out, "    private ByteBuffer nodesBuf;");
    let _ = writeln!(out, "    private ByteBuffer textSlabBuf;");
    let _ = writeln!(out, "    private ByteBuffer childrenSlabBuf;");
    let _ = writeln!(out, "    private ByteBuffer dirtyIndicesBuf;");
    let _ = writeln!(out, "    private ByteBuffer removedIdsBuf;");
    let _ = writeln!(out, "    private byte[] textScratch = new byte[256];");
    let _ = writeln!(out);
    let _ = writeln!(out, "    /**");
    let _ = writeln!(
        out,
        "     * Rebinds this reusable reader to the latest frame's direct buffers without copying."
    );
    let _ = writeln!(out, "     */");
    let _ = writeln!(out, "    public void attach(");
    let _ = writeln!(out, "            long schemaHash,");
    let _ = writeln!(out, "            long frameId,");
    let _ = writeln!(out, "            int nodeCount,");
    let _ = writeln!(out, "            int dirtyCount,");
    let _ = writeln!(out, "            int removedCount,");
    let _ = writeln!(out, "            int textSlabLen,");
    let _ = writeln!(out, "            int childrenSlabLen,");
    let _ = writeln!(out, "            ByteBuffer nodesBuf,");
    let _ = writeln!(out, "            ByteBuffer textSlabBuf,");
    let _ = writeln!(out, "            ByteBuffer childrenSlabBuf,");
    let _ = writeln!(out, "            ByteBuffer dirtyIndicesBuf,");
    let _ = writeln!(out, "            ByteBuffer removedIdsBuf) {{");
    let _ = writeln!(out, "        if (schemaHash != SCHEMA_HASH) {{");
    let _ = writeln!(
        out,
        "            throw new IllegalStateException(\"NveSceneView schema hash mismatch: expected 0x\" + Long.toHexString(SCHEMA_HASH) + \", got 0x\" + Long.toHexString(schemaHash));"
    );
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        this.frameId = frameId;");
    let _ = writeln!(out, "        this.nodeCount = Math.max(0, nodeCount);");
    let _ = writeln!(out, "        this.dirtyCount = Math.max(0, dirtyCount);");
    let _ = writeln!(out, "        this.removedCount = Math.max(0, removedCount);");
    let _ = writeln!(out, "        this.textSlabLen = Math.max(0, textSlabLen);");
    let _ = writeln!(out, "        this.childrenSlabLen = Math.max(0, childrenSlabLen);");
    let _ = writeln!(
        out,
        "        this.nodesBuf = nodesBuf != null ? nodesBuf.order(ByteOrder.LITTLE_ENDIAN) : null;"
    );
    let _ = writeln!(
        out,
        "        this.textSlabBuf = textSlabBuf != null ? textSlabBuf.order(ByteOrder.LITTLE_ENDIAN) : null;"
    );
    let _ = writeln!(
        out,
        "        this.childrenSlabBuf = childrenSlabBuf != null ? childrenSlabBuf.order(ByteOrder.LITTLE_ENDIAN) : null;"
    );
    let _ = writeln!(
        out,
        "        this.dirtyIndicesBuf = dirtyIndicesBuf != null ? dirtyIndicesBuf.order(ByteOrder.LITTLE_ENDIAN) : null;"
    );
    let _ = writeln!(
        out,
        "        this.removedIdsBuf = removedIdsBuf != null ? removedIdsBuf.order(ByteOrder.LITTLE_ENDIAN) : null;"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public long frameId() {{ return frameId; }}");
    let _ = writeln!(out, "    public int nodeCount() {{ return nodeCount; }}");
    let _ = writeln!(out, "    public int dirtyCount() {{ return dirtyCount; }}");
    let _ = writeln!(out, "    public int removedCount() {{ return removedCount; }}");
    let _ = writeln!(out, "    public int textSlabLen() {{ return textSlabLen; }}");
    let _ = writeln!(out, "    public int childrenSlabLen() {{ return childrenSlabLen; }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public int dirtyIndex(int i) {{");
    let _ = writeln!(out, "        return dirtyIndicesBuf.getInt(i << 2);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public int removedId(int i) {{");
    let _ = writeln!(out, "        return removedIdsBuf.getInt(i << 2);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public int nodeId(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_NODE_ID); }}");
    let _ = writeln!(out, "    public int kind(int idx) {{ return nodesBuf.get(idx * NODE_STRIDE + OFFSET_KIND) & 0xFF; }}");
    let _ = writeln!(out, "    public int flags(int idx) {{ return nodesBuf.get(idx * NODE_STRIDE + OFFSET_FLAGS) & 0xFF; }}");
    let _ = writeln!(out, "    public int opacityU8(int idx) {{ return nodesBuf.get(idx * NODE_STRIDE + OFFSET_OPACITY_U8) & 0xFF; }}");
    let _ = writeln!(out, "    public float opacity(int idx) {{ return opacityU8(idx) / 255.0f; }}");
    let _ = writeln!(out, "    public float x(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_X); }}");
    let _ = writeln!(out, "    public float y(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_Y); }}");
    let _ = writeln!(out, "    public float width(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_WIDTH); }}");
    let _ = writeln!(out, "    public float height(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_HEIGHT); }}");
    let _ = writeln!(out, "    public int bgRgba(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_BG_RGBA); }}");
    let _ = writeln!(out, "    public int textRgba(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_TEXT_RGBA); }}");
    let _ = writeln!(out, "    public int borderRgba(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_BORDER_RGBA); }}");
    let _ = writeln!(out, "    public float borderRadius(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_BORDER_RADIUS); }}");
    let _ = writeln!(out, "    public float borderWidth(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_BORDER_WIDTH); }}");
    let _ = writeln!(out, "    public float fontSize(int idx) {{ return nodesBuf.getFloat(idx * NODE_STRIDE + OFFSET_FONT_SIZE); }}");
    let _ = writeln!(out, "    public int textOffset(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_TEXT_OFFSET); }}");
    let _ = writeln!(out, "    public int textLen(int idx) {{ return nodesBuf.getShort(idx * NODE_STRIDE + OFFSET_TEXT_LEN) & 0xFFFF; }}");
    let _ = writeln!(out, "    public int childCount(int idx) {{ return nodesBuf.getShort(idx * NODE_STRIDE + OFFSET_CHILD_COUNT) & 0xFFFF; }}");
    let _ = writeln!(out, "    public int childOffset(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_CHILD_OFFSET); }}");
    let _ = writeln!(out, "    public int zIndex(int idx) {{ return nodesBuf.getInt(idx * NODE_STRIDE + OFFSET_Z_INDEX); }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public boolean isLeafText(int idx) {{ return (flags(idx) & FLAG_LEAF_TEXT) != 0; }}");
    let _ = writeln!(out, "    public boolean isInteractive(int idx) {{ return (flags(idx) & FLAG_INTERACTIVE) != 0; }}");
    let _ = writeln!(out, "    public boolean clipsBounds(int idx) {{ return (flags(idx) & FLAG_CLIP_BOUNDS) != 0; }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public int childId(int nodeIdx, int childPos) {{");
    let _ = writeln!(out, "        int offset = childOffset(nodeIdx) + childPos;");
    let _ = writeln!(out, "        return childrenSlabBuf.getInt(offset << 2);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "    public String readText(int nodeIdx) {{");
    let _ = writeln!(out, "        int len = textLen(nodeIdx);");
    let _ = writeln!(out, "        if (len == 0 || textSlabBuf == null) {{");
    let _ = writeln!(out, "            return \"\";");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        int off = textOffset(nodeIdx);");
    let _ = writeln!(out, "        if (off < 0 || off + len > textSlabLen) {{");
    let _ = writeln!(out, "            return \"\";");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        if (textScratch.length < len) {{");
    let _ = writeln!(out, "            textScratch = new byte[Math.max(len, textScratch.length << 1)];");
    let _ = writeln!(out, "        }}");
    let _ = writeln!(out, "        ByteBuffer dup = textSlabBuf.duplicate();");
    let _ = writeln!(out, "        dup.position(off);");
    let _ = writeln!(out, "        dup.get(textScratch, 0, len);");
    let _ = writeln!(out, "        return new String(textScratch, 0, len, StandardCharsets.UTF_8);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "}}");
    out
}

// ============================================================================
// Scene ABI Target 5: TypeScript / WASM (`hosts/web-shell/nve_scene_view.ts`)
// ============================================================================

/// Generates `hosts/web-shell/nve_scene_view.ts` directly from
/// `crates/nve-codegen/specs/nve_scene_abi.nve.rs`.
pub fn generate_ts_scene_view() -> String {
    let spec = scene_abi_spec();
    let schema_hash = compute_scene_schema_hash(&spec);
    let render_node = spec.find_struct("NveRenderNode").unwrap();
    let node_size = render_node.expected_size.unwrap();

    let mut out = String::with_capacity(5120);
    let _ = writeln!(
        out,
        "// @generated by nve-codegen (hybrid unified ABI generator) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source 64-byte NveRenderNode + NveSceneView zero-copy WASM memory reader."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "export const NVE_SCENE_SCHEMA_HASH: bigint = 0x{schema_hash:016X}n;"
    );
    let _ = writeln!(
        out,
        "export const NVE_RENDER_NODE_STRIDE = {node_size} as const;"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_FLAG = {{");
    let _ = writeln!(out, "  LEAF_TEXT: 1 << 0,");
    let _ = writeln!(out, "  INTERACTIVE: 1 << 1,");
    let _ = writeln!(out, "  CLIP_BOUNDS: 1 << 2,");
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_KIND = {{");
    for c in &spec.consts {
        if let Some(suffix) = c.name.strip_prefix("NVE_NODE_KIND_") {
            let _ = writeln!(out, "  {suffix}: {},", c.value);
        }
    }
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_OFFSET = {{");
    for f in &render_node.fields {
        let _ = writeln!(
            out,
            "  {}: {},",
            snake_to_screaming(&f.name),
            f.offset.unwrap()
        );
    }
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/** Pointer & count descriptor for a WASM `NveSceneView` inside linear memory. */"
    );
    let _ = writeln!(out, "export interface NveSceneMemoryLayout {{");
    let _ = writeln!(out, "  readonly schemaHash: bigint;");
    let _ = writeln!(out, "  readonly frameId: bigint;");
    let _ = writeln!(out, "  readonly nodeCount: number;");
    let _ = writeln!(out, "  readonly dirtyCount: number;");
    let _ = writeln!(out, "  readonly removedCount: number;");
    let _ = writeln!(out, "  readonly textSlabLen: number;");
    let _ = writeln!(out, "  readonly childrenSlabLen: number;");
    let _ = writeln!(out, "  readonly nodesPtr: number;");
    let _ = writeln!(out, "  readonly textSlabPtr: number;");
    let _ = writeln!(out, "  readonly childrenSlabPtr: number;");
    let _ = writeln!(out, "  readonly dirtyIndicesPtr: number;");
    let _ = writeln!(out, "  readonly removedIdsPtr: number;");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "const sharedUtf8Decoder = new TextDecoder('utf-8', {{ fatal: true }});"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/** Zero-copy view over an `NveSceneView` stored directly inside `WebAssembly.Memory.buffer`. */"
    );
    let _ = writeln!(out, "export class NveSceneViewReader {{");
    let _ = writeln!(out, "  readonly frameId: bigint;");
    let _ = writeln!(out, "  readonly nodeCount: number;");
    let _ = writeln!(out, "  readonly textSlab: Uint8Array;");
    let _ = writeln!(out, "  readonly childrenSlab: Uint32Array;");
    let _ = writeln!(out, "  readonly dirtyIndices: Uint32Array;");
    let _ = writeln!(out, "  readonly removedIds: Uint32Array;");
    let _ = writeln!(out, "  private readonly nodesView: DataView;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  constructor(memoryBuffer: ArrayBuffer, layout: NveSceneMemoryLayout) {{"
    );
    let _ = writeln!(out, "    if (layout.schemaHash !== NVE_SCENE_SCHEMA_HASH) {{");
    let _ = writeln!(
        out,
        "      throw new Error(`NveSceneView schema hash mismatch: expected 0x${{NVE_SCENE_SCHEMA_HASH.toString(16)}}, got 0x${{layout.schemaHash.toString(16)}}`);"
    );
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    this.frameId = layout.frameId;");
    let _ = writeln!(out, "    this.nodeCount = layout.nodeCount;");
    let _ = writeln!(
        out,
        "    this.nodesView = new DataView(memoryBuffer, layout.nodesPtr, layout.nodeCount * NVE_RENDER_NODE_STRIDE);"
    );
    let _ = writeln!(
        out,
        "    this.textSlab = new Uint8Array(memoryBuffer, layout.textSlabPtr, layout.textSlabLen);"
    );
    let _ = writeln!(
        out,
        "    this.childrenSlab = new Uint32Array(memoryBuffer, layout.childrenSlabPtr, layout.childrenSlabLen);"
    );
    let _ = writeln!(
        out,
        "    this.dirtyIndices = new Uint32Array(memoryBuffer, layout.dirtyIndicesPtr, layout.dirtyCount);"
    );
    let _ = writeln!(
        out,
        "    this.removedIds = new Uint32Array(memoryBuffer, layout.removedIdsPtr, layout.removedCount);"
    );
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  /** Reads an `NveSceneView` header directly at `viewPtr` inside wasm32 linear memory. */"
    );
    let _ = writeln!(
        out,
        "  static fromWasm32Pointer(memoryBuffer: ArrayBuffer, viewPtr: number): NveSceneViewReader {{"
    );
    let _ = writeln!(out, "    const hdr = new DataView(memoryBuffer, viewPtr, 60);");
    let _ = writeln!(out, "    return new NveSceneViewReader(memoryBuffer, {{");
    let _ = writeln!(out, "      schemaHash: hdr.getBigUint64(0, true),");
    let _ = writeln!(out, "      frameId: hdr.getBigUint64(8, true),");
    let _ = writeln!(out, "      nodeCount: hdr.getUint32(16, true),");
    let _ = writeln!(out, "      dirtyCount: hdr.getUint32(20, true),");
    let _ = writeln!(out, "      removedCount: hdr.getUint32(24, true),");
    let _ = writeln!(out, "      textSlabLen: hdr.getUint32(28, true),");
    let _ = writeln!(out, "      childrenSlabLen: hdr.getUint32(32, true),");
    let _ = writeln!(out, "      nodesPtr: hdr.getUint32(40, true),");
    let _ = writeln!(out, "      textSlabPtr: hdr.getUint32(44, true),");
    let _ = writeln!(out, "      childrenSlabPtr: hdr.getUint32(48, true),");
    let _ = writeln!(out, "      dirtyIndicesPtr: hdr.getUint32(52, true),");
    let _ = writeln!(out, "      removedIdsPtr: hdr.getUint32(56, true),");
    let _ = writeln!(out, "    }});");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  nodeId(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.NODE_ID, true); }}");
    let _ = writeln!(out, "  kind(idx: number): number {{ return this.nodesView.getUint8(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.KIND); }}");
    let _ = writeln!(out, "  flags(idx: number): number {{ return this.nodesView.getUint8(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.FLAGS); }}");
    let _ = writeln!(out, "  opacityU8(idx: number): number {{ return this.nodesView.getUint8(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.OPACITY_U8); }}");
    let _ = writeln!(out, "  opacity(idx: number): number {{ return this.opacityU8(idx) / 255.0; }}");
    let _ = writeln!(out, "  x(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.X, true); }}");
    let _ = writeln!(out, "  y(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.Y, true); }}");
    let _ = writeln!(out, "  width(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.WIDTH, true); }}");
    let _ = writeln!(out, "  height(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.HEIGHT, true); }}");
    let _ = writeln!(out, "  bgRgba(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.BG_RGBA, true); }}");
    let _ = writeln!(out, "  textRgba(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.TEXT_RGBA, true); }}");
    let _ = writeln!(out, "  borderRgba(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.BORDER_RGBA, true); }}");
    let _ = writeln!(out, "  borderRadius(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.BORDER_RADIUS, true); }}");
    let _ = writeln!(out, "  borderWidth(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.BORDER_WIDTH, true); }}");
    let _ = writeln!(out, "  fontSize(idx: number): number {{ return this.nodesView.getFloat32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.FONT_SIZE, true); }}");
    let _ = writeln!(out, "  textOffset(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.TEXT_OFFSET, true); }}");
    let _ = writeln!(out, "  textLen(idx: number): number {{ return this.nodesView.getUint16(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.TEXT_LEN, true); }}");
    let _ = writeln!(out, "  childCount(idx: number): number {{ return this.nodesView.getUint16(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.CHILD_COUNT, true); }}");
    let _ = writeln!(out, "  childOffset(idx: number): number {{ return this.nodesView.getUint32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.CHILD_OFFSET, true); }}");
    let _ = writeln!(out, "  zIndex(idx: number): number {{ return this.nodesView.getInt32(idx * NVE_RENDER_NODE_STRIDE + NVE_NODE_OFFSET.Z_INDEX, true); }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  isLeafText(idx: number): boolean {{ return (this.flags(idx) & NVE_NODE_FLAG.LEAF_TEXT) !== 0; }}");
    let _ = writeln!(out, "  isInteractive(idx: number): boolean {{ return (this.flags(idx) & NVE_NODE_FLAG.INTERACTIVE) !== 0; }}");
    let _ = writeln!(out, "  clipsBounds(idx: number): boolean {{ return (this.flags(idx) & NVE_NODE_FLAG.CLIP_BOUNDS) !== 0; }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  readText(idx: number): string {{");
    let _ = writeln!(out, "    const len = this.textLen(idx);");
    let _ = writeln!(out, "    if (len === 0) return '';");
    let _ = writeln!(out, "    const start = this.textOffset(idx);");
    let _ = writeln!(out, "    const end = start + len;");
    let _ = writeln!(out, "    if (end > this.textSlab.byteLength) {{");
    let _ = writeln!(out, "      throw new RangeError(`Text slice [${{start}}..${{end}}] exceeds textSlab (${{this.textSlab.byteLength}})`);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return sharedUtf8Decoder.decode(this.textSlab.subarray(start, end));");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out);
    let _ = writeln!(out, "  childrenOf(idx: number): Uint32Array {{");
    let _ = writeln!(out, "    const count = this.childCount(idx);");
    let _ = writeln!(out, "    if (count === 0) return new Uint32Array(0);");
    let _ = writeln!(out, "    const start = this.childOffset(idx);");
    let _ = writeln!(out, "    const end = start + count;");
    let _ = writeln!(out, "    if (end > this.childrenSlab.length) {{");
    let _ = writeln!(out, "      throw new RangeError(`Children slice [${{start}}..${{end}}] exceeds childrenSlab (${{this.childrenSlab.length}})`);");
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    return this.childrenSlab.subarray(start, end);");
    let _ = writeln!(out, "  }}");
    let _ = writeln!(out, "}}");
    out
}

// ============================================================================
// Unified File Bundle Entrypoints (5 Core + 5 Scene = 10 Checked-in Artifacts)
// ============================================================================

/// Generates all 5 checked-in Core ABI files keyed by workspace-relative path.
pub fn generate_all_core_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_core_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_abi.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_abi.h".to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_core_abi_gen.dart".to_owned(),
            content: generate_dart_core_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/NveCoreAbi.java".to_owned(),
            content: generate_java_core_abi(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_core_abi.ts".to_owned(),
            content: generate_ts_core_abi(),
        },
    ]
}

/// Generates all 5 checked-in Scene ABI files keyed by workspace-relative path.
pub fn generate_all_scene_abi_files() -> Vec<GeneratedFile> {
    let c_header = generate_c_scene_abi_h();
    vec![
        GeneratedFile {
            relative_path: "cxx/nve-fabric-bridge/include/nve_scene_abi.h".to_owned(),
            content: c_header.clone(),
        },
        GeneratedFile {
            relative_path: "hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.h".to_owned(),
            content: c_header,
        },
        GeneratedFile {
            relative_path: "hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart".to_owned(),
            content: generate_dart_scene_abi(),
        },
        GeneratedFile {
            relative_path:
                "hosts/android-shell/android-app/src/dev/nve/android/NveSceneBuffer.java"
                    .to_owned(),
            content: generate_java_scene_buffer(),
        },
        GeneratedFile {
            relative_path: "hosts/web-shell/nve_scene_view.ts".to_owned(),
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
