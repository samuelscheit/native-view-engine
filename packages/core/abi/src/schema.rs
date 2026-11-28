//! Single-source-of-truth `const` ABI metadata descriptors and colocated schema traits
//! (`NveSchema`, `NveStructSchema`, `NveUnionSchema`, `NveEnumSchema`) populated directly by
//! `#[derive(NveAbi)]` on the real compiled `#[repr(C)]` types in `nve-abi`.
//!
//! All descriptors are `const` / dead-code-eliminated in runtime binaries (`wasm32`, native apps)
//! and consumed directly by `nve-codegen` to emit all checked-in C11/C++20, Swift, Dart, Java,
//! and TypeScript ABI bindings with zero duplicated spec files.

use crate::event::{
    InteractionEvent, LayoutEvent, NveEvent, NveEventBatch, RawEventBytes, ScrollEvent,
    TextInputEvent, NVE_EVENT_ALIGN, NVE_EVENT_INLINE_KEY_CAP, NVE_EVENT_SIZE, NVE_TABLE_CUSTOM,
    NVE_TABLE_INTERACTION, NVE_TABLE_LAYOUT, NVE_TABLE_SCROLL, NVE_TABLE_TEXT_INPUT,
};
use crate::ffi::{
    NveBufferSlice, NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints,
    NVE_IFACE_COMPONENT, NVE_IFACE_DEV, NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use crate::host::{NveHostCallbacks, NVE_CHANNEL_FLAG_RETAIN};
use crate::value::{
    NveCommandSlice, NveMapEntry, NvePropEntry, NvePropSlice, NveStrSlice, NveValue,
    NveValueArray, NveValueData, NveValueMap, NveValueTag,
};
use crate::{EventFlags, InterfaceId, ROOT_NODE_ID};

pub use crate::runtime::runtime_abi_descriptor;
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
    pub interface_id: InterfaceId,
    pub c_name: &'static str,
    pub doc: Option<&'static str>,
    pub methods: &'static [AbiMethodDescriptor],
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
    assert!(crate::value::NVE_BIN_VALUE_MAX_DEPTH == 64);
};

const fn abi_const(name: &'static str, value: u64, hex: bool, doc: Option<&'static str>) -> AbiConstDescriptor {
    AbiConstDescriptor { name, ty: "u32", value, hex, doc }
}

const fn abi_const_u64(name: &'static str, value: u64, hex: bool, doc: Option<&'static str>) -> AbiConstDescriptor {
    AbiConstDescriptor { name, ty: "u64", value, hex, doc }
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
    abi_const("NVE_ABI_MAGIC", u32::from_le_bytes(*b"NVE2") as u64, true, Some("Wire magic bytes (`\"NVE2\"` little-endian).")),
    abi_const("NVE_ABI_VERSION", 1, false, Some("Binary `NVE2` wire protocol version.")),
    abi_const("NVE_ROOT_NODE_ID", ROOT_NODE_ID.raw() as u64, false, Some("Reserved root node ID for every surface.")),
    abi_const("NVE_BIN_VALUE_MAX_DEPTH", 64, false, Some("Maximum permitted container nesting depth for `NveBinValue`.")),
    abi_const("NVE_CHANNEL_FLAG_RETAIN", NVE_CHANNEL_FLAG_RETAIN as u64, false, Some("Flag bit on `channel_publish` / `ChannelMessage` requesting retained channel state.")),
    abi_const("NVE_PRIMARY_SLOT_NONE", crate::table::NVE_PRIMARY_SLOT_NONE as u64, true, Some("`NveNodeRow::primary_slot` sentinel (`0xFF`): plain container with no realizing table.")),
    abi_const("NVE_ROW_INDEX_PAGE_BITS", crate::table::NVE_ROW_INDEX_PAGE_BITS as u64, false, Some("`log2` of the node indices one `NveTableView::row_index_pages` page covers.")),
    abi_const("NVE_ROW_INDEX_PAGE_LEN", crate::table::NVE_ROW_INDEX_PAGE_LEN as u64, false, Some("Entries per `NveTableView::row_index_pages` page.")),
    abi_const("NVE_ROW_INDEX_NO_PAGE", crate::table::NVE_ROW_INDEX_NO_PAGE as u64, true, Some("`NveTableView::row_index_dir` entry of a node-index range without a page.")),
    abi_const("NVE_EVENT_SIZE", NVE_EVENT_SIZE as u64, false, Some("Exact byte stride of one uniform `NveEvent` record (64 bytes).")),
    abi_const("NVE_EVENT_ALIGN", NVE_EVENT_ALIGN as u64, false, Some("Rust cache-line alignment of `NveEvent` slabs (64 bytes).")),
    abi_const("NVE_EVENT_INLINE_KEY_CAP", NVE_EVENT_INLINE_KEY_CAP as u64, false, Some("Maximum inline UTF-8 key bytes stored in `NveEvent::payload[2..6]`.")),
    abi_const("NVE_EVENT_FLAG_MOD_META", EventFlags::MOD_META.raw() as u64, true, Some("Meta / Command modifier flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_MOD_CTRL", EventFlags::MOD_CTRL.raw() as u64, true, Some("Control modifier flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_MOD_SHIFT", EventFlags::MOD_SHIFT.raw() as u64, true, Some("Shift modifier flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_MOD_ALT", EventFlags::MOD_ALT.raw() as u64, true, Some("Alt / Option modifier flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_MOD_REPEAT", EventFlags::MOD_REPEAT.raw() as u64, true, Some("Key auto-repeat flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_BUTTON_SHIFT", EventFlags::BUTTON_SHIFT as u64, false, Some("Bit shift for pointer button index in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_BUTTON_MASK", EventFlags::BUTTON_MASK.raw() as u64, true, Some("Pointer button bitmask (bits 8..=10) in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_CAPTURE", EventFlags::CAPTURE.raw() as u64, true, Some("Pointer capture active flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_COMPOSING", EventFlags::COMPOSING.raw() as u64, true, Some("IME composition active flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_NO_BUBBLE", EventFlags::NO_BUBBLE.raw() as u64, true, Some("Target-only (non-bubbling) dispatch flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_UTF8_BYTES", EventFlags::UTF8_BYTES.raw() as u64, true, Some("Ingress-validated UTF-8 byte-arena slice flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_INLINE_KEY_UTF8", EventFlags::INLINE_KEY_UTF8.raw() as u64, true, Some("Inline UTF-8 key (`<= 16 B` in `payload[2..6]`) flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_CUSTOM_NAMED", EventFlags::CUSTOM_NAMED.raw() as u64, true, Some("Custom named event prefix flag in `NveEvent::flags`.")),
    abi_const("NVE_EVENT_FLAG_KEY_SPLIT_BYTES", EventFlags::KEY_SPLIT_BYTES.raw() as u64, true, Some("Arena-split `[key, code]` UTF-8 flag for keys `> 16 B` in `NveEvent::flags`.")),
    abi_const_u64("NVE_TABLE_INTERACTION", NVE_TABLE_INTERACTION.0, true, Some("Compile-time XXH64 `TableKey` for `nve.interaction`.")),
    abi_const_u64("NVE_TABLE_TEXT_INPUT", NVE_TABLE_TEXT_INPUT.0, true, Some("Compile-time XXH64 `TableKey` for `nve.text_input`.")),
    abi_const_u64("NVE_TABLE_SCROLL", NVE_TABLE_SCROLL.0, true, Some("Compile-time XXH64 `TableKey` for `nve.scroll`.")),
    abi_const_u64("NVE_TABLE_LAYOUT", NVE_TABLE_LAYOUT.0, true, Some("Compile-time XXH64 `TableKey` for `nve.layout`.")),
    abi_const_u64("NVE_TABLE_CUSTOM", NVE_TABLE_CUSTOM.0, true, Some("Compile-time XXH64 `TableKey` for `nve.custom`.")),
];

const CORE_ALIASES: &[AbiAliasDescriptor] = &[
    AbiAliasDescriptor { name: "NveNodeId", target: "u32", doc: Some("Generational node identifier (`index:24 | generation:8`; `0` is null).") },
    AbiAliasDescriptor { name: "NveSurfaceId", target: "u32", doc: Some("Generational surface identifier (`index:24 | generation:8`; `0` is null).") },
    AbiAliasDescriptor { name: "NveFrameId", target: "u64", doc: Some("Monotonic 64-bit frame counter.") },
    AbiAliasDescriptor { name: "NveTableKey", target: "u64", doc: Some("Stable 64-bit table identity (`xxh64(name)`).") },
    AbiAliasDescriptor { name: "NveTimestampUs", target: "u64", doc: Some("Monotonic timestamp in microseconds.") },
    AbiAliasDescriptor { name: "NveDurationUs", target: "u64", doc: Some("Elapsed duration in microseconds.") },
    AbiAliasDescriptor { name: "NveEventId", target: "u32", doc: Some("Table-scoped event discriminant (`0..32`).") },
    AbiAliasDescriptor { name: "NveEventFlags", target: "u32", doc: Some("Bitflags for event modifiers, pointer buttons, capture, IME, and framing.") },
    AbiAliasDescriptor { name: "NveArenaOffset", target: "u32", doc: Some("Byte offset inside an event batch byte arena.") },
    AbiAliasDescriptor { name: "NveByteLen", target: "u32", doc: Some("32-bit byte length across ABI arenas and slices.") },
    AbiAliasDescriptor { name: "NveInterfaceId", target: "u32", doc: Some("Modular C ABI interface identifier passed to `nve_get_interface`.") },
    AbiAliasDescriptor { name: "NveAppHandle", target: "*mut c_void", doc: Some("Opaque handle to an instantiated NVE app/component.") },
    AbiAliasDescriptor { name: "NveHostInvokeFn", target: "*const c_void", doc: Some("Host capability invocation callback function pointer (`NveHostCallbacks::invoke`).") },
    AbiAliasDescriptor { name: "NveHostChannelPublishFn", target: "*const c_void", doc: Some("Host channel-bus publish callback function pointer (`NveHostCallbacks::channel_publish`).") },
];

const CORE_ENUMS: &[AbiEnumDescriptor] = &[
    <NveGestureDisposition as NveEnumSchema>::SCHEMA,
    <NveValueTag as NveEnumSchema>::SCHEMA,
    <InteractionEvent as NveEnumSchema>::SCHEMA,
    <TextInputEvent as NveEnumSchema>::SCHEMA,
    <ScrollEvent as NveEnumSchema>::SCHEMA,
    <LayoutEvent as NveEnumSchema>::SCHEMA,
];

const CORE_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveBufferSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveStrSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValueArray as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValueMap as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Union(<NveValueData as NveUnionSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveValue as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveMapEntry as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<RawEventBytes as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveEvent as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveEventBatch as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NvePropEntry as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NvePropSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveCommandSlice as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSizeConstraints as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveIntrinsicSize as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveHostCallbacks as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<crate::table::NveTableView as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<crate::sink::SurfaceToken as NveStructSchema>::SCHEMA),
];

const CORE_MODULES: &[AbiModuleDescriptor] = &[
    AbiModuleDescriptor {
        name: "Surface",
        trait_name: "NveSurfaceApiSpec",
        interface_id: NVE_IFACE_SURFACE,
        c_name: "NveSurfaceApi",
        doc: Some("Required core surface interface (`NVE_IFACE_SURFACE = 1`)."),
        methods: &[
            abi_method("create", &[abi_param("surface_id", "NveSurfaceId")], "NveAppHandle", false, false, None),
            abi_method("destroy", &[], "()", true, false, None),
            abi_method("render", &[abi_param("frame_id", "NveFrameId")], "NveBufferSlice", true, false, None),
            abi_method("dispatch_events", &[abi_param("batch", "NveEventBatch")], "u32", true, false, None),
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
            abi_method("set_props", &[abi_param("entries", "*const NveMapEntry"), abi_param("count", "u32")], "u32", true, false, None),
            abi_method("drain_emitted_events", &[], "NveEventBatch", true, false, None),
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
        doc: Some("Optional binary host-capability & channel-bus interface (`NVE_IFACE_HOST = 5`)."),
        methods: &[
            abi_method("bind_host", &[abi_param("callbacks", "*const NveHostCallbacks")], "()", true, false, None),
            abi_method("dispatch_host_events", &[abi_param("batch", "NveBufferSlice")], "u32", true, false, None),
        ],
    },
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
    }
}

const fn hash_tag_str(h: &mut crate::Xxh64Hasher, s: &str) {
    h.update_u64(s.len() as u64);
    h.update(s.as_bytes());
}

const fn hash_opt_usize(h: &mut crate::Xxh64Hasher, v: Option<usize>) {
    match v {
        Some(n) => {
            h.update(&[1]);
            h.update_u64(n as u64);
        }
        None => h.update(&[0]),
    }
}

const fn hash_fields(h: &mut crate::Xxh64Hasher, fields: &[AbiFieldDescriptor]) {
    h.update_u64(fields.len() as u64);
    let mut i = 0;
    while i < fields.len() {
        let f = &fields[i];
        hash_tag_str(h, f.name);
        hash_tag_str(h, f.abi_type());
        hash_opt_usize(h, f.expected_offset);
        hash_opt_usize(h, f.expected_offset_64);
        i += 1;
    }
}

const fn hash_sig(
    h: &mut crate::Xxh64Hasher,
    name: &str,
    params: &[AbiParamDescriptor],
    return_type: &str,
    has_self: bool,
    optional: bool,
) {
    hash_tag_str(h, name);
    h.update(&[has_self as u8, optional as u8]);
    h.update_u64(params.len() as u64);
    let mut j = 0;
    while j < params.len() {
        hash_tag_str(h, params[j].name);
        hash_tag_str(h, params[j].ty);
        j += 1;
    }
    hash_tag_str(h, return_type);
}

/// Computes the deterministic, target-independent compile-time XXH64 fingerprint of
/// [`core_abi_descriptor`] directly from the `#[derive(NveAbi)]` metadata (zero hardcoded hex).
pub const fn compute_core_abi_schema_hash() -> u64 {
    let desc = core_abi_descriptor();
    let mut h = crate::Xxh64Hasher::new(0);
    hash_tag_str(&mut h, "nve.core_abi.v2");

    let mut i = 0;
    while i < desc.consts.len() {
        let c = &desc.consts[i];
        hash_tag_str(&mut h, "const");
        hash_tag_str(&mut h, c.name);
        hash_tag_str(&mut h, c.ty);
        h.update_u64(c.value);
        i += 1;
    }

    i = 0;
    while i < desc.aliases.len() {
        let a = &desc.aliases[i];
        hash_tag_str(&mut h, "alias");
        hash_tag_str(&mut h, a.name);
        hash_tag_str(&mut h, a.target);
        i += 1;
    }

    i = 0;
    while i < desc.enums.len() {
        let e = &desc.enums[i];
        hash_tag_str(&mut h, "enum");
        hash_tag_str(&mut h, e.name);
        hash_tag_str(&mut h, e.repr);
        h.update_u64(e.variants.len() as u64);
        let mut j = 0;
        while j < e.variants.len() {
            hash_tag_str(&mut h, e.variants[j].name);
            h.update_u64(e.variants[j].value as u64);
            j += 1;
        }
        i += 1;
    }

    i = 0;
    while i < desc.records.len() {
        match &desc.records[i] {
            AbiRecordDescriptor::Struct(s) => {
                hash_tag_str(&mut h, "struct");
                hash_tag_str(&mut h, s.name);
                hash_opt_usize(&mut h, s.explicit_align);
                hash_opt_usize(&mut h, s.expected_size);
                hash_opt_usize(&mut h, s.expected_size_64);
                hash_fields(&mut h, s.fields);
            }
            AbiRecordDescriptor::Union(u) => {
                hash_tag_str(&mut h, "union");
                hash_tag_str(&mut h, u.name);
                hash_opt_usize(&mut h, u.explicit_align);
                hash_opt_usize(&mut h, u.expected_size);
                hash_opt_usize(&mut h, u.expected_size_64);
                hash_fields(&mut h, u.fields);
            }
        }
        i += 1;
    }

    i = 0;
    while i < desc.modules.len() {
        let m = &desc.modules[i];
        hash_tag_str(&mut h, "module");
        hash_tag_str(&mut h, m.name);
        h.update_u64(m.interface_id.raw() as u64);
        h.update_u64(m.methods.len() as u64);
        let mut j = 0;
        while j < m.methods.len() {
            let fm = &m.methods[j];
            hash_sig(&mut h, fm.name, fm.params, fm.return_type, fm.has_self, fm.optional);
            j += 1;
        }
        i += 1;
    }


    h.digest()
}
