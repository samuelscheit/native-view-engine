//! Single-source-of-truth `const` ABI metadata descriptors and colocated schema traits
//! (`NveSchema`, `NveStructSchema`, `NveUnionSchema`, `NveEnumSchema`) populated directly by
//! `#[derive(NveAbi)]` on the real compiled `#[repr(C)]` types in `nve-abi`.
//!
//! All descriptors are `const` / dead-code-eliminated in runtime binaries (`wasm32`, native apps)
//! and consumed directly by `nve-codegen` to emit all checked-in C11/C++20, Swift, Dart, Java,
//! and TypeScript ABI bindings with zero duplicated spec files.

use crate::ffi::{
    ComponentKind, NativeEventKind, NveBufferSlice, NveEventPayloadTag, NveFrameStats,
    NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints, NveSurfaceConfig, Opcode,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use crate::host::{NveHostCallbacks, NVE_CHANNEL_FLAG_RETAIN};
use crate::scene::{
    NveRenderNode, NveSceneView, NVE_NODE_CLIP_BOUNDS, NVE_NODE_INTERACTIVE, NVE_NODE_KIND_CUSTOM,
    NVE_NODE_KIND_IMAGE, NVE_NODE_KIND_PRESSABLE, NVE_NODE_KIND_ROOT, NVE_NODE_KIND_SCROLL_VIEW,
    NVE_NODE_KIND_SKIA_CANVAS, NVE_NODE_KIND_TEXT, NVE_NODE_KIND_TEXT_INPUT, NVE_NODE_KIND_VIEW,
    NVE_NODE_LEAF_TEXT, NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE,
};
use crate::value::{
    NveCommandSlice, NveDirectEvent, NveEventBatch, NveMapEntry, NvePropEntry, NvePropSlice,
    NveStrSlice, NveValue, NveValueArray, NveValueData, NveValueMap, NveValueTag,
};
use crate::ROOT_NODE_ID;

pub use nve_abi_macros::NveAbi;

/// Compile-time metadata for one named field of a `#[repr(C)]` struct or union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiFieldDescriptor {
    pub name: &'static str,
    pub ty: &'static str,
    pub c_type: Option<&'static str>,
    pub doc: Option<&'static str>,
    /// Real byte offset computed directly by `rustc` via `core::mem::offset_of!`.
    pub offset: usize,
    /// Optional target-independent offset contract (`#[nve_abi(offset = N)]`).
    pub expected_offset: Option<usize>,
    /// Optional 64-bit target offset contract (`#[nve_abi(offset64 = N)]`).
    pub expected_offset_64: Option<usize>,
}

impl AbiFieldDescriptor {
    /// Returns the explicit `#[nve_abi(c_type = "...")]` override if set, else the Rust field type.
    #[inline]
    pub const fn abi_type(&self) -> &'static str {
        match self.c_type {
            Some(ct) => ct,
            None => self.ty,
        }
    }
}

/// Compile-time metadata for a `#[repr(C)]` struct derived via `#[derive(NveAbi)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiStructDescriptor {
    pub name: &'static str,
    pub doc: Option<&'static str>,
    pub size: usize,
    pub align: usize,
    pub explicit_align: Option<usize>,
    pub expected_size: Option<usize>,
    pub expected_size_64: Option<usize>,
    pub fields: &'static [AbiFieldDescriptor],
}

/// Compile-time metadata for a `#[repr(C)]` union derived via `#[derive(NveAbi)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiUnionDescriptor {
    pub name: &'static str,
    pub doc: Option<&'static str>,
    pub size: usize,
    pub align: usize,
    pub explicit_align: Option<usize>,
    pub expected_size: Option<usize>,
    pub expected_size_64: Option<usize>,
    pub fields: &'static [AbiFieldDescriptor],
}

/// Compile-time metadata for one variant of a `#[repr(u8|u16|u32|i32)]` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiEnumVariantDescriptor {
    pub name: &'static str,
    pub value: i64,
    pub doc: Option<&'static str>,
}

/// Compile-time metadata for a `#[repr(u8|u16|u32|i32)]` enum derived via `#[derive(NveAbi)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiEnumDescriptor {
    pub name: &'static str,
    pub repr: &'static str,
    pub doc: Option<&'static str>,
    pub c_prefix: Option<&'static str>,
    pub dart_fallback: Option<&'static str>,
    pub variants: &'static [AbiEnumVariantDescriptor],
}

/// Unified descriptor tag exposed by [`NveSchema`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiItemDescriptor {
    Struct(AbiStructDescriptor),
    Union(AbiUnionDescriptor),
    Enum(AbiEnumDescriptor),
}

/// Ordered struct or union entry inside [`AbiFileDescriptor::records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiRecordDescriptor {
    Struct(AbiStructDescriptor),
    Union(AbiUnionDescriptor),
}

/// Compile-time constant descriptor (e.g. wire magic, ABI version, flag bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiConstDescriptor {
    pub name: &'static str,
    pub ty: &'static str,
    pub value: u64,
    pub hex: bool,
    pub doc: Option<&'static str>,
}

/// Compile-time opaque handle or function-pointer typedef descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiAliasDescriptor {
    pub name: &'static str,
    pub target: &'static str,
    pub doc: Option<&'static str>,
}

/// Named parameter of a vtable method or C API function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiParamDescriptor {
    pub name: &'static str,
    pub ty: &'static str,
}

/// One slot in a modular C ABI interface vtable (e.g. `NveSurfaceApi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiMethodDescriptor {
    pub name: &'static str,
    pub params: &'static [AbiParamDescriptor],
    pub return_type: &'static str,
    pub doc: Option<&'static str>,
    pub has_self: bool,
    pub optional: bool,
}

/// Modular `nve_get_interface(id)` vtable contract descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiModuleDescriptor {
    pub name: &'static str,
    pub trait_name: &'static str,
    pub interface_id: u32,
    pub c_name: &'static str,
    pub doc: Option<&'static str>,
    pub methods: &'static [AbiMethodDescriptor],
}

/// Direct `extern "C"` function declaration descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiCFunctionDescriptor {
    pub name: &'static str,
    pub params: &'static [AbiParamDescriptor],
    pub return_type: &'static str,
    pub doc: Option<&'static str>,
}

/// Complete colocated ABI file descriptor consumed directly by `nve-codegen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiFileDescriptor {
    pub namespace: Option<&'static str>,
    pub consts: &'static [AbiConstDescriptor],
    pub aliases: &'static [AbiAliasDescriptor],
    pub enums: &'static [AbiEnumDescriptor],
    pub records: &'static [AbiRecordDescriptor],
    pub modules: &'static [AbiModuleDescriptor],
    pub c_functions: &'static [AbiCFunctionDescriptor],
}

/// Unified schema trait implemented by `#[derive(NveAbi)]` on structs, unions, and enums.
pub trait NveSchema {
    const ITEM: AbiItemDescriptor;
}

/// Schema trait implemented by `#[derive(NveAbi)]` on `#[repr(C)]` structs.
pub trait NveStructSchema: NveSchema {
    const SCHEMA: AbiStructDescriptor;
}

/// Schema trait implemented by `#[derive(NveAbi)]` on `#[repr(C)]` unions.
pub trait NveUnionSchema: NveSchema {
    const SCHEMA: AbiUnionDescriptor;
}

/// Schema trait implemented by `#[derive(NveAbi)]` on `#[repr(u8|u16|u32|i32)]` enums.
pub trait NveEnumSchema: NveSchema {
    const SCHEMA: AbiEnumDescriptor;
}

// Compile-time cross-check of wire-gated constants when `feature = "wire"` is enabled.
#[cfg(feature = "wire")]
const _: () = {
    assert!(crate::codec::NVE_ABI_MAGIC == 0x3145_564E);
    assert!(crate::codec::NVE_ABI_VERSION == 1);
    assert!(crate::value::NVE_BIN_VALUE_MAX_DEPTH == 64);
    assert!(u32::from_le_bytes(crate::host::NVE_HOST_EVENT_MAGIC) == 0x3148_564E);
    assert!(crate::host::NVE_HOST_EVENT_VERSION == 1);
};

const fn abi_const(name: &'static str, value: u64, hex: bool, doc: Option<&'static str>) -> AbiConstDescriptor {
    AbiConstDescriptor { name, ty: "u32", value, hex, doc }
}

const fn abi_param(name: &'static str, ty: &'static str) -> AbiParamDescriptor {
    AbiParamDescriptor { name, ty }
}

const fn abi_method(
    name: &'static str,
    params: &'static [AbiParamDescriptor],
    return_type: &'static str,
    has_self: bool,
    optional: bool,
    doc: Option<&'static str>,
) -> AbiMethodDescriptor {
    AbiMethodDescriptor { name, params, return_type, doc, has_self, optional }
}

const CORE_CONSTS: &[AbiConstDescriptor] = &[
    abi_const("NVE_ABI_MAGIC", u32::from_le_bytes(*b"NVE1") as u64, true, Some("Wire magic bytes (`\"NVE1\"` little-endian).")),
    abi_const("NVE_ABI_VERSION", 1, false, Some("Binary `CommandBuffer` ABI version.")),
    abi_const("NVE_ROOT_NODE_ID", ROOT_NODE_ID as u64, false, Some("Reserved root node ID for every surface.")),
    abi_const("NVE_BIN_VALUE_MAX_DEPTH", 64, false, Some("Maximum permitted container nesting depth for `NveBinValue`.")),
    abi_const("NVE_HOST_EVENT_MAGIC", u32::from_le_bytes(*b"NVH1") as u64, true, Some("Binary `NveHostEventBatch` wire magic (`\"NVH1\"` little-endian).")),
    abi_const("NVE_HOST_EVENT_VERSION", 1, false, Some("Binary `NveHostEventBatch` wire version.")),
    abi_const("NVE_CHANNEL_FLAG_RETAIN", NVE_CHANNEL_FLAG_RETAIN as u64, false, Some("Flag bit on `channel_publish` / `ChannelMessage` requesting retained channel state.")),
];

const CORE_ALIASES: &[AbiAliasDescriptor] = &[
    AbiAliasDescriptor { name: "NveAppHandle", target: "*mut c_void", doc: Some("Opaque handle to an instantiated NVE app/component.") },
    AbiAliasDescriptor { name: "NveFabricHostHandle", target: "*mut c_void", doc: Some("Opaque handle to a C++20 `nve-fabric-bridge` host instance.") },
    AbiAliasDescriptor { name: "NveHostInvokeFn", target: "*const c_void", doc: Some("Host capability invocation callback function pointer (`NveHostCallbacks::invoke`).") },
    AbiAliasDescriptor { name: "NveHostChannelPublishFn", target: "*const c_void", doc: Some("Host channel-bus publish callback function pointer (`NveHostCallbacks::channel_publish`).") },
];

const CORE_ENUMS: &[AbiEnumDescriptor] = &[
    <Opcode as NveEnumSchema>::SCHEMA,
    <ComponentKind as NveEnumSchema>::SCHEMA,
    <NveGestureDisposition as NveEnumSchema>::SCHEMA,
    <NveValueTag as NveEnumSchema>::SCHEMA,
    <NativeEventKind as NveEnumSchema>::SCHEMA,
    <NveEventPayloadTag as NveEnumSchema>::SCHEMA,
];

const CORE_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveBufferSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveStrSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValueArray as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValueMap as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Union(<NveValueData as NveUnionSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValue as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveMapEntry as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveDirectEvent as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveEventBatch as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NvePropEntry as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NvePropSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveCommandSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSurfaceConfig as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveFrameStats as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSizeConstraints as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveIntrinsicSize as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveHostCallbacks as NveStructSchema>::SCHEMA),
];

const CORE_MODULES: &[AbiModuleDescriptor] = &[
    AbiModuleDescriptor {
        name: "Surface",
        trait_name: "NveSurfaceApiSpec",
        interface_id: NVE_IFACE_SURFACE,
        c_name: "NveSurfaceApi",
        doc: Some("Required core surface interface (`NVE_IFACE_SURFACE = 1`)."),
        methods: &[
            abi_method("create", &[abi_param("surface_id", "u32")], "NveAppHandle", false, false, None),
            abi_method("destroy", &[], "()", true, false, None),
            abi_method("render", &[abi_param("frame_id", "u64")], "NveBufferSlice", true, false, None),
            abi_method("dispatch_events", &[abi_param("events", "NveBufferSlice")], "u32", true, false, None),
            abi_method("dispatch_events_direct", &[abi_param("batch", "NveEventBatch")], "u32", true, true, Some("Optional zero-copy direct event dispatch (may be NULL).")),
        ],
    },
    AbiModuleDescriptor {
        name: "Layout",
        trait_name: "NveLayoutApiSpec",
        interface_id: NVE_IFACE_LAYOUT,
        c_name: "NveLayoutApi",
        doc: Some("Optional layout & gesture interface (`NVE_IFACE_LAYOUT = 2`). Zero-serialization register calls."),
        methods: &[
            abi_method("measure", &[abi_param("constraints", "NveSizeConstraints")], "NveIntrinsicSize", true, false, None),
            abi_method("hit_test", &[abi_param("x", "f32"), abi_param("y", "f32"), abi_param("pointer_phase", "u8")], "u8", true, false, None),
        ],
    },
    AbiModuleDescriptor {
        name: "Component",
        trait_name: "NveComponentApiSpec",
        interface_id: NVE_IFACE_COMPONENT,
        c_name: "NveComponentApi",
        doc: Some("Optional embedded component props & outbound event interface (`NVE_IFACE_COMPONENT = 3`)."),
        methods: &[
            abi_method("set_props", &[abi_param("props", "NveBufferSlice")], "u32", true, false, None),
            abi_method("drain_emitted_events", &[], "NveBufferSlice", true, false, None),
            abi_method("set_props_direct", &[abi_param("entries", "*const NveMapEntry"), abi_param("count", "u32")], "u32", true, true, Some("Optional zero-copy structured props update (may be NULL).")),
        ],
    },
    AbiModuleDescriptor {
        name: "Dev",
        trait_name: "NveDevApiSpec",
        interface_id: NVE_IFACE_DEV,
        c_name: "NveDevApi",
        doc: Some("Optional dev / 2-tier hot-reload interface (`NVE_IFACE_DEV = 4`, may be NULL in release builds)."),
        methods: &[
            abi_method(
                "apply_hot_patch",
                &[abi_param("patch_nvp1", "NveBufferSlice")],
                "u32",
                true,
                false,
                Some("Applies one binary `NVP1` `HotPatchMsg` frame exactly as broadcast by `nve dev`\n(returns 1 when the app changed; 0 for non-template or malformed frames)."),
            ),
            abi_method("snapshot_state", &[], "NveBufferSlice", true, false, None),
            abi_method("restore_state", &[abi_param("state", "NveBufferSlice")], "()", true, false, None),
        ],
    },
    AbiModuleDescriptor {
        name: "Host",
        trait_name: "NveHostApiSpec",
        interface_id: NVE_IFACE_HOST,
        c_name: "NveHostApi",
        doc: Some("Optional zero-JSON host-capability & channel-bus interface (`NVE_IFACE_HOST = 5`)."),
        methods: &[
            abi_method("bind_host", &[abi_param("callbacks", "*const NveHostCallbacks")], "()", true, false, None),
            abi_method("dispatch_host_events", &[abi_param("batch", "NveBufferSlice")], "u32", true, false, None),
        ],
    },
];

const CORE_C_FUNCTIONS: &[AbiCFunctionDescriptor] = &[
    AbiCFunctionDescriptor {
        name: "nve_fabric_host_create",
        params: &[abi_param("config", "NveSurfaceConfig")],
        return_type: "NveFabricHostHandle",
        doc: Some("C API implemented by `cxx/nve-fabric-bridge` for iOS, macOS, Android, and headless Fabric tests."),
    },
    AbiCFunctionDescriptor { name: "nve_fabric_host_destroy", params: &[abi_param("host", "NveFabricHostHandle")], return_type: "()", doc: None },
    AbiCFunctionDescriptor {
        name: "nve_fabric_host_submit_frame",
        params: &[
            abi_param("host", "NveFabricHostHandle"),
            abi_param("command_buffer", "NveBufferSlice"),
            abi_param("out_stats", "*mut NveFrameStats"),
        ],
        return_type: "i32",
        doc: None,
    },
    AbiCFunctionDescriptor { name: "nve_fabric_host_drain_events", params: &[abi_param("host", "NveFabricHostHandle")], return_type: "NveBufferSlice", doc: None },
    AbiCFunctionDescriptor { name: "nve_fabric_host_node_count", params: &[abi_param("host", "NveFabricHostHandle")], return_type: "u32", doc: None },
];

const SCENE_CONSTS: &[AbiConstDescriptor] = &[
    abi_const("NVE_RENDER_NODE_SIZE", NVE_RENDER_NODE_SIZE as u64, false, Some("Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).")),
    abi_const("NVE_RENDER_NODE_ALIGN", NVE_RENDER_NODE_ALIGN as u64, false, Some("Required cache-line alignment of `NveRenderNode` backing slabs in Rust.")),
    abi_const("NVE_NODE_LEAF_TEXT", NVE_NODE_LEAF_TEXT as u64, false, Some("Bit flags on `NveRenderNode::flags`.")),
    abi_const("NVE_NODE_INTERACTIVE", NVE_NODE_INTERACTIVE as u64, false, None),
    abi_const("NVE_NODE_CLIP_BOUNDS", NVE_NODE_CLIP_BOUNDS as u64, false, None),
    abi_const("NVE_NODE_KIND_ROOT", NVE_NODE_KIND_ROOT as u64, false, Some("Compact `uint8_t` `NveRenderNode::kind` discriminants.")),
    abi_const("NVE_NODE_KIND_VIEW", NVE_NODE_KIND_VIEW as u64, false, None),
    abi_const("NVE_NODE_KIND_TEXT", NVE_NODE_KIND_TEXT as u64, false, None),
    abi_const("NVE_NODE_KIND_SCROLL_VIEW", NVE_NODE_KIND_SCROLL_VIEW as u64, false, None),
    abi_const("NVE_NODE_KIND_IMAGE", NVE_NODE_KIND_IMAGE as u64, false, None),
    abi_const("NVE_NODE_KIND_TEXT_INPUT", NVE_NODE_KIND_TEXT_INPUT as u64, false, None),
    abi_const("NVE_NODE_KIND_PRESSABLE", NVE_NODE_KIND_PRESSABLE as u64, false, None),
    abi_const("NVE_NODE_KIND_SKIA_CANVAS", NVE_NODE_KIND_SKIA_CANVAS as u64, false, None),
    abi_const("NVE_NODE_KIND_CUSTOM", NVE_NODE_KIND_CUSTOM as u64, false, None),
];

const SCENE_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveRenderNode as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSceneView as NveStructSchema>::SCHEMA),
];

/// Returns the single-source-of-truth Core C ABI descriptor derived directly from the real
/// compiled `#[repr(C)]` types in `packages/core/abi`.
pub const fn core_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveCoreAbi"),
        consts: CORE_CONSTS,
        aliases: CORE_ALIASES,
        enums: CORE_ENUMS,
        records: CORE_RECORDS,
        modules: CORE_MODULES,
        c_functions: CORE_C_FUNCTIONS,
    }
}

/// Returns the single-source-of-truth 64-byte Scene ABI descriptor derived directly from the real
/// compiled `NveRenderNode` and `NveSceneView` structs in `packages/core/abi`.
pub const fn scene_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveSceneAbi"),
        consts: SCENE_CONSTS,
        aliases: &[],
        enums: &[],
        records: SCENE_RECORDS,
        modules: &[],
        c_functions: &[],
    }
}
