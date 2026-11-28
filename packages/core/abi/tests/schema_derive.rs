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
    NveSurfaceApi, NveValueData, ReprEnum,
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
    assert_eq!(core.records.len(), 18);
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

    let tok_schema = <nve_abi::SurfaceToken as NveStructSchema>::SCHEMA;
    assert_eq!(tok_schema.name, "NveSurfaceToken");
    assert_eq!(tok_schema.size, 8);
    assert_eq!(tok_schema.fields.len(), 2);
    assert_eq!(tok_schema.fields[0].abi_type(), "NveSurfaceId");
    assert_eq!(tok_schema.fields[1].offset, 4);
    assert!(core
        .records
        .iter()
        .any(|r| matches!(r, AbiRecordDescriptor::Struct(s) if s.name == "NveSurfaceToken")));

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

#[repr(i8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ReprEnum)]
enum MinI8 {
    Min = -128,
    Zero = 0,
    Max = 127,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ReprEnum)]
enum MinI32 {
    Min = -2147483648,
    NegOne = -1,
    Max = 2147483647,
}

#[repr(i64)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ReprEnum)]
#[repr_enum(fallback = r#type)]
enum MinI64Raw {
    Min = -9223372036854775808,
    r#type = 0,
    r#match = 9223372036854775807,
}

#[repr(C)]
#[derive(Clone, Copy, NveAbi)]
#[nve_abi(vtable)]
struct RawIdentVtable {
    pub r#match: Option<unsafe extern "C" fn(r#type: u32, r#fn: *const u8) -> i32>,
}

#[test]
fn repr_enum_signed_min_boundaries_and_raw_idents_round_trip() {
    assert_eq!(MinI8::COUNT, 3);
    assert_eq!(MinI8::ALL, [MinI8::Min, MinI8::Zero, MinI8::Max]);
    assert_eq!(MinI8::Min.as_i8(), i8::MIN);
    assert_eq!(MinI8::try_from_i8(i8::MIN), Some(MinI8::Min));
    assert_eq!(MinI8::try_from_i8(i8::MAX), Some(MinI8::Max));
    assert_eq!(MinI8::try_from_i8(1), None);

    assert_eq!(MinI32::Min.as_i32(), i32::MIN);
    assert_eq!(MinI32::try_from_i32(i32::MIN), Some(MinI32::Min));
    assert_eq!(MinI32::try_from_i32(-1), Some(MinI32::NegOne));
    assert_eq!(MinI32::try_from_i32(i32::MAX), Some(MinI32::Max));
    assert_eq!(MinI32::try_from_i32(42), None);

    assert_eq!(MinI64Raw::COUNT, 3);
    assert_eq!(
        MinI64Raw::ALL,
        [MinI64Raw::Min, MinI64Raw::r#type, MinI64Raw::r#match]
    );
    assert_eq!(MinI64Raw::Min.as_i64(), i64::MIN);
    assert_eq!(MinI64Raw::try_from_i64(i64::MIN), Some(MinI64Raw::Min));
    assert_eq!(MinI64Raw::try_from_i64(0), Some(MinI64Raw::r#type));
    assert_eq!(MinI64Raw::try_from_i64(i64::MAX), Some(MinI64Raw::r#match));
    assert_eq!(MinI64Raw::try_from_i64(99), None);
    assert_eq!(MinI64Raw::from_i64(99), MinI64Raw::r#type);

    let vt = <RawIdentVtable as NveVtableSchema>::SCHEMA;
    assert_eq!(vt.methods.len(), 1);
    assert_eq!(vt.methods[0].name, "match");
    assert!(vt.methods[0].optional);
    assert_eq!(vt.methods[0].params.len(), 2);
    assert_eq!(vt.methods[0].params[0].name, "type");
    assert_eq!(vt.methods[0].params[0].ty, "u32");
    assert_eq!(vt.methods[0].params[1].name, "fn");
    assert_eq!(vt.methods[0].params[1].ty, "*const u8");
    assert_eq!(vt.methods[0].return_type, "i32");
}

#[test]
fn schema_hash_is_sensitive_to_every_abi_category_mutation() {
    use nve_abi::schema::{
        hash_abi_descriptor_into, AbiAliasDescriptor, AbiCallbackDescriptor, AbiConstDescriptor,
        AbiEnumDescriptor, AbiEnumVariantDescriptor, AbiFieldDescriptor, AbiFileDescriptor,
        AbiMethodDescriptor, AbiParamDescriptor, AbiStructDescriptor, AbiVtableDescriptor,
    };
    use nve_abi::Xxh64Hasher;

    let hash_desc = |d: &AbiFileDescriptor| -> u64 {
        let mut h = Xxh64Hasher::new(0);
        hash_abi_descriptor_into(&mut h, d);
        h.digest()
    };

    let base_core = core_abi_descriptor();
    let base_h = hash_desc(&base_core);

    // 1. Namespace mutation
    let mut ns_mut = base_core;
    ns_mut.namespace = Some("DifferentNamespace");
    assert_ne!(hash_desc(&ns_mut), base_h, "namespace must affect hash");

    // 2. Const value mutation
    let mut consts = base_core.consts.to_vec();
    consts[0] = AbiConstDescriptor {
        value: consts[0].value.wrapping_add(1),
        ..consts[0]
    };
    let mut const_mut = base_core;
    const_mut.consts = Box::leak(consts.into_boxed_slice());
    assert_ne!(hash_desc(&const_mut), base_h, "const value must affect hash");

    // 3. Alias target mutation
    let mut aliases = base_core.aliases.to_vec();
    aliases[0] = AbiAliasDescriptor {
        target: "u64",
        ..aliases[0]
    };
    let mut alias_mut = base_core;
    alias_mut.aliases = Box::leak(aliases.into_boxed_slice());
    assert_ne!(hash_desc(&alias_mut), base_h, "alias target must affect hash");

    // 4. Callback param & return mutations
    let mut cbs = base_core.callbacks.to_vec();
    let mut cb_params = cbs[0].params.to_vec();
    cb_params[0] = AbiParamDescriptor {
        ty: "u64",
        ..cb_params[0]
    };
    cbs[0] = AbiCallbackDescriptor {
        params: Box::leak(cb_params.into_boxed_slice()),
        ..cbs[0]
    };
    let mut cb_param_mut = base_core;
    cb_param_mut.callbacks = Box::leak(cbs.into_boxed_slice());
    assert_ne!(
        hash_desc(&cb_param_mut),
        base_h,
        "callback param type must affect hash"
    );

    let mut cbs_ret = base_core.callbacks.to_vec();
    cbs_ret[0] = AbiCallbackDescriptor {
        return_type: "u64",
        ..cbs_ret[0]
    };
    let mut cb_ret_mut = base_core;
    cb_ret_mut.callbacks = Box::leak(cbs_ret.into_boxed_slice());
    assert_ne!(
        hash_desc(&cb_ret_mut),
        base_h,
        "callback return type must affect hash"
    );

    // 5. Enum discriminant mutation
    let mut enums = base_core.enums.to_vec();
    let mut vars = enums[0].variants.to_vec();
    vars[0] = AbiEnumVariantDescriptor {
        value: vars[0].value + 99,
        ..vars[0]
    };
    enums[0] = AbiEnumDescriptor {
        variants: Box::leak(vars.into_boxed_slice()),
        ..enums[0]
    };
    let mut enum_mut = base_core;
    enum_mut.enums = Box::leak(enums.into_boxed_slice());
    assert_ne!(
        hash_desc(&enum_mut),
        base_h,
        "enum discriminant must affect hash"
    );

    // 6. Struct field type, expected_offset, and explicit_align mutations
    let mut recs = base_core.records.to_vec();
    let AbiRecordDescriptor::Struct(first_s) = recs[0] else {
        panic!("expected struct record at index 0");
    };
    let mut fields = first_s.fields.to_vec();
    fields[0] = AbiFieldDescriptor {
        ty: "*mut u8",
        ..fields[0]
    };
    recs[0] = AbiRecordDescriptor::Struct(AbiStructDescriptor {
        fields: Box::leak(fields.into_boxed_slice()),
        ..first_s
    });
    let mut field_ty_mut = base_core;
    field_ty_mut.records = Box::leak(recs.into_boxed_slice());
    assert_ne!(
        hash_desc(&field_ty_mut),
        base_h,
        "struct field type must affect hash"
    );

    let mut recs_off = base_core.records.to_vec();
    let mut fields_off = first_s.fields.to_vec();
    fields_off[0] = AbiFieldDescriptor {
        expected_offset: Some(8),
        ..fields_off[0]
    };
    recs_off[0] = AbiRecordDescriptor::Struct(AbiStructDescriptor {
        fields: Box::leak(fields_off.into_boxed_slice()),
        ..first_s
    });
    let mut field_off_mut = base_core;
    field_off_mut.records = Box::leak(recs_off.into_boxed_slice());
    assert_ne!(
        hash_desc(&field_off_mut),
        base_h,
        "struct field expected_offset must affect hash"
    );

    let mut recs_al = base_core.records.to_vec();
    recs_al[0] = AbiRecordDescriptor::Struct(AbiStructDescriptor {
        explicit_align: Some(16),
        ..first_s
    });
    let mut align_mut = base_core;
    align_mut.records = Box::leak(recs_al.into_boxed_slice());
    assert_ne!(
        hash_desc(&align_mut),
        base_h,
        "struct explicit_align must affect hash"
    );

    // 7. Vtable slot order, optionality, and signature mutations
    let rt = runtime_abi_descriptor();
    let rt_h = hash_desc(&rt);
    let mut vts_order = rt.vtables.to_vec();
    let mut methods_order = vts_order[0].methods.to_vec();
    methods_order.swap(0, 1);
    vts_order[0] = AbiVtableDescriptor {
        methods: Box::leak(methods_order.into_boxed_slice()),
        ..vts_order[0]
    };
    let mut vt_order_mut = rt;
    vt_order_mut.vtables = Box::leak(vts_order.into_boxed_slice());
    assert_ne!(
        hash_desc(&vt_order_mut),
        rt_h,
        "vtable slot order must affect hash"
    );

    let mut vts_opt = rt.vtables.to_vec();
    let mut methods_opt = vts_opt[0].methods.to_vec();
    methods_opt[0] = AbiMethodDescriptor {
        optional: !methods_opt[0].optional,
        ..methods_opt[0]
    };
    vts_opt[0] = AbiVtableDescriptor {
        methods: Box::leak(methods_opt.into_boxed_slice()),
        ..vts_opt[0]
    };
    let mut vt_opt_mut = rt;
    vt_opt_mut.vtables = Box::leak(vts_opt.into_boxed_slice());
    assert_ne!(
        hash_desc(&vt_opt_mut),
        rt_h,
        "vtable slot optionality must affect hash"
    );

    let mut vts_sig = rt.vtables.to_vec();
    let mut methods_sig = vts_sig[0].methods.to_vec();
    methods_sig[0] = AbiMethodDescriptor {
        return_type: "u64",
        ..methods_sig[0]
    };
    vts_sig[0] = AbiVtableDescriptor {
        methods: Box::leak(methods_sig.into_boxed_slice()),
        ..vts_sig[0]
    };
    let mut vt_sig_mut = rt;
    vt_sig_mut.vtables = Box::leak(vts_sig.into_boxed_slice());
    assert_ne!(
        hash_desc(&vt_sig_mut),
        rt_h,
        "vtable slot signature must affect hash"
    );

    // 8. Core hash chained into Runtime hash (reconstructs exact `NVE_RUNTIME_SCHEMA_HASH`)
    let rt_tag = "nve.runtime_abi.v2";
    let mut rt_hasher_actual = Xxh64Hasher::new(0);
    rt_hasher_actual.update_u64(rt_tag.len() as u64);
    rt_hasher_actual.update(rt_tag.as_bytes());
    rt_hasher_actual.update_u64(nve_abi::NVE_CORE_ABI_SCHEMA_HASH);
    hash_abi_descriptor_into(&mut rt_hasher_actual, &rt);
    assert_eq!(
        rt_hasher_actual.digest(),
        nve_abi::NVE_RUNTIME_SCHEMA_HASH,
        "reconstructed runtime hash must match NVE_RUNTIME_SCHEMA_HASH"
    );

    let mut rt_hasher_perturbed = Xxh64Hasher::new(0);
    rt_hasher_perturbed.update_u64(rt_tag.len() as u64);
    rt_hasher_perturbed.update(rt_tag.as_bytes());
    rt_hasher_perturbed.update_u64(nve_abi::NVE_CORE_ABI_SCHEMA_HASH ^ 1);
    hash_abi_descriptor_into(&mut rt_hasher_perturbed, &rt);
    assert_ne!(
        rt_hasher_perturbed.digest(),
        nve_abi::NVE_RUNTIME_SCHEMA_HASH,
        "perturbing NVE_CORE_ABI_SCHEMA_HASH must change NVE_RUNTIME_SCHEMA_HASH"
    );
}
