//! Integration tests verifying `#[derive(NveAbi)]` on real compiled types (both built-in
//! `nve-abi` descriptors and downstream structs, unions, and signed/unsigned enums).

use core::mem::{offset_of, size_of};

use nve_abi::host::NveHostStatus;
use nve_abi::schema::{
    core_abi_descriptor, runtime_abi_descriptor, AbiItemDescriptor, AbiRecordDescriptor,
    NveAliasSchema, NveEnumSchema, NveSchema, NveStructSchema, NveUnionSchema, NveVtableSchema,
};
use nve_abi::{
    InteractionEvent, NodeId, NveAbi, NveBufferSlice, NveEvent, NveRuntimeApi, NveSceneView,
    NveSurfaceApi, NveValueData,
};

/// Custom aligned POD struct exercising all `#[nve_abi(...)]` struct & field options.
/// Line 2 with "quotes" and `backticks`.
#[repr(C, align(16))]
#[derive(Clone, Copy, NveAbi)]
#[nve_abi(name = "CustomAlignedPod", size = 32, align = 16)]
struct LocalPod {
    #[nve_abi(offset = 0, doc = "Explicit doc override")]
    pub tag: u32,
    #[nve_abi(offset = 4)]
    pub pad: [u8; 4],
    #[nve_abi(offset = 8, c_type = "const void*")]
    pub raw_ptr: *const core::ffi::c_void,
    #[nve_abi(offset = 16)]
    pub payload: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy, NveAbi)]
#[nve_abi(size = 8)]
union LocalUnion {
    pub as_u64: u64,
    pub as_f64: f64,
    pub as_bytes: [u8; 8],
}

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, NveAbi)]
#[nve_abi(name = "LocalSignedCode", c_prefix = "LOCAL_ERR_", dart_fallback = "Unknown")]
enum LocalSigned {
    Ok = 0,
    Unknown = -1,
    CustomHigh = 0x100,
}

#[test]
fn derive_nve_abi_handles_downstream_struct_union_and_signed_enum() {
    let s = <LocalPod as NveStructSchema>::SCHEMA;
    assert_eq!(s.name, "CustomAlignedPod");
    assert_eq!(s.size, size_of::<LocalPod>());
    assert_eq!(s.align, 16);
    assert_eq!(s.explicit_align, Some(16));
    assert_eq!(s.expected_size, Some(32));
    assert!(s.doc.unwrap().contains("\"quotes\""));
    assert_eq!(s.fields.len(), 4);
    assert_eq!(s.fields[0].offset, offset_of!(LocalPod, tag));
    assert_eq!(s.fields[0].doc, Some("Explicit doc override"));
    assert_eq!(s.fields[1].ty, "[u8; 4]");
    assert_eq!(s.fields[2].ty, "*const c_void");
    assert_eq!(s.fields[2].c_type, Some("const void*"));
    assert_eq!(s.fields[2].abi_type(), "const void*");
    assert_eq!(s.fields[3].offset, offset_of!(LocalPod, payload));

    let u = <LocalUnion as NveUnionSchema>::SCHEMA;
    assert_eq!(u.name, "LocalUnion");
    assert_eq!(u.size, 8);
    assert_eq!(u.fields.len(), 3);
    assert!(u.fields.iter().all(|f| f.offset == 0));

    let e = <LocalSigned as NveEnumSchema>::SCHEMA;
    assert_eq!(e.name, "LocalSignedCode");
    assert_eq!(e.repr, "i32");
    assert_eq!(e.c_prefix, Some("LOCAL_ERR_"));
    assert_eq!(e.dart_fallback, Some("Unknown"));
    assert_eq!(e.variants.len(), 3);
    assert_eq!(e.variants[0].value, 0);
    assert_eq!(e.variants[1].value, -1);
    assert_eq!(e.variants[2].value, 256);

    assert_eq!(<LocalPod as NveSchema>::ITEM, AbiItemDescriptor::Struct(s));
    assert_eq!(<LocalUnion as NveSchema>::ITEM, AbiItemDescriptor::Union(u));
    assert_eq!(<LocalSigned as NveSchema>::ITEM, AbiItemDescriptor::Enum(e));
}

#[test]
fn core_and_scene_view_descriptors_reflect_real_rustc_layouts() {
    let core = core_abi_descriptor();
    assert_eq!(core.namespace, Some("NveCoreAbi"));
    assert_eq!(core.consts.len(), 31);
    assert_eq!(core.aliases.len(), 14);
    assert_eq!(core.aliases[0].name, "NveNodeId");
    assert_eq!(core.aliases[0].rust_name, Some("NodeId"));
    assert_eq!(core.aliases[1].name, "NveSurfaceId");
    assert_eq!(core.aliases[3].name, "NveTableKey");
    assert_eq!(core.aliases[5].name, "NveDurationUs");
    assert_eq!(core.aliases[10].name, "NveInterfaceId");
    assert_eq!(core.aliases[11].name, "NveHostServiceId");
    assert_eq!(core.aliases[12].name, "NveHostRequestId");
    assert_eq!(core.callbacks.len(), 3);
    assert_eq!(core.enums.len(), 7);
    assert_eq!(core.records.len(), 17);
    assert_eq!(core.vtables.len(), 5);

    let rt = runtime_abi_descriptor();
    assert_eq!(rt.namespace, Some("NveRuntimeAbi"));
    assert_eq!(rt.consts.len(), 9);
    assert_eq!(rt.aliases.len(), 1);
    assert_eq!(rt.aliases[0].name, "NveRuntimeHandle");
    assert_eq!(rt.callbacks.len(), 4);
    assert_eq!(rt.enums.len(), 1);
    assert_eq!(rt.records.len(), 8);
    assert_eq!(rt.vtables.len(), 1);
    assert_eq!(rt.vtables[0].name, "NveRuntimeApi");
    assert_eq!(rt.vtables[0].slots_offset, 16);
    assert_eq!(rt.vtables[0].header_fields.len(), 3);
    assert_eq!(rt.vtables[0].methods.len(), 37);

    assert_eq!(
        <NodeId as NveSchema>::ITEM,
        AbiItemDescriptor::Alias(<NodeId as NveAliasSchema>::SCHEMA)
    );
    assert_eq!(
        <NveSurfaceApi as NveSchema>::ITEM,
        AbiItemDescriptor::Vtable(<NveSurfaceApi as NveVtableSchema>::SCHEMA)
    );
    assert_eq!(
        <NveRuntimeApi as NveSchema>::ITEM,
        AbiItemDescriptor::Vtable(<NveRuntimeApi as NveVtableSchema>::SCHEMA)
    );

    let ev_schema = <NveEvent as NveStructSchema>::SCHEMA;
    assert_eq!(ev_schema.size, 64);
    assert_eq!(ev_schema.align, 64);
    assert_eq!(ev_schema.explicit_align, Some(64));
    assert_eq!(ev_schema.fields.len(), 8);

    // Union NveValueData stays in exact declaration slot 4 between NveValueMap and NveValue
    assert!(matches!(
        core.records[4],
        AbiRecordDescriptor::Union(u) if u.name == "NveValueData"
    ));

    let table = <nve_abi::table::NveTableView as NveStructSchema>::SCHEMA;
    assert!(core.records.iter().any(|r| matches!(r, AbiRecordDescriptor::Struct(s) if s.name == "NveTableView")));
    assert_eq!(table.size, size_of::<nve_abi::table::NveTableView>());

    let view = <NveSceneView as NveStructSchema>::SCHEMA;
    assert_eq!(view.size, size_of::<NveSceneView>());
    assert_eq!(view.fields[12].name, "rows");
    assert_eq!(view.fields[12].offset, offset_of!(NveSceneView, rows));
    assert_eq!(view.fields[12].expected_offset, None);

    let union_desc = <NveValueData as NveUnionSchema>::SCHEMA;
    assert_eq!(union_desc.size, size_of::<NveValueData>());
    assert_eq!(union_desc.fields.len(), 7);
    assert!(union_desc.fields.iter().all(|f| f.offset == 0));

    let status = <NveHostStatus as NveEnumSchema>::SCHEMA;
    assert_eq!(status.repr, "i32");
    assert_eq!(status.variants[0].value, 0);
    assert_eq!(status.variants[2].value, -1);
    assert_eq!(status.variants[9].value, -8);

    assert_eq!(
        <NveBufferSlice as NveSchema>::ITEM,
        AbiItemDescriptor::Struct(<NveBufferSlice as NveStructSchema>::SCHEMA)
    );
    assert_eq!(
        <NveValueData as NveSchema>::ITEM,
        AbiItemDescriptor::Union(<NveValueData as NveUnionSchema>::SCHEMA)
    );
    assert_eq!(
        <InteractionEvent as NveSchema>::ITEM,
        AbiItemDescriptor::Enum(<InteractionEvent as NveEnumSchema>::SCHEMA)
    );
}
