//! Deterministic `NVE_RUNTIME_SCHEMA_HASH` over the runtime ABI layout and the single-source
//! `runtime_abi_descriptor()` consumed by codegen.

use core::mem::{offset_of, size_of};

use nve_hash::Xxh64Hasher;

use super::*;
use crate::schema::{
    AbiConstDescriptor, AbiEnumDescriptor, AbiFileDescriptor, AbiRecordDescriptor, NveEnumSchema,
    NveStructSchema,
};
use crate::table::NveTableView;

const fn hash_rt_field(h: &mut Xxh64Hasher, name: &str, ty: &str, sep: &[u8], v: usize) {
    h.update(b"|");
    h.update(name.as_bytes());
    h.update(b":");
    h.update(ty.as_bytes());
    h.update(sep);
    h.update_u64(v as u64);
}

const fn hash_rt_method(h: &mut Xxh64Hasher, idx: usize, name: &str, sig: &str) {
    h.update(b"|fn#");
    h.update_u64(idx as u64);
    h.update(b":");
    h.update(name.as_bytes());
    h.update(b"=");
    h.update(sig.as_bytes());
}

const fn hash_rt_variant(h: &mut Xxh64Hasher, name: &str, value: i32) {
    h.update(b"|");
    h.update(name.as_bytes());
    h.update(b"=");
    h.update_u64(value as i64 as u64);
}

const fn compute_runtime_schema_hash() -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(b"nve.runtime_abi.v1");
    h.update_u64(NVE_RUNTIME_ABI_VERSION as u64);

    h.update(b";NveRuntimeStatus");
    hash_rt_variant(&mut h, "Ok", NveRuntimeStatus::Ok.as_i32());
    hash_rt_variant(&mut h, "InvalidArg", NveRuntimeStatus::InvalidArg.as_i32());
    hash_rt_variant(&mut h, "Unsupported", NveRuntimeStatus::Unsupported.as_i32());
    hash_rt_variant(&mut h, "Busy", NveRuntimeStatus::Busy.as_i32());
    hash_rt_variant(&mut h, "Io", NveRuntimeStatus::Io.as_i32());
    hash_rt_variant(&mut h, "Panic", NveRuntimeStatus::Panic.as_i32());
    hash_rt_variant(&mut h, "NotFound", NveRuntimeStatus::NotFound.as_i32());
    hash_rt_variant(&mut h, "Error", NveRuntimeStatus::Error.as_i32());

    h.update(b";NveLayoutRect");
    h.update_u64(size_of::<NveLayoutRect>() as u64);
    hash_rt_field(&mut h, "x", "f32", b"@", offset_of!(NveLayoutRect, x));
    hash_rt_field(&mut h, "y", "f32", b"@", offset_of!(NveLayoutRect, y));
    hash_rt_field(&mut h, "width", "f32", b"@", offset_of!(NveLayoutRect, width));
    hash_rt_field(&mut h, "height", "f32", b"@", offset_of!(NveLayoutRect, height));

    h.update(b";NveNodeRow");
    h.update_u64(size_of::<NveNodeRow>() as u64);
    h.update_u64(NVE_NODE_ROW_FLAG_LIVE as u64);
    hash_rt_field(&mut h, "table_set", "[u32;4]", b"@", offset_of!(NveNodeRow, table_set));
    hash_rt_field(&mut h, "parent", "u32", b"@", offset_of!(NveNodeRow, parent));
    hash_rt_field(&mut h, "child_off", "u32", b"@", offset_of!(NveNodeRow, child_off));
    hash_rt_field(&mut h, "child_len", "u32", b"@", offset_of!(NveNodeRow, child_len));
    hash_rt_field(&mut h, "generation", "u8", b"@", offset_of!(NveNodeRow, generation));
    hash_rt_field(&mut h, "primary_slot", "u8", b"@", offset_of!(NveNodeRow, primary_slot));
    hash_rt_field(&mut h, "effects", "u8", b"@", offset_of!(NveNodeRow, effects));
    hash_rt_field(&mut h, "flags", "u8", b"@", offset_of!(NveNodeRow, flags));

    h.update(b";NveResourceView");
    h.update_u64(NVE_RESOURCE_FLAG_EPHEMERAL as u64);
    hash_rt_field(&mut h, "kind_lo", "u32", b"@", offset_of!(NveResourceView, kind_lo));
    hash_rt_field(&mut h, "kind_hi", "u32", b"@", offset_of!(NveResourceView, kind_hi));
    hash_rt_field(&mut h, "len", "u32", b"@", offset_of!(NveResourceView, len));
    hash_rt_field(&mut h, "flags", "u32", b"@", offset_of!(NveResourceView, flags));
    hash_rt_field(&mut h, "bytes", "*const u8", b"#", 4);

    h.update(b";NveRuntimeConfig");
    h.update_u64(size_of::<NveRuntimeConfig>() as u64);
    hash_rt_field(&mut h, "surface_id", "u32", b"@", offset_of!(NveRuntimeConfig, surface_id));
    hash_rt_field(&mut h, "width", "f32", b"@", offset_of!(NveRuntimeConfig, width));
    hash_rt_field(&mut h, "height", "f32", b"@", offset_of!(NveRuntimeConfig, height));
    hash_rt_field(&mut h, "scale_factor", "f32", b"@", offset_of!(NveRuntimeConfig, scale_factor));
    hash_rt_field(&mut h, "flags", "u32", b"@", offset_of!(NveRuntimeConfig, flags));
    hash_rt_field(&mut h, "_reserved", "u32", b"@", offset_of!(NveRuntimeConfig, _reserved));

    h.update(b";NveRuntimeMetrics");
    h.update_u64(size_of::<NveRuntimeMetrics>() as u64);
    hash_rt_field(&mut h, "frame_id", "u64", b"@", offset_of!(NveRuntimeMetrics, frame_id));
    hash_rt_field(&mut h, "reconcile_us", "u64", b"@", offset_of!(NveRuntimeMetrics, reconcile_us));
    hash_rt_field(&mut h, "layout_us", "u64", b"@", offset_of!(NveRuntimeMetrics, layout_us));
    hash_rt_field(&mut h, "publish_us", "u64", b"@", offset_of!(NveRuntimeMetrics, publish_us));
    hash_rt_field(&mut h, "wire_bytes_rx", "u64", b"@", offset_of!(NveRuntimeMetrics, wire_bytes_rx));
    hash_rt_field(&mut h, "wire_bytes_tx", "u64", b"@", offset_of!(NveRuntimeMetrics, wire_bytes_tx));
    hash_rt_field(&mut h, "live_nodes", "u32", b"@", offset_of!(NveRuntimeMetrics, live_nodes));
    hash_rt_field(&mut h, "dirty_nodes", "u32", b"@", offset_of!(NveRuntimeMetrics, dirty_nodes));
    hash_rt_field(&mut h, "pair_connected", "u32", b"@", offset_of!(NveRuntimeMetrics, pair_connected));
    hash_rt_field(&mut h, "_reserved", "u32", b"@", offset_of!(NveRuntimeMetrics, _reserved));

    h.update(b";NveTableView");
    hash_rt_field(&mut h, "key_lo", "u32", b"@", offset_of!(NveTableView, key_lo));
    hash_rt_field(&mut h, "key_hi", "u32", b"@", offset_of!(NveTableView, key_hi));
    hash_rt_field(&mut h, "slot", "u32", b"@", offset_of!(NveTableView, slot));
    hash_rt_field(&mut h, "words", "u32", b"@", offset_of!(NveTableView, words));
    hash_rt_field(&mut h, "row_count", "u32", b"@", offset_of!(NveTableView, row_count));
    hash_rt_field(&mut h, "dirty_count", "u32", b"@", offset_of!(NveTableView, dirty_count));
    hash_rt_field(&mut h, "rows", "*const u32", b"#", 6);
    hash_rt_field(&mut h, "owners", "*const u32", b"#", 7);
    hash_rt_field(&mut h, "dirty_rows", "*const u32", b"#", 8);

    h.update(b";NveRuntimeSceneView");
    hash_rt_field(&mut h, "schema_hash", "u64", b"@", offset_of!(NveRuntimeSceneView, schema_hash));
    hash_rt_field(&mut h, "frame_id", "u64", b"@", offset_of!(NveRuntimeSceneView, frame_id));
    hash_rt_field(&mut h, "node_capacity", "u32", b"@", offset_of!(NveRuntimeSceneView, node_capacity));
    hash_rt_field(&mut h, "dirty_count", "u32", b"@", offset_of!(NveRuntimeSceneView, dirty_count));
    hash_rt_field(&mut h, "removed_count", "u32", b"@", offset_of!(NveRuntimeSceneView, removed_count));
    hash_rt_field(&mut h, "text_slab_len", "u32", b"@", offset_of!(NveRuntimeSceneView, text_slab_len));
    hash_rt_field(&mut h, "children_slab_len", "u32", b"@", offset_of!(NveRuntimeSceneView, children_slab_len));
    hash_rt_field(&mut h, "table_count", "u32", b"@", offset_of!(NveRuntimeSceneView, table_count));
    hash_rt_field(&mut h, "resource_capacity", "u32", b"@", offset_of!(NveRuntimeSceneView, resource_capacity));
    hash_rt_field(&mut h, "dirty_resource_count", "u32", b"@", offset_of!(NveRuntimeSceneView, dirty_resource_count));
    hash_rt_field(&mut h, "rows", "*const NveNodeRow", b"#", 10);
    hash_rt_field(&mut h, "rects", "*const NveLayoutRect", b"#", 11);
    hash_rt_field(&mut h, "text_slab", "*const u8", b"#", 12);
    hash_rt_field(&mut h, "children_slab", "*const u32", b"#", 13);
    hash_rt_field(&mut h, "dirty_indices", "*const u32", b"#", 14);
    hash_rt_field(&mut h, "removed_ids", "*const u32", b"#", 15);
    hash_rt_field(&mut h, "tables", "*const NveTableView", b"#", 16);
    hash_rt_field(&mut h, "resources", "*const NveResourceView", b"#", 17);
    hash_rt_field(&mut h, "dirty_resources", "*const u32", b"#", 18);

    h.update(b";NveRuntimeApi");
    hash_rt_field(&mut h, "abi_version", "u32", b"@", offset_of!(NveRuntimeApi, abi_version));
    hash_rt_field(&mut h, "struct_size", "u32", b"@", offset_of!(NveRuntimeApi, struct_size));
    hash_rt_field(&mut h, "schema_hash", "u64", b"@", offset_of!(NveRuntimeApi, schema_hash));
    hash_rt_method(&mut h, 0, "create", "(const NveRuntimeConfig*)->NveRuntimeHandle");
    hash_rt_method(&mut h, 1, "destroy", "(NveRuntimeHandle)->void");
    hash_rt_method(&mut h, 2, "set_wake_callback", "(NveRuntimeHandle,NveRuntimeWakeFn,void*)->i32");
    hash_rt_method(&mut h, 3, "set_viewport", "(NveRuntimeHandle,f32,f32,f32)->i32");
    hash_rt_method(&mut h, 4, "attach_app_interface", "(NveRuntimeHandle,NveGetInterfaceFn)->i32");
    hash_rt_method(&mut h, 5, "submit_nve2", "(NveRuntimeHandle,NveBufferSlice)->i32");
    hash_rt_method(&mut h, 6, "tick", "(NveRuntimeHandle,f32,u32*)->i32");
    hash_rt_method(&mut h, 7, "dispatch_events", "(NveRuntimeHandle,NveEventBatch,u32*)->i32");
    hash_rt_method(&mut h, 8, "hit_test", "(NveRuntimeHandle,f32,f32,u32*)->i32");
    hash_rt_method(&mut h, 9, "scroll_by", "(NveRuntimeHandle,u32,f32,f32,f32*,f32*)->i32");
    hash_rt_method(&mut h, 10, "scene_acquire", "(NveRuntimeHandle,NveRuntimeSceneView*)->i32");
    hash_rt_method(&mut h, 11, "scene_release", "(NveRuntimeHandle)->i32");
    hash_rt_method(&mut h, 12, "get_metrics", "(NveRuntimeHandle,NveRuntimeMetrics*)->i32");
    hash_rt_method(&mut h, 13, "pair_connect", "(NveRuntimeHandle,NveStrSlice,NveStrSlice)->i32");
    hash_rt_method(&mut h, 14, "pair_disconnect", "(NveRuntimeHandle)->i32");
    hash_rt_method(&mut h, 15, "pair_is_connected", "(NveRuntimeHandle)->u32");
    hash_rt_method(&mut h, 16, "last_error", "(u8*,usize)->usize");
    hash_rt_method(
        &mut h,
        17,
        "set_text_measurer",
        "(NveRuntimeHandle,NveTextMeasureFn(void*,NveStrSlice,f32,u32,f32,f32*,f32*)->i32,void*)->i32",
    );
    h.digest()
}

/// Deterministic XXH64 fingerprint of the zero-copy runtime scene and vtable ABI.
pub const NVE_RUNTIME_SCHEMA_HASH: u64 = compute_runtime_schema_hash();

const RUNTIME_CONSTS: &[AbiConstDescriptor] = &[
    AbiConstDescriptor {
        name: "NVE_RUNTIME_ABI_VERSION",
        ty: "u32",
        value: NVE_RUNTIME_ABI_VERSION as u64,
        hex: false,
        doc: Some("Runtime C ABI version passed to `nve_runtime_get_interface`."),
    },
    AbiConstDescriptor {
        name: "NVE_LAYOUT_RECT_SIZE",
        ty: "u32",
        value: NVE_LAYOUT_RECT_SIZE as u64,
        hex: false,
        doc: Some("Exact byte stride of one `NveLayoutRect` record (16 bytes)."),
    },
    AbiConstDescriptor {
        name: "NVE_NODE_ROW_SIZE",
        ty: "u32",
        value: NVE_NODE_ROW_SIZE as u64,
        hex: false,
        doc: Some("Exact byte stride of one `NveNodeRow` record (32 bytes)."),
    },
    AbiConstDescriptor {
        name: "NVE_NODE_ROW_FLAG_LIVE",
        ty: "u32",
        value: NVE_NODE_ROW_FLAG_LIVE as u64,
        hex: false,
        doc: Some("Bit 0 in `NveNodeRow::flags` indicating an allocated live node slot."),
    },
    AbiConstDescriptor {
        name: "NVE_RESOURCE_FLAG_EPHEMERAL",
        ty: "u32",
        value: NVE_RESOURCE_FLAG_EPHEMERAL as u64,
        hex: false,
        doc: Some("Bit 0 in `NveResourceView::flags`: the resource is rewritten in place."),
    },
];

const RUNTIME_ENUMS: &[AbiEnumDescriptor] = &[<NveRuntimeStatus as NveEnumSchema>::SCHEMA];

const RUNTIME_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveLayoutRect as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveNodeRow as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeConfig as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeMetrics as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveTableView as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveResourceView as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeSceneView as NveStructSchema>::SCHEMA),
];

/// Returns the single-source-of-truth Runtime ABI descriptor derived directly from the compiled
/// `#[derive(NveAbi)]` types in `packages/core/abi/src/runtime.rs`.
pub const fn runtime_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveRuntimeAbi"),
        consts: RUNTIME_CONSTS,
        aliases: &[],
        enums: RUNTIME_ENUMS,
        records: RUNTIME_RECORDS,
        modules: &[],
        c_functions: &[],
    }
}
