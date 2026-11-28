//! Compile-time Runtime ABI schema hash ([`NVE_RUNTIME_SCHEMA_HASH`]) and single-source
//! codegen descriptor ([`runtime_abi_descriptor`]).

use core::mem::{offset_of, size_of};

use nve_hash::Xxh64Hasher;

use crate::schema::{
    AbiConstDescriptor, AbiEnumDescriptor, AbiFileDescriptor, AbiRecordDescriptor, NveEnumSchema,
    NveStructSchema,
};
use crate::table::NveTableView;

use super::vtable::hash_rt_signatures;
use super::{
    NveLayoutRect, NveNodeRow, NveRuntimeApi, NveRuntimeConfig, NveRuntimeMetrics,
    NveRuntimeStatus, NveRuntimeTextStyle, NveSceneView, NveScrollOffset, NveSurfaceMeasure,
    NVE_LAYOUT_RECT_SIZE, NVE_LINE_HEIGHT_AUTO, NVE_LINE_HEIGHT_PERCENT, NVE_LINE_HEIGHT_POINTS,
    NVE_MEASURE_FLAG_DRY_RUN, NVE_MEASURE_FLAG_IDEAL, NVE_NODE_ROW_FLAG_LIVE, NVE_NODE_ROW_SIZE,
    NVE_RUNTIME_ABI_VERSION, RUNTIME_API_SLOTS, RUNTIME_CALLBACK_TYPEDEFS,
};

const fn hash_rt_field(h: &mut Xxh64Hasher, name: &str, ty: &str, sep: &[u8], v: usize) {
    h.update(b"|");
    h.update(name.as_bytes());
    h.update(b":");
    h.update(ty.as_bytes());
    h.update(sep);
    h.update_u64(v as u64);
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

    h.update(b";NveSurfaceMeasure");
    h.update_u64(size_of::<NveSurfaceMeasure>() as u64);
    hash_rt_field(&mut h, "width", "f32", b"@", offset_of!(NveSurfaceMeasure, width));
    hash_rt_field(&mut h, "height", "f32", b"@", offset_of!(NveSurfaceMeasure, height));
    hash_rt_field(
        &mut h,
        "first_baseline",
        "f32",
        b"@",
        offset_of!(NveSurfaceMeasure, first_baseline),
    );
    hash_rt_field(
        &mut h,
        "last_baseline",
        "f32",
        b"@",
        offset_of!(NveSurfaceMeasure, last_baseline),
    );
    h.update_u64(NVE_MEASURE_FLAG_DRY_RUN as u64);
    h.update_u64(NVE_MEASURE_FLAG_IDEAL as u64);

    h.update(b";NveScrollOffset");
    h.update_u64(size_of::<NveScrollOffset>() as u64);
    hash_rt_field(&mut h, "node_id", "u32", b"@", offset_of!(NveScrollOffset, node_id));
    hash_rt_field(&mut h, "x", "f32", b"@", offset_of!(NveScrollOffset, x));
    hash_rt_field(&mut h, "y", "f32", b"@", offset_of!(NveScrollOffset, y));

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

    h.update(b";NveRuntimeConfig");
    h.update_u64(size_of::<NveRuntimeConfig>() as u64);
    hash_rt_field(&mut h, "surface_id", "u32", b"@", offset_of!(NveRuntimeConfig, surface_id));
    hash_rt_field(&mut h, "width", "f32", b"@", offset_of!(NveRuntimeConfig, width));
    hash_rt_field(&mut h, "height", "f32", b"@", offset_of!(NveRuntimeConfig, height));
    hash_rt_field(&mut h, "scale_factor", "f32", b"@", offset_of!(NveRuntimeConfig, scale_factor));
    hash_rt_field(&mut h, "flags", "u32", b"@", offset_of!(NveRuntimeConfig, flags));
    hash_rt_field(&mut h, "_reserved", "u32", b"@", offset_of!(NveRuntimeConfig, _reserved));

    h.update(b";NveRuntimeTextStyle");
    hash_rt_field(&mut h, "font_size", "f32", b"@", offset_of!(NveRuntimeTextStyle, font_size));
    hash_rt_field(
        &mut h,
        "letter_spacing",
        "f32",
        b"@",
        offset_of!(NveRuntimeTextStyle, letter_spacing),
    );
    hash_rt_field(&mut h, "line_height", "f32", b"@", offset_of!(NveRuntimeTextStyle, line_height));
    hash_rt_field(&mut h, "max_lines", "u32", b"@", offset_of!(NveRuntimeTextStyle, max_lines));
    hash_rt_field(&mut h, "font_weight", "u16", b"@", offset_of!(NveRuntimeTextStyle, font_weight));
    hash_rt_field(
        &mut h,
        "line_height_unit",
        "u8",
        b"@",
        offset_of!(NveRuntimeTextStyle, line_height_unit),
    );
    hash_rt_field(&mut h, "font_style", "u8", b"@", offset_of!(NveRuntimeTextStyle, font_style));
    hash_rt_field(&mut h, "white_space", "u8", b"@", offset_of!(NveRuntimeTextStyle, white_space));
    hash_rt_field(&mut h, "word_break", "u8", b"@", offset_of!(NveRuntimeTextStyle, word_break));
    hash_rt_field(
        &mut h,
        "overflow_wrap",
        "u8",
        b"@",
        offset_of!(NveRuntimeTextStyle, overflow_wrap),
    );
    hash_rt_field(&mut h, "_reserved", "u8", b"@", offset_of!(NveRuntimeTextStyle, _reserved));
    hash_rt_field(
        &mut h,
        "font_family",
        "NveStrSlice",
        b"@",
        offset_of!(NveRuntimeTextStyle, font_family),
    );
    h.update_u64(NVE_LINE_HEIGHT_POINTS as u64);
    h.update_u64(NVE_LINE_HEIGHT_PERCENT as u64);
    h.update_u64(NVE_LINE_HEIGHT_AUTO as u64);

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
    hash_rt_field(
        &mut h,
        "pair_connected",
        "u32",
        b"@",
        offset_of!(NveRuntimeMetrics, pair_connected),
    );
    hash_rt_field(&mut h, "_reserved", "u32", b"@", offset_of!(NveRuntimeMetrics, _reserved));

    h.update(b";NveTableView");
    hash_rt_field(&mut h, "key_lo", "u32", b"@", offset_of!(NveTableView, key_lo));
    hash_rt_field(&mut h, "key_hi", "u32", b"@", offset_of!(NveTableView, key_hi));
    hash_rt_field(&mut h, "slot", "u32", b"@", offset_of!(NveTableView, slot));
    hash_rt_field(&mut h, "words", "u32", b"@", offset_of!(NveTableView, words));
    hash_rt_field(&mut h, "row_count", "u32", b"@", offset_of!(NveTableView, row_count));
    hash_rt_field(&mut h, "dirty_count", "u32", b"@", offset_of!(NveTableView, dirty_count));
    hash_rt_field(
        &mut h,
        "row_index_dir_len",
        "u32",
        b"@",
        offset_of!(NveTableView, row_index_dir_len),
    );
    hash_rt_field(
        &mut h,
        "row_index_page_count",
        "u32",
        b"@",
        offset_of!(NveTableView, row_index_page_count),
    );
    hash_rt_field(&mut h, "rows", "*const u32", b"#", 8);
    hash_rt_field(&mut h, "owners", "*const u32", b"#", 9);
    hash_rt_field(&mut h, "dirty_rows", "*const u32", b"#", 10);
    hash_rt_field(&mut h, "row_index_dir", "*const u32", b"#", 11);
    hash_rt_field(&mut h, "row_index_pages", "*const u32", b"#", 12);
    h.update_u64(crate::table::NVE_ROW_INDEX_PAGE_BITS as u64);

    h.update(b";NveSceneView");
    hash_rt_field(&mut h, "schema_hash", "u64", b"@", offset_of!(NveSceneView, schema_hash));
    hash_rt_field(&mut h, "frame_id", "u64", b"@", offset_of!(NveSceneView, frame_id));
    hash_rt_field(&mut h, "node_capacity", "u32", b"@", offset_of!(NveSceneView, node_capacity));
    hash_rt_field(&mut h, "dirty_count", "u32", b"@", offset_of!(NveSceneView, dirty_count));
    hash_rt_field(&mut h, "removed_count", "u32", b"@", offset_of!(NveSceneView, removed_count));
    hash_rt_field(&mut h, "text_slab_len", "u32", b"@", offset_of!(NveSceneView, text_slab_len));
    hash_rt_field(
        &mut h,
        "children_slab_len",
        "u32",
        b"@",
        offset_of!(NveSceneView, children_slab_len),
    );
    hash_rt_field(&mut h, "table_count", "u32", b"@", offset_of!(NveSceneView, table_count));
    hash_rt_field(&mut h, "scroll_count", "u32", b"@", offset_of!(NveSceneView, scroll_count));
    hash_rt_field(&mut h, "scene_epoch", "u32", b"@", offset_of!(NveSceneView, scene_epoch));
    hash_rt_field(
        &mut h,
        "dirty_resource_count",
        "u32",
        b"@",
        offset_of!(NveSceneView, dirty_resource_count),
    );
    hash_rt_field(&mut h, "moved_count", "u32", b"@", offset_of!(NveSceneView, moved_count));
    hash_rt_field(&mut h, "rows", "*const NveNodeRow", b"#", 10);
    hash_rt_field(&mut h, "rects", "*const NveLayoutRect", b"#", 11);
    hash_rt_field(&mut h, "text_slab", "*const u8", b"#", 12);
    hash_rt_field(&mut h, "children_slab", "*const u32", b"#", 13);
    hash_rt_field(&mut h, "dirty_indices", "*const u32", b"#", 14);
    hash_rt_field(&mut h, "removed_ids", "*const u32", b"#", 15);
    hash_rt_field(&mut h, "tables", "*const NveTableView", b"#", 16);
    hash_rt_field(&mut h, "scroll_offsets", "*const NveScrollOffset", b"#", 17);
    hash_rt_field(&mut h, "dirty_resources", "*const u32", b"#", 18);
    hash_rt_field(&mut h, "moved_indices", "*const u32", b"#", 19);

    h.update(b";NveRuntimeApi");
    hash_rt_field(&mut h, "abi_version", "u32", b"@", offset_of!(NveRuntimeApi, abi_version));
    hash_rt_field(&mut h, "struct_size", "u32", b"@", offset_of!(NveRuntimeApi, struct_size));
    hash_rt_field(&mut h, "schema_hash", "u64", b"@", offset_of!(NveRuntimeApi, schema_hash));
    hash_rt_signatures(&mut h, b";callbacks", RUNTIME_CALLBACK_TYPEDEFS);
    hash_rt_signatures(&mut h, b";slots", RUNTIME_API_SLOTS);
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
        name: "NVE_LINE_HEIGHT_POINTS",
        ty: "u32",
        value: NVE_LINE_HEIGHT_POINTS as u64,
        hex: false,
        doc: Some("`NveRuntimeTextStyle::line_height_unit`: absolute line height in points."),
    },
    AbiConstDescriptor {
        name: "NVE_LINE_HEIGHT_PERCENT",
        ty: "u32",
        value: NVE_LINE_HEIGHT_PERCENT as u64,
        hex: false,
        doc: Some("`NveRuntimeTextStyle::line_height_unit`: multiplier × 100 of `font_size`."),
    },
    AbiConstDescriptor {
        name: "NVE_LINE_HEIGHT_AUTO",
        ty: "u32",
        value: NVE_LINE_HEIGHT_AUTO as u64,
        hex: false,
        doc: Some("`NveRuntimeTextStyle::line_height_unit`: `normal` (the font's own line height)."),
    },
    AbiConstDescriptor {
        name: "NVE_MEASURE_FLAG_DRY_RUN",
        ty: "u32",
        value: NVE_MEASURE_FLAG_DRY_RUN as u64,
        hex: false,
        doc: Some("`measure_surface` flag: pure sizing pass; never mutates committed viewport/rects or publishes."),
    },
    AbiConstDescriptor {
        name: "NVE_MEASURE_FLAG_IDEAL",
        ty: "u32",
        value: NVE_MEASURE_FLAG_IDEAL as u64,
        hex: false,
        doc: Some("`measure_surface` flag: intrinsic unconstrained sizing pass."),
    },
];

const RUNTIME_ENUMS: &[AbiEnumDescriptor] = &[<NveRuntimeStatus as NveEnumSchema>::SCHEMA];

const RUNTIME_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveLayoutRect as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSurfaceMeasure as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveScrollOffset as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveNodeRow as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeConfig as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeMetrics as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeTextStyle as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveSceneView as NveStructSchema>::SCHEMA),
];

/// Returns the single-source-of-truth Runtime ABI descriptor derived directly from the compiled
/// `#[derive(NveAbi)]` types in `packages/core/abi/src/runtime/`.
pub const fn runtime_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveRuntimeAbi"),
        consts: RUNTIME_CONSTS,
        aliases: &[],
        enums: RUNTIME_ENUMS,
        records: RUNTIME_RECORDS,
        modules: &[],
    }
}
