//! Single-source-of-truth `const` ABI metadata descriptors and colocated schema traits
//! (`NveSchema`, `NveStructSchema`, `NveUnionSchema`, `NveEnumSchema`, `NveAliasSchema`,
//! `NveVtableSchema`) populated directly by `#[derive(NveAbi)]` and `abi_{consts,aliases,callbacks}!`
//! on the real compiled `#[repr(C)]` / `#[repr(transparent)]` types in `nve-abi`.
//!
//! All descriptors are `const` / dead-code-eliminated in runtime binaries (`wasm32`, native apps)
//! and consumed directly by `nve-codegen` to emit all checked-in C11/C++20, Swift, Dart, Java,
//! and TypeScript ABI bindings with zero duplicated spec files.

use crate::event::{
    InteractionEvent, LayoutEvent, NveEvent, NveEventBatch, RawEventBytes, ScrollEvent,
    TextInputEvent, EVENT_ABI_CONSTS, EVENT_FLAG_ABI_CONSTS, EVENT_TABLE_ABI_CONSTS,
};
use crate::ffi::{
    NveBufferSlice, NveComponentApi, NveDevApi, NveGestureDisposition, NveIntrinsicSize,
    NveLayoutApi, NveSizeConstraints, NveSurfaceApi, FFI_ABI_ALIASES, FFI_ABI_CALLBACKS,
    FFI_ABI_CONSTS,
};
use crate::host::{
    HostRequestId, HostServiceId, NveHostApi, NveHostCallbacks, NveHostStatus, HOST_ABI_CALLBACKS,
    HOST_ABI_CONSTS,
};
use crate::id::{
    ArenaOffset, ByteLen, DurationUs, EventFlags, EventId, FrameId, InterfaceId, NodeId, SurfaceId,
    TimestampUs, ID_ABI_CONSTS,
};
use crate::table::{NveTableView, TABLE_ABI_ALIASES, TABLE_ABI_CONSTS};
use crate::value::{
    NveCommandSlice, NveMapEntry, NvePropEntry, NvePropSlice, NveStrSlice, NveValue,
    NveValueArray, NveValueData, NveValueMap, NveValueTag, VALUE_ABI_CONSTS,
};

pub use crate::runtime::runtime_abi_descriptor;
pub use nve_abi_macros::NveAbi;

/// Compile-time metadata for one named field of a `#[repr(C)]` struct, union, or vtable header.
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

/// Compile-time constant descriptor (e.g. wire magic, ABI version, flag bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiConstDescriptor {
    pub name: &'static str,
    pub ty: &'static str,
    pub value: u64,
    pub hex: bool,
    pub doc: Option<&'static str>,
}

impl AbiConstDescriptor {
    pub(crate) const EMPTY: Self = Self {
        name: "",
        ty: "",
        value: 0,
        hex: false,
        doc: None,
    };
}

/// Compile-time opaque handle or `#[repr(transparent)]` value-alias typedef descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiAliasDescriptor {
    /// C typedef name (`"NveNodeId"`, `"NveAppHandle"`).
    pub name: &'static str,
    /// Rust type name when it differs from `name` (`Some("NodeId")`, `Some("TableKey")`).
    pub rust_name: Option<&'static str>,
    /// Underlying ABI type (`"u32"`, `"u64"`, `"*mut c_void"`).
    pub target: &'static str,
    pub doc: Option<&'static str>,
}

impl AbiAliasDescriptor {
    pub(crate) const EMPTY: Self = Self {
        name: "",
        rust_name: None,
        target: "",
        doc: None,
    };
}

/// Named parameter of a vtable method or callback typedef.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiParamDescriptor {
    pub name: &'static str,
    pub ty: &'static str,
}

/// Compile-time `extern "C" fn` callback typedef descriptor (`NveHostInvokeFn`, `NveRuntimeWakeFn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiCallbackDescriptor {
    pub name: &'static str,
    pub params: &'static [AbiParamDescriptor],
    pub return_type: &'static str,
    pub doc: Option<&'static str>,
}

impl AbiCallbackDescriptor {
    pub(crate) const EMPTY: Self = Self {
        name: "",
        params: &[],
        return_type: "",
        doc: None,
    };
}

/// One function-pointer slot in a C ABI vtable (`NveSurfaceApi`, `NveRuntimeApi`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiMethodDescriptor {
    pub name: &'static str,
    pub params: &'static [AbiParamDescriptor],
    pub return_type: &'static str,
    pub doc: Option<&'static str>,
    /// `true` iff the Rust slot type is `Option<extern "C" fn(...)>`.
    pub optional: bool,
}

/// C ABI vtable contract descriptor derived directly from a `#[repr(C)]` struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiVtableDescriptor {
    /// C and Rust struct name (`"NveSurfaceApi"`, `"NveRuntimeApi"`).
    pub name: &'static str,
    /// Modular interface name (`Some("Surface")`) for `nve_get_interface` modules.
    pub module_name: Option<&'static str>,
    /// Interface ID (`Some(NVE_IFACE_SURFACE)`) for `nve_get_interface` modules.
    pub interface_id: Option<InterfaceId>,
    pub doc: Option<&'static str>,
    pub size: usize,
    pub align: usize,
    /// Byte offset of the first function-pointer slot (`0` for pure vtables, `16` for `NveRuntimeApi`).
    pub slots_offset: usize,
    /// Leading non-function-pointer data fields before the slot table (empty for modular interfaces).
    pub header_fields: &'static [AbiFieldDescriptor],
    /// Function-pointer slots in vtable declaration order.
    pub methods: &'static [AbiMethodDescriptor],
}

/// Unified descriptor tag exposed by [`NveSchema`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiItemDescriptor {
    Struct(AbiStructDescriptor),
    Union(AbiUnionDescriptor),
    Enum(AbiEnumDescriptor),
    Alias(AbiAliasDescriptor),
    Vtable(AbiVtableDescriptor),
}

/// Ordered struct or union entry inside [`AbiFileDescriptor::records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiRecordDescriptor {
    Struct(AbiStructDescriptor),
    Union(AbiUnionDescriptor),
}

/// Complete colocated ABI file descriptor consumed directly by `nve-codegen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbiFileDescriptor {
    pub namespace: Option<&'static str>,
    pub consts: &'static [AbiConstDescriptor],
    pub aliases: &'static [AbiAliasDescriptor],
    pub callbacks: &'static [AbiCallbackDescriptor],
    pub enums: &'static [AbiEnumDescriptor],
    pub records: &'static [AbiRecordDescriptor],
    pub vtables: &'static [AbiVtableDescriptor],
}

/// Unified schema trait implemented by `#[derive(NveAbi)]` on structs, unions, enums, aliases, and vtables.
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

/// Schema trait implemented by `#[derive(NveAbi)]` on `#[repr(transparent)]` handle newtypes.
pub trait NveAliasSchema: NveSchema {
    const SCHEMA: AbiAliasDescriptor;
}

/// Schema trait implemented by `#[derive(NveAbi)]` on `#[repr(C)]` C ABI vtable structs.
pub trait NveVtableSchema: NveSchema {
    const SCHEMA: AbiVtableDescriptor;
}

/// Zero-`unsafe` compile-time concatenation of `&[T]` descriptor slices into one flat `const` array.
macro_rules! concat_abi_slices {
    ($name:ident : [$ty:ty] = [ $($slice:expr),+ $(,)? ]) => {
        const $name: &[$ty] = &{
            const TOTAL: usize = 0 $(+ $slice.len())+;
            let mut out = [<$ty>::EMPTY; TOTAL];
            let mut cursor = 0usize;
            $({
                let src: &[$ty] = $slice;
                let mut i = 0usize;
                while i < src.len() {
                    out[cursor] = src[i];
                    cursor += 1;
                    i += 1;
                }
            })+
            out
        };
    };
}
pub(crate) use concat_abi_slices;

concat_abi_slices!(CORE_CONSTS: [AbiConstDescriptor] = [
    FFI_ABI_CONSTS,
    ID_ABI_CONSTS,
    VALUE_ABI_CONSTS,
    HOST_ABI_CONSTS,
    TABLE_ABI_CONSTS,
    EVENT_ABI_CONSTS,
    EVENT_FLAG_ABI_CONSTS,
    EVENT_TABLE_ABI_CONSTS,
]);

const CORE_HANDLE_ALIASES_PRE: &[AbiAliasDescriptor] = &[
    <NodeId as NveAliasSchema>::SCHEMA,
    <SurfaceId as NveAliasSchema>::SCHEMA,
    <FrameId as NveAliasSchema>::SCHEMA,
];

const CORE_HANDLE_ALIASES_POST: &[AbiAliasDescriptor] = &[
    <TimestampUs as NveAliasSchema>::SCHEMA,
    <DurationUs as NveAliasSchema>::SCHEMA,
    <EventId as NveAliasSchema>::SCHEMA,
    <EventFlags as NveAliasSchema>::SCHEMA,
    <ArenaOffset as NveAliasSchema>::SCHEMA,
    <ByteLen as NveAliasSchema>::SCHEMA,
    <InterfaceId as NveAliasSchema>::SCHEMA,
    <HostServiceId as NveAliasSchema>::SCHEMA,
    <HostRequestId as NveAliasSchema>::SCHEMA,
];

concat_abi_slices!(CORE_ALIASES: [AbiAliasDescriptor] = [
    CORE_HANDLE_ALIASES_PRE,
    TABLE_ABI_ALIASES,
    CORE_HANDLE_ALIASES_POST,
    FFI_ABI_ALIASES,
]);

concat_abi_slices!(CORE_CALLBACKS: [AbiCallbackDescriptor] = [
    HOST_ABI_CALLBACKS,
    FFI_ABI_CALLBACKS,
]);

const CORE_ENUMS: &[AbiEnumDescriptor] = &[
    <NveGestureDisposition as NveEnumSchema>::SCHEMA,
    <NveValueTag as NveEnumSchema>::SCHEMA,
    <InteractionEvent as NveEnumSchema>::SCHEMA,
    <TextInputEvent as NveEnumSchema>::SCHEMA,
    <ScrollEvent as NveEnumSchema>::SCHEMA,
    <LayoutEvent as NveEnumSchema>::SCHEMA,
    <NveHostStatus as NveEnumSchema>::SCHEMA,
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
    AbiRecordDescriptor::Struct(<NveTableView as NveStructSchema>::SCHEMA),
];

const CORE_VTABLES: &[AbiVtableDescriptor] = &[
    <NveSurfaceApi as NveVtableSchema>::SCHEMA,
    <NveLayoutApi as NveVtableSchema>::SCHEMA,
    <NveComponentApi as NveVtableSchema>::SCHEMA,
    <NveDevApi as NveVtableSchema>::SCHEMA,
    <NveHostApi as NveVtableSchema>::SCHEMA,
];

/// Returns the single-source-of-truth Core C ABI descriptor derived directly from the real
/// compiled `#[repr(C)]` / `#[repr(transparent)]` types in `packages/core/abi`.
pub const fn core_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveCoreAbi"),
        consts: CORE_CONSTS,
        aliases: CORE_ALIASES,
        callbacks: CORE_CALLBACKS,
        enums: CORE_ENUMS,
        records: CORE_RECORDS,
        vtables: CORE_VTABLES,
    }
}

pub(crate) const fn hash_tag_str(h: &mut crate::Xxh64Hasher, s: &str) {
    h.update_u64(s.len() as u64);
    h.update(s.as_bytes());
}

const fn hash_opt_str(h: &mut crate::Xxh64Hasher, v: Option<&str>) {
    match v {
        Some(s) => {
            h.update(&[1]);
            hash_tag_str(h, s);
        }
        None => h.update(&[0]),
    }
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

const fn hash_params_and_ret(
    h: &mut crate::Xxh64Hasher,
    params: &[AbiParamDescriptor],
    return_type: &str,
) {
    h.update_u64(params.len() as u64);
    let mut j = 0;
    while j < params.len() {
        hash_tag_str(h, params[j].name);
        hash_tag_str(h, params[j].ty);
        j += 1;
    }
    hash_tag_str(h, return_type);
}

/// Target-independent compile-time XXH64 fingerprint of an [`AbiFileDescriptor`].
///
/// Never hashes target-dependent `size`, `align`, `field.offset`, or `slots_offset`, so the
/// resulting digest is identical across LP64, LLP64, ILP32, i386 System V, and `wasm32`.
pub const fn hash_abi_descriptor_into(h: &mut crate::Xxh64Hasher, desc: &AbiFileDescriptor) {
    hash_opt_str(h, desc.namespace);
    h.update_u64(desc.consts.len() as u64);
    let mut i = 0;
    while i < desc.consts.len() {
        let c = &desc.consts[i];
        hash_tag_str(h, "const");
        hash_tag_str(h, c.name);
        hash_tag_str(h, c.ty);
        h.update_u64(c.value);
        i += 1;
    }

    h.update_u64(desc.aliases.len() as u64);
    i = 0;
    while i < desc.aliases.len() {
        let a = &desc.aliases[i];
        hash_tag_str(h, "alias");
        hash_tag_str(h, a.name);
        hash_tag_str(h, a.target);
        i += 1;
    }

    h.update_u64(desc.callbacks.len() as u64);
    i = 0;
    while i < desc.callbacks.len() {
        let cb = &desc.callbacks[i];
        hash_tag_str(h, "callback");
        hash_tag_str(h, cb.name);
        hash_params_and_ret(h, cb.params, cb.return_type);
        i += 1;
    }

    h.update_u64(desc.enums.len() as u64);
    i = 0;
    while i < desc.enums.len() {
        let e = &desc.enums[i];
        hash_tag_str(h, "enum");
        hash_tag_str(h, e.name);
        hash_tag_str(h, e.repr);
        h.update_u64(e.variants.len() as u64);
        let mut j = 0;
        while j < e.variants.len() {
            hash_tag_str(h, e.variants[j].name);
            h.update_u64(e.variants[j].value as u64);
            j += 1;
        }
        i += 1;
    }

    h.update_u64(desc.records.len() as u64);
    i = 0;
    while i < desc.records.len() {
        match &desc.records[i] {
            AbiRecordDescriptor::Struct(s) => {
                hash_tag_str(h, "struct");
                hash_tag_str(h, s.name);
                hash_opt_usize(h, s.explicit_align);
                hash_opt_usize(h, s.expected_size);
                hash_opt_usize(h, s.expected_size_64);
                hash_fields(h, s.fields);
            }
            AbiRecordDescriptor::Union(u) => {
                hash_tag_str(h, "union");
                hash_tag_str(h, u.name);
                hash_opt_usize(h, u.explicit_align);
                hash_opt_usize(h, u.expected_size);
                hash_opt_usize(h, u.expected_size_64);
                hash_fields(h, u.fields);
            }
        }
        i += 1;
    }

    h.update_u64(desc.vtables.len() as u64);
    i = 0;
    while i < desc.vtables.len() {
        let v = &desc.vtables[i];
        hash_tag_str(h, "vtable");
        hash_tag_str(h, v.name);
        hash_opt_str(h, v.module_name);
        match v.interface_id {
            Some(id) => {
                h.update(&[1]);
                h.update_u64(id.raw() as u64);
            }
            None => h.update(&[0]),
        }
        hash_fields(h, v.header_fields);
        h.update_u64(v.methods.len() as u64);
        let mut j = 0;
        while j < v.methods.len() {
            let m = &v.methods[j];
            hash_tag_str(h, m.name);
            h.update(&[m.optional as u8]);
            hash_params_and_ret(h, m.params, m.return_type);
            j += 1;
        }
        i += 1;
    }
}

/// Computes the deterministic, target-independent compile-time XXH64 fingerprint of
/// [`core_abi_descriptor`] directly from the `#[derive(NveAbi)]` metadata (zero hardcoded hex).
pub const fn compute_core_abi_schema_hash() -> u64 {
    let mut h = crate::Xxh64Hasher::new(0);
    hash_tag_str(&mut h, "nve.core_abi.v3");
    hash_abi_descriptor_into(&mut h, &core_abi_descriptor());
    h.digest()
}
