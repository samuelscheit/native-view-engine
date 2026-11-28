//! Single-source-of-truth `const` ABI metadata descriptors and colocated schema traits
//! (`NveSchema`, `NveStructSchema`, `NveUnionSchema`, `NveEnumSchema`) populated directly by
//! `#[derive(NveAbi)]` on the real compiled `#[repr(C)]` types in `nve-abi`.
//!
//! All descriptors are `const` / dead-code-eliminated in runtime binaries (`wasm32`, native apps)
//! and consumed directly by `nve-codegen` to emit all checked-in C11/C++20, Swift, Dart, Java,
//! and TypeScript ABI bindings with zero duplicated spec files.

use crate::ffi::{
    NativeEventKind, NveBufferSlice, NveEventPayloadTag, NveGestureDisposition,
    NveIntrinsicSize, NveSizeConstraints, NVE_IFACE_COMPONENT, NVE_IFACE_DEV,
    NVE_IFACE_HOST, NVE_IFACE_LAYOUT, NVE_IFACE_SURFACE,
};
use crate::host::{NveHostCallbacks, NVE_CHANNEL_FLAG_RETAIN};
use crate::value::{
    NveCommandSlice, NveDirectEvent, NveEventBatch, NveMapEntry, NvePropEntry, NvePropSlice,
    NveStrSlice, NveValue, NveValueArray, NveValueData, NveValueMap, NveValueTag,
    NVE_EVENT_FLAG_BUTTON_MASK, NVE_EVENT_FLAG_CAPTURE, NVE_EVENT_FLAG_COMPOSING, NVE_KEY_MOD_ALT,
    NVE_KEY_MOD_CTRL, NVE_KEY_MOD_META, NVE_KEY_MOD_REPEAT, NVE_KEY_MOD_SHIFT,
};
use crate::ROOT_NODE_ID;

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
    pub interface_id: u32,
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
    abi_const("NVE_ROOT_NODE_ID", ROOT_NODE_ID as u64, false, Some("Reserved root node ID for every surface.")),
    abi_const("NVE_BIN_VALUE_MAX_DEPTH", 64, false, Some("Maximum permitted container nesting depth for `NveBinValue`.")),
    abi_const("NVE_CHANNEL_FLAG_RETAIN", NVE_CHANNEL_FLAG_RETAIN as u64, false, Some("Flag bit on `channel_publish` / `ChannelMessage` requesting retained channel state.")),
    abi_const("NVE_KEY_MOD_META", NVE_KEY_MOD_META as u64, true, Some("Meta / Command modifier flag in `NveDirectEvent::modifiers`.")),
    abi_const("NVE_KEY_MOD_CTRL", NVE_KEY_MOD_CTRL as u64, true, Some("Control modifier flag in `NveDirectEvent::modifiers`.")),
    abi_const("NVE_KEY_MOD_SHIFT", NVE_KEY_MOD_SHIFT as u64, true, Some("Shift modifier flag in `NveDirectEvent::modifiers`.")),
    abi_const("NVE_KEY_MOD_ALT", NVE_KEY_MOD_ALT as u64, true, Some("Alt / Option modifier flag in `NveDirectEvent::modifiers`.")),
    abi_const("NVE_KEY_MOD_REPEAT", NVE_KEY_MOD_REPEAT as u64, true, Some("Key auto-repeat flag in `NveDirectEvent::modifiers`.")),
    abi_const("NVE_EVENT_FLAG_BUTTON_MASK", NVE_EVENT_FLAG_BUTTON_MASK as u64, true, Some("Pointer button bitmask (bits 0..=2) in `NveDirectEvent::event_flags`.")),
    abi_const("NVE_EVENT_FLAG_CAPTURE", NVE_EVENT_FLAG_CAPTURE as u64, true, Some("Pointer capture active flag in `NveDirectEvent::event_flags`.")),
    abi_const("NVE_EVENT_FLAG_COMPOSING", NVE_EVENT_FLAG_COMPOSING as u64, true, Some("IME composition active flag in `NveDirectEvent::event_flags`.")),
    abi_const("NVE_PRIMARY_SLOT_NONE", crate::table::NVE_PRIMARY_SLOT_NONE as u64, true, Some("`NveNodeRow::primary_slot` sentinel (`0xFF`): plain container with no realizing table.")),
    abi_const("NVE_ROW_INDEX_PAGE_BITS", crate::table::NVE_ROW_INDEX_PAGE_BITS as u64, false, Some("`log2` of the node indices one `NveTableView::row_index_pages` page covers.")),
    abi_const("NVE_ROW_INDEX_PAGE_LEN", crate::table::NVE_ROW_INDEX_PAGE_LEN as u64, false, Some("Entries per `NveTableView::row_index_pages` page.")),
    abi_const("NVE_ROW_INDEX_NO_PAGE", crate::table::NVE_ROW_INDEX_NO_PAGE as u64, true, Some("`NveTableView::row_index_dir` entry of a node-index range without a page.")),
];

const CORE_ALIASES: &[AbiAliasDescriptor] = &[
    AbiAliasDescriptor { name: "NveAppHandle", target: "*mut c_void", doc: Some("Opaque handle to an instantiated NVE app/component.") },
    AbiAliasDescriptor { name: "NveHostInvokeFn", target: "*const c_void", doc: Some("Host capability invocation callback function pointer (`NveHostCallbacks::invoke`).") },
    AbiAliasDescriptor { name: "NveHostChannelPublishFn", target: "*const c_void", doc: Some("Host channel-bus publish callback function pointer (`NveHostCallbacks::channel_publish`).") },
];

const CORE_ENUMS: &[AbiEnumDescriptor] = &[
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
    AbiRecordDescriptor::Struct(<NveSizeConstraints as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveIntrinsicSize as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveHostCallbacks as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<crate::table::NveTableView as NveStructSchema>::SCHEMA),
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
        h.update_u64(m.interface_id as u64);
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
