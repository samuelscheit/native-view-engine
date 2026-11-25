//! Single-source schema descriptor and deterministic 5-target generator for NVE's zero-decode
//! 64-byte `NveRenderNode` + `NveSceneView` ABI.
//!
//! All five checked-in target bindings are emitted from [`NVE_RENDER_NODE_FIELDS`] and
//! [`NVE_SCENE_VIEW_FIELDS`] and verified byte-for-byte in `codegen_tests.rs`:
//! 1. `cxx/nve-fabric-bridge/include/nve_scene_abi.h` (C11 / C++20)
//! 2. `hosts/swiftui-kit/Sources/CNveAbi/include/nve_scene_abi.h` (Swift C module)
//! 3. `hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart` (Dart `dart:ffi`)
//! 4. `hosts/android-shell/android-app/src/dev/nve/android/NveSceneBuffer.java` (Android `DirectByteBuffer`)
//! 5. `hosts/web-shell/nve_scene_view.ts` (TypeScript / WASM `DataView`)

use std::fmt::Write as _;

use crate::ir::GeneratedFile;
use crate::naming::to_pascal_case;

/// Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).
pub const NVE_RENDER_NODE_SIZE: usize = 64;
/// Required cache-line alignment of `NveRenderNode`.
pub const NVE_RENDER_NODE_ALIGN: usize = 64;

/// Bit flags on `NveRenderNode::flags`.
pub const NVE_NODE_LEAF_TEXT: u8 = 1 << 0;
pub const NVE_NODE_INTERACTIVE: u8 = 1 << 1;
pub const NVE_NODE_CLIP_BOUNDS: u8 = 1 << 2;

/// Canonical scalar/pointer field type inside the scene ABI schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneFieldType {
    U8,
    U16,
    U32,
    I32,
    U64,
    F32,
    PtrConstRenderNode,
    PtrConstU8,
    PtrConstU32,
}

impl SceneFieldType {
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::I32 => "i32",
            Self::U64 => "u64",
            Self::F32 => "f32",
            Self::PtrConstRenderNode => "*const NveRenderNode",
            Self::PtrConstU8 => "*const u8",
            Self::PtrConstU32 => "*const u32",
        }
    }

    pub const fn c_type(self) -> &'static str {
        match self {
            Self::U8 => "uint8_t",
            Self::U16 => "uint16_t",
            Self::U32 => "uint32_t",
            Self::I32 => "int32_t",
            Self::U64 => "uint64_t",
            Self::F32 => "float",
            Self::PtrConstRenderNode => "const NveRenderNode*",
            Self::PtrConstU8 => "const uint8_t*",
            Self::PtrConstU32 => "const uint32_t*",
        }
    }

    pub const fn fixed_byte_size(self) -> Option<usize> {
        match self {
            Self::U8 => Some(1),
            Self::U16 => Some(2),
            Self::U32 | Self::I32 | Self::F32 => Some(4),
            Self::U64 => Some(8),
            Self::PtrConstRenderNode | Self::PtrConstU8 | Self::PtrConstU32 => None,
        }
    }

    pub const fn dart_ffi_annotation(self) -> Option<&'static str> {
        match self {
            Self::U8 => Some("@ffi.Uint8()"),
            Self::U16 => Some("@ffi.Uint16()"),
            Self::U32 => Some("@ffi.Uint32()"),
            Self::I32 => Some("@ffi.Int32()"),
            Self::U64 => Some("@ffi.Uint64()"),
            Self::F32 => Some("@ffi.Float()"),
            Self::PtrConstRenderNode | Self::PtrConstU8 | Self::PtrConstU32 => None,
        }
    }

    pub const fn dart_field_type(self) -> &'static str {
        match self {
            Self::U8 | Self::U16 | Self::U32 | Self::I32 | Self::U64 => "int",
            Self::F32 => "double",
            Self::PtrConstRenderNode => "ffi.Pointer<NveRenderNode>",
            Self::PtrConstU8 => "ffi.Pointer<ffi.Uint8>",
            Self::PtrConstU32 => "ffi.Pointer<ffi.Uint32>",
        }
    }
}

/// One field in the canonical scene ABI schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneFieldDesc {
    pub name: &'static str,
    pub ty: SceneFieldType,
    /// Fixed byte offset for fixed-size scalar fields (`usize::MAX` for pointer fields whose
    /// offset depends on target pointer width).
    pub offset: usize,
    pub doc: &'static str,
}

/// Canonical 64-byte `NveRenderNode` layout (mirrors `crates/nve-abi/src/scene.rs` verbatim).
pub const NVE_RENDER_NODE_FIELDS: &[SceneFieldDesc] = &[
    SceneFieldDesc {
        name: "node_id",
        ty: SceneFieldType::U32,
        offset: 0,
        doc: "Stable node identifier (`NVE_ROOT_NODE_ID == 1`).",
    },
    SceneFieldDesc {
        name: "kind",
        ty: SceneFieldType::U8,
        offset: 4,
        doc: "Compact `NVE_NODE_KIND_*` component discriminant.",
    },
    SceneFieldDesc {
        name: "flags",
        ty: SceneFieldType::U8,
        offset: 5,
        doc: "Bitmask of `NVE_NODE_LEAF_TEXT`, `NVE_NODE_INTERACTIVE`, `NVE_NODE_CLIP_BOUNDS`.",
    },
    SceneFieldDesc {
        name: "opacity_u8",
        ty: SceneFieldType::U8,
        offset: 6,
        doc: "Normalized opacity quantized to `0..=255` (`255 == 1.0`).",
    },
    SceneFieldDesc {
        name: "_pad0",
        ty: SceneFieldType::U8,
        offset: 7,
        doc: "Explicit zero padding byte.",
    },
    SceneFieldDesc {
        name: "x",
        ty: SceneFieldType::F32,
        offset: 8,
        doc: "Parent-relative X origin in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "y",
        ty: SceneFieldType::F32,
        offset: 12,
        doc: "Parent-relative Y origin in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "width",
        ty: SceneFieldType::F32,
        offset: 16,
        doc: "Computed layout width in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "height",
        ty: SceneFieldType::F32,
        offset: 20,
        doc: "Computed layout height in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "bg_rgba",
        ty: SceneFieldType::U32,
        offset: 24,
        doc: "Packed RGBA8888 background color (`r | (g<<8) | (b<<16) | (a<<24)`).",
    },
    SceneFieldDesc {
        name: "text_rgba",
        ty: SceneFieldType::U32,
        offset: 28,
        doc: "Packed RGBA8888 foreground text color.",
    },
    SceneFieldDesc {
        name: "border_rgba",
        ty: SceneFieldType::U32,
        offset: 32,
        doc: "Packed RGBA8888 border stroke color.",
    },
    SceneFieldDesc {
        name: "border_radius",
        ty: SceneFieldType::F32,
        offset: 36,
        doc: "Corner radius in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "border_width",
        ty: SceneFieldType::F32,
        offset: 40,
        doc: "Border stroke thickness in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "font_size",
        ty: SceneFieldType::F32,
        offset: 44,
        doc: "Font size in logical points / DIPs.",
    },
    SceneFieldDesc {
        name: "text_offset",
        ty: SceneFieldType::U32,
        offset: 48,
        doc: "Byte offset into `text_slab`.",
    },
    SceneFieldDesc {
        name: "text_len",
        ty: SceneFieldType::U16,
        offset: 52,
        doc: "UTF-8 byte length inside `text_slab`.",
    },
    SceneFieldDesc {
        name: "child_count",
        ty: SceneFieldType::U16,
        offset: 54,
        doc: "Number of child `u32` IDs inside `children_slab`.",
    },
    SceneFieldDesc {
        name: "child_offset",
        ty: SceneFieldType::U32,
        offset: 56,
        doc: "Element offset (in `u32` entries) into `children_slab`.",
    },
    SceneFieldDesc {
        name: "z_index",
        ty: SceneFieldType::I32,
        offset: 60,
        doc: "Stacking z-index order.",
    },
];

/// Canonical `NveSceneView` header + slab pointers layout.
pub const NVE_SCENE_VIEW_FIELDS: &[SceneFieldDesc] = &[
    SceneFieldDesc {
        name: "schema_hash",
        ty: SceneFieldType::U64,
        offset: 0,
        doc: "Must equal `NVE_SCENE_SCHEMA_HASH`.",
    },
    SceneFieldDesc {
        name: "frame_id",
        ty: SceneFieldType::U64,
        offset: 8,
        doc: "Monotonic frame counter.",
    },
    SceneFieldDesc {
        name: "node_count",
        ty: SceneFieldType::U32,
        offset: 16,
        doc: "Total live nodes in `nodes`.",
    },
    SceneFieldDesc {
        name: "dirty_count",
        ty: SceneFieldType::U32,
        offset: 20,
        doc: "Count of changed node indices in `dirty_indices`.",
    },
    SceneFieldDesc {
        name: "removed_count",
        ty: SceneFieldType::U32,
        offset: 24,
        doc: "Count of removed node IDs in `removed_ids`.",
    },
    SceneFieldDesc {
        name: "text_slab_len",
        ty: SceneFieldType::U32,
        offset: 28,
        doc: "Byte length of `text_slab`.",
    },
    SceneFieldDesc {
        name: "children_slab_len",
        ty: SceneFieldType::U32,
        offset: 32,
        doc: "Element count (`u32` entries) of `children_slab`.",
    },
    SceneFieldDesc {
        name: "_pad",
        ty: SceneFieldType::U32,
        offset: 36,
        doc: "Explicit 8-byte pointer alignment padding.",
    },
    SceneFieldDesc {
        name: "nodes",
        ty: SceneFieldType::PtrConstRenderNode,
        offset: usize::MAX,
        doc: "Contiguous array of `node_count` `NveRenderNode` entries.",
    },
    SceneFieldDesc {
        name: "text_slab",
        ty: SceneFieldType::PtrConstU8,
        offset: usize::MAX,
        doc: "Contiguous UTF-8 text byte slab.",
    },
    SceneFieldDesc {
        name: "children_slab",
        ty: SceneFieldType::PtrConstU32,
        offset: usize::MAX,
        doc: "Contiguous `u32` child node ID slab.",
    },
    SceneFieldDesc {
        name: "dirty_indices",
        ty: SceneFieldType::PtrConstU32,
        offset: usize::MAX,
        doc: "Indices into `nodes` modified during `frame_id`.",
    },
    SceneFieldDesc {
        name: "removed_ids",
        ty: SceneFieldType::PtrConstU32,
        offset: usize::MAX,
        doc: "Node IDs removed during `frame_id`.",
    },
];

// Compile-time check that `NVE_RENDER_NODE_FIELDS` packs tightly into exactly 64 bytes.
const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < NVE_RENDER_NODE_FIELDS.len() {
        let f = NVE_RENDER_NODE_FIELDS[i];
        assert!(f.offset == cursor);
        let sz = match f.ty.fixed_byte_size() {
            Some(s) => s,
            None => panic!("NveRenderNode cannot contain pointer fields"),
        };
        assert!(cursor % sz == 0);
        cursor += sz;
        i += 1;
    }
    assert!(cursor == NVE_RENDER_NODE_SIZE);
};

// ============================================================================
// Deterministic FNV-1a 64-bit Schema Hash (identical to `crates/nve-abi/src/scene.rs`)
// ============================================================================

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

const fn fnv1a_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(FNV1A_PRIME);
        i += 1;
    }
    h
}

const fn fnv1a_u64(h: u64, v: u64) -> u64 {
    fnv1a_bytes(h, &v.to_le_bytes())
}

const fn compute_schema_hash() -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv1a_bytes(h, b"NveRenderNode");
    h = fnv1a_u64(h, NVE_RENDER_NODE_SIZE as u64);

    let mut i = 0;
    while i < NVE_RENDER_NODE_FIELDS.len() {
        let f = NVE_RENDER_NODE_FIELDS[i];
        h = fnv1a_bytes(h, b"|");
        h = fnv1a_bytes(h, f.name.as_bytes());
        h = fnv1a_bytes(h, b":");
        h = fnv1a_bytes(h, f.ty.canonical_name().as_bytes());
        h = fnv1a_bytes(h, b"@");
        h = fnv1a_u64(h, f.offset as u64);
        i += 1;
    }

    h = fnv1a_bytes(h, b";NveSceneView");
    let mut j = 0;
    while j < NVE_SCENE_VIEW_FIELDS.len() {
        let f = NVE_SCENE_VIEW_FIELDS[j];
        h = fnv1a_bytes(h, b"|");
        h = fnv1a_bytes(h, f.name.as_bytes());
        h = fnv1a_bytes(h, b":");
        h = fnv1a_bytes(h, f.ty.canonical_name().as_bytes());
        if f.offset != usize::MAX {
            h = fnv1a_bytes(h, b"@");
            h = fnv1a_u64(h, f.offset as u64);
        } else {
            h = fnv1a_bytes(h, b"#");
            h = fnv1a_u64(h, j as u64);
        }
        j += 1;
    }
    h
}

/// Deterministic FNV-1a 64-bit schema fingerprint shared verbatim across Rust, C/C++, Swift, Dart,
/// Java/Android, and TypeScript/WASM.
pub const NVE_SCENE_SCHEMA_HASH: u64 = compute_schema_hash();

fn snake_to_camel(s: &str) -> String {
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

// ============================================================================
// Target 1 & 2: C11 / C++20 / Swift Header (`nve_scene_abi.h`)
// ============================================================================

/// Generates the deterministic C11/C++20/Swift `nve_scene_abi.h` header with compile-time
/// `sizeof` / `alignof` / `offsetof` assertions on every field.
pub fn generate_c_scene_abi_h() -> String {
    let mut out = String::with_capacity(4096);
    let _ = writeln!(out, "/* @generated by nve-codegen (`abi_schema`) — DO NOT EDIT MANUALLY. */");
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
    let _ = writeln!(out, "#define NVE_SCENE_STATIC_ASSERT(cond, msg) static_assert(cond, msg)");
    let _ = writeln!(out, "#else");
    let _ = writeln!(out, "#define NVE_SCENE_STATIC_ASSERT(cond, msg) _Static_assert(cond, msg)");
    let _ = writeln!(out, "#endif");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "#define NVE_SCENE_SCHEMA_HASH 0x{NVE_SCENE_SCHEMA_HASH:016X}ULL"
    );
    let _ = writeln!(out, "#define NVE_RENDER_NODE_SIZE {NVE_RENDER_NODE_SIZE}u");
    let _ = writeln!(out, "#define NVE_RENDER_NODE_ALIGN {NVE_RENDER_NODE_ALIGN}u");
    let _ = writeln!(out);
    let _ = writeln!(out, "/* NveRenderNode::flags bits */");
    let _ = writeln!(out, "#define NVE_NODE_LEAF_TEXT 1u");
    let _ = writeln!(out, "#define NVE_NODE_INTERACTIVE 2u");
    let _ = writeln!(out, "#define NVE_NODE_CLIP_BOUNDS 4u");
    let _ = writeln!(out);
    let _ = writeln!(out, "/* Compact uint8_t NveRenderNode::kind discriminants */");
    let _ = writeln!(out, "#define NVE_NODE_KIND_ROOT 0u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_VIEW 1u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_TEXT 2u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_SCROLL_VIEW 3u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_IMAGE 4u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_TEXT_INPUT 5u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_PRESSABLE 6u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_SKIA_CANVAS 7u");
    let _ = writeln!(out, "#define NVE_NODE_KIND_CUSTOM 255u");
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
    for field in NVE_RENDER_NODE_FIELDS {
        let _ = writeln!(
            out,
            "  {} {}; /* offset {} */",
            field.ty.c_type(),
            field.name,
            field.offset
        );
    }
    let _ = writeln!(out, "}} NveRenderNode;");
    let _ = writeln!(out);
    let _ = writeln!(out, "typedef struct NveSceneView {{");
    for field in NVE_SCENE_VIEW_FIELDS {
        let _ = writeln!(out, "  {} {};", field.ty.c_type(), field.name);
    }
    let _ = writeln!(out, "}} NveSceneView;");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "NVE_SCENE_STATIC_ASSERT(sizeof(NveRenderNode) == {NVE_RENDER_NODE_SIZE}u, \"NveRenderNode must be 64 bytes\");"
    );
    for field in NVE_RENDER_NODE_FIELDS {
        let _ = writeln!(
            out,
            "NVE_SCENE_STATIC_ASSERT(offsetof(NveRenderNode, {}) == {}u, \"NveRenderNode.{} offset mismatch\");",
            field.name, field.offset, field.name
        );
    }
    for field in NVE_SCENE_VIEW_FIELDS {
        if field.offset != usize::MAX {
            let _ = writeln!(
                out,
                "NVE_SCENE_STATIC_ASSERT(offsetof(NveSceneView, {}) == {}u, \"NveSceneView.{} offset mismatch\");",
                field.name, field.offset, field.name
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
// Target 3: Dart `dart:ffi` (`hosts/flutter-plugin/lib/src/nve_scene_abi_gen.dart`)
// ============================================================================

/// Generates the deterministic Dart `dart:ffi` structs and zero-copy `NveSceneReader`.
pub fn generate_dart_scene_abi() -> String {
    let mut out = String::with_capacity(6144);
    let signed_hash = NVE_SCENE_SCHEMA_HASH as i64;

    let _ = writeln!(
        out,
        "// @generated by nve-codegen (`abi_schema`) — DO NOT EDIT MANUALLY."
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
        "/// Deterministic FNV-1a 64-bit schema hash (`0x{NVE_SCENE_SCHEMA_HASH:016X}`)."
    );
    let _ = writeln!(
        out,
        "/// Stored as 64-bit two's-complement `int` matching `@ffi.Uint64()` bit pattern on native VM."
    );
    let _ = writeln!(out, "const int nveSceneSchemaHash = {signed_hash};");
    let _ = writeln!(
        out,
        "const String nveSceneSchemaHashHex = '0x{NVE_SCENE_SCHEMA_HASH:016X}';"
    );
    let _ = writeln!(out, "const int nveRenderNodeStride = {NVE_RENDER_NODE_SIZE};");
    let _ = writeln!(out);
    let _ = writeln!(out, "// NveRenderNode::flags bit constants");
    let _ = writeln!(out, "const int nveNodeLeafText = 1 << 0;");
    let _ = writeln!(out, "const int nveNodeInteractive = 1 << 1;");
    let _ = writeln!(out, "const int nveNodeClipBounds = 1 << 2;");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Compact uint8 NveRenderNode::kind discriminants");
    let _ = writeln!(out, "const int nveNodeKindRoot = 0;");
    let _ = writeln!(out, "const int nveNodeKindView = 1;");
    let _ = writeln!(out, "const int nveNodeKindText = 2;");
    let _ = writeln!(out, "const int nveNodeKindScrollView = 3;");
    let _ = writeln!(out, "const int nveNodeKindImage = 4;");
    let _ = writeln!(out, "const int nveNodeKindTextInput = 5;");
    let _ = writeln!(out, "const int nveNodeKindPressable = 6;");
    let _ = writeln!(out, "const int nveNodeKindSkiaCanvas = 7;");
    let _ = writeln!(out, "const int nveNodeKindCustom = 255;");
    let _ = writeln!(out);
    let _ = writeln!(out, "// Exact byte offsets within one 64-byte NveRenderNode");
    for field in NVE_RENDER_NODE_FIELDS {
        let const_name = format!(
            "nveNodeOffset{}",
            to_pascal_case(field.name.trim_start_matches('_'))
        );
        let _ = writeln!(out, "const int {const_name} = {};", field.offset);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "/// 64-byte zero-decode render node (`NveRenderNode` in `nve_scene_abi.h`).");
    let _ = writeln!(out, "final class NveRenderNode extends ffi.Struct {{");
    for field in NVE_RENDER_NODE_FIELDS {
        let dart_name = if field.name == "_pad0" {
            "pad0".to_owned()
        } else {
            snake_to_camel(field.name)
        };
        if let Some(ann) = field.ty.dart_ffi_annotation() {
            let _ = writeln!(out, "  {ann}");
        }
        let _ = writeln!(
            out,
            "  external {} {dart_name}; // offset {}",
            field.ty.dart_field_type(),
            field.offset
        );
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Zero-decode scene snapshot header + slab pointers (`NveSceneView` in `nve_scene_abi.h`)."
    );
    let _ = writeln!(out, "final class NveSceneView extends ffi.Struct {{");
    for field in NVE_SCENE_VIEW_FIELDS {
        let dart_name = if field.name == "_pad" {
            "pad".to_owned()
        } else {
            snake_to_camel(field.name)
        };
        if let Some(ann) = field.ty.dart_ffi_annotation() {
            let _ = writeln!(out, "  {ann}");
        }
        let _ = writeln!(out, "  external {} {dart_name};", field.ty.dart_field_type());
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// Error thrown when an `NveSceneView` fails schema or bounds validation.");
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
    let _ = writeln!(out, "  /// Zero-copy `Uint32List` view over `node`'s children in `childrenSlab`.");
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
// Target 4: Java / Android DirectByteBuffer (`NveSceneBuffer.java`)
// ============================================================================

/// Generates the deterministic Java/Android `NveSceneBuffer.java` zero-GC `DirectByteBuffer` reader.
pub fn generate_java_scene_buffer() -> String {
    let mut out = String::with_capacity(6144);
    let _ = writeln!(
        out,
        "// @generated by nve-codegen (`abi_schema`) — DO NOT EDIT MANUALLY."
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
        "    public static final long SCHEMA_HASH = 0x{NVE_SCENE_SCHEMA_HASH:016X}L;"
    );
    let _ = writeln!(out, "    public static final int NODE_STRIDE = {NVE_RENDER_NODE_SIZE};");
    let _ = writeln!(out);
    let _ = writeln!(out, "    // NveRenderNode::flags bits");
    let _ = writeln!(out, "    public static final int FLAG_LEAF_TEXT = 1 << 0;");
    let _ = writeln!(out, "    public static final int FLAG_INTERACTIVE = 1 << 1;");
    let _ = writeln!(out, "    public static final int FLAG_CLIP_BOUNDS = 1 << 2;");
    let _ = writeln!(out);
    let _ = writeln!(out, "    // Compact uint8 NveRenderNode::kind discriminants");
    let _ = writeln!(out, "    public static final int KIND_ROOT = 0;");
    let _ = writeln!(out, "    public static final int KIND_VIEW = 1;");
    let _ = writeln!(out, "    public static final int KIND_TEXT = 2;");
    let _ = writeln!(out, "    public static final int KIND_SCROLL_VIEW = 3;");
    let _ = writeln!(out, "    public static final int KIND_IMAGE = 4;");
    let _ = writeln!(out, "    public static final int KIND_TEXT_INPUT = 5;");
    let _ = writeln!(out, "    public static final int KIND_PRESSABLE = 6;");
    let _ = writeln!(out, "    public static final int KIND_SKIA_CANVAS = 7;");
    let _ = writeln!(out, "    public static final int KIND_CUSTOM = 255;");
    let _ = writeln!(out);
    let _ = writeln!(out, "    // Exact byte offsets within one 64-byte NveRenderNode");
    for field in NVE_RENDER_NODE_FIELDS {
        let _ = writeln!(
            out,
            "    public static final int OFFSET_{} = {};",
            snake_to_screaming(field.name),
            field.offset
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
// Target 5: TypeScript / WASM (`hosts/web-shell/nve_scene_view.ts`)
// ============================================================================

/// Generates the deterministic TypeScript/WASM zero-copy `NveSceneViewReader`.
pub fn generate_ts_scene_view() -> String {
    let mut out = String::with_capacity(5120);
    let _ = writeln!(
        out,
        "// @generated by nve-codegen (`abi_schema`) — DO NOT EDIT MANUALLY."
    );
    let _ = writeln!(
        out,
        "// Single-source 64-byte NveRenderNode + NveSceneView zero-copy WASM memory reader."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "export const NVE_SCENE_SCHEMA_HASH: bigint = 0x{NVE_SCENE_SCHEMA_HASH:016X}n;"
    );
    let _ = writeln!(
        out,
        "export const NVE_RENDER_NODE_STRIDE = {NVE_RENDER_NODE_SIZE} as const;"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_FLAG = {{");
    let _ = writeln!(out, "  LEAF_TEXT: 1 << 0,");
    let _ = writeln!(out, "  INTERACTIVE: 1 << 1,");
    let _ = writeln!(out, "  CLIP_BOUNDS: 1 << 2,");
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_KIND = {{");
    let _ = writeln!(out, "  ROOT: 0,");
    let _ = writeln!(out, "  VIEW: 1,");
    let _ = writeln!(out, "  TEXT: 2,");
    let _ = writeln!(out, "  SCROLL_VIEW: 3,");
    let _ = writeln!(out, "  IMAGE: 4,");
    let _ = writeln!(out, "  TEXT_INPUT: 5,");
    let _ = writeln!(out, "  PRESSABLE: 6,");
    let _ = writeln!(out, "  SKIA_CANVAS: 7,");
    let _ = writeln!(out, "  CUSTOM: 255,");
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(out, "export const NVE_NODE_OFFSET = {{");
    for field in NVE_RENDER_NODE_FIELDS {
        let _ = writeln!(
            out,
            "  {}: {},",
            snake_to_screaming(field.name),
            field.offset
        );
    }
    let _ = writeln!(out, "}} as const;");
    let _ = writeln!(out);
    let _ = writeln!(out, "/** Pointer & count descriptor for a WASM `NveSceneView` inside linear memory. */");
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
    let _ = writeln!(out, "const sharedUtf8Decoder = new TextDecoder('utf-8', {{ fatal: true }});");
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

/// Generates all 5 checked-in scene ABI files keyed by workspace-relative path.
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
