//! Integration tests verifying `#[derive(NveAbi)]` on real compiled types (both built-in
//! `nve-abi` descriptors and downstream structs, unions, and signed/unsigned enums).

use core::mem::{align_of, offset_of, size_of};

use nve_abi::host::{NveHostEventKind, NveHostStatus};
use nve_abi::schema::{
    core_abi_descriptor, scene_abi_descriptor, AbiItemDescriptor, AbiRecordDescriptor,
    NveEnumSchema, NveSchema, NveStructSchema, NveUnionSchema,
};
use nve_abi::{
    NveAbi, NveBufferSlice, NveRenderNode, NveSceneView, NveValueData, Opcode,
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
fn core_and_scene_descriptors_reflect_real_rustc_layouts() {
    let core = core_abi_descriptor();
    assert_eq!(core.namespace, Some("NveCoreAbi"));
    assert_eq!(core.consts.len(), 7);
    assert_eq!(core.aliases.len(), 4);
    assert_eq!(core.enums.len(), 6);
    assert_eq!(core.records.len(), 17);
    assert_eq!(core.modules.len(), 5);
    assert_eq!(core.c_functions.len(), 5);

    // Union NveValueData stays in exact declaration slot 4 between NveValueMap and NveValue
    assert!(matches!(
        core.records[4],
        AbiRecordDescriptor::Union(u) if u.name == "NveValueData"
    ));

    let scene = scene_abi_descriptor();
    assert_eq!(scene.namespace, Some("NveSceneAbi"));
    assert_eq!(scene.consts.len(), 14);
    assert_eq!(scene.records.len(), 2);

    let render = <NveRenderNode as NveStructSchema>::SCHEMA;
    assert_eq!(render.size, size_of::<NveRenderNode>());
    assert_eq!(render.align, align_of::<NveRenderNode>());
    assert_eq!(render.explicit_align, Some(64));
    assert_eq!(render.expected_size, Some(64));
    assert_eq!(render.fields.len(), 20);
    assert_eq!(render.fields[19].name, "z_index");
    assert_eq!(render.fields[19].offset, offset_of!(NveRenderNode, z_index));

    let view = <NveSceneView as NveStructSchema>::SCHEMA;
    assert_eq!(view.size, size_of::<NveSceneView>());
    assert_eq!(view.fields[8].name, "nodes");
    assert_eq!(view.fields[8].offset, offset_of!(NveSceneView, nodes));
    assert_eq!(view.fields[8].expected_offset, None);

    let union_desc = <NveValueData as NveUnionSchema>::SCHEMA;
    assert_eq!(union_desc.size, size_of::<NveValueData>());
    assert_eq!(union_desc.fields.len(), 7);
    assert!(union_desc.fields.iter().all(|f| f.offset == 0));

    let status = <NveHostStatus as NveEnumSchema>::SCHEMA;
    assert_eq!(status.repr, "i32");
    assert_eq!(status.variants[0].value, 0);
    assert_eq!(status.variants[2].value, -1);
    assert_eq!(status.variants[9].value, -8);

    let host_ev = <NveHostEventKind as NveEnumSchema>::SCHEMA;
    assert_eq!(host_ev.variants.len(), 3);
    assert_eq!(host_ev.variants[2].value, 3);

    assert_eq!(
        <NveBufferSlice as NveSchema>::ITEM,
        AbiItemDescriptor::Struct(<NveBufferSlice as NveStructSchema>::SCHEMA)
    );
    assert_eq!(
        <NveValueData as NveSchema>::ITEM,
        AbiItemDescriptor::Union(<NveValueData as NveUnionSchema>::SCHEMA)
    );
    assert_eq!(
        <Opcode as NveSchema>::ITEM,
        AbiItemDescriptor::Enum(<Opcode as NveEnumSchema>::SCHEMA)
    );
}
