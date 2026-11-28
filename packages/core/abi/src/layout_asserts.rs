//! Unconditional compile-time ABI layout verification for every `#[repr(C)]` vtable,
//! POD struct, union, and FFI enum in `nve-abi`.
//!
//! Mirrors the C11/C++20 `NVE_STATIC_ASSERT` checks emitted into `nve_abi.generated.h`
//! and `nve_runtime_abi.generated.h` so any Rust/C layout divergence fails compilation
//! immediately across both 32-bit and 64-bit targets.

use core::mem::{align_of, offset_of, size_of};

use crate::event::{
    InteractionEvent, LayoutEvent, NveEvent, NveEventBatch, RawEventBytes, ScrollEvent,
    TextInputEvent, NVE_EVENT_ALIGN, NVE_EVENT_SIZE,
};
use crate::ffi::{NveBufferSlice, NveGestureDisposition, NveIntrinsicSize, NveSizeConstraints};
use crate::host::{
    NveHostCallbacks, NveHostEventBatchHeader, NveHostEventRecordHeader, NveHostStatus,
};
use crate::runtime::{
    NveLayoutRect, NveNodeRow, NveRuntimeConfig, NveRuntimeMetrics, NveRuntimeStatus,
    NveSceneView, NveScrollOffset, NveSurfaceMeasure, NVE_LAYOUT_RECT_SIZE, NVE_NODE_ROW_SIZE,
};
use crate::value::{
    NveCommandSlice, NveMapEntry, NvePropEntry, NvePropSlice, NveStrSlice, NveValue,
    NveValueArray, NveValueData, NveValueMap, NveValueTag,
};

const PTR_SIZE: usize = size_of::<*const ()>();
const PTR_ALIGN: usize = align_of::<*const ()>();

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

// Pointer-width-aware assertions (compile-checked on both 32-bit and 64-bit targets).
const _: () = {
    // ---- Repr enums ----
    assert!(size_of::<NveGestureDisposition>() == 1);
    assert!(align_of::<NveGestureDisposition>() == 1);
    assert!(size_of::<InteractionEvent>() == 1);
    assert!(align_of::<InteractionEvent>() == 1);
    assert!(size_of::<TextInputEvent>() == 1);
    assert!(align_of::<TextInputEvent>() == 1);
    assert!(size_of::<ScrollEvent>() == 1);
    assert!(align_of::<ScrollEvent>() == 1);
    assert!(size_of::<LayoutEvent>() == 1);
    assert!(align_of::<LayoutEvent>() == 1);
    assert!(size_of::<NveHostStatus>() == 4);
    assert!(align_of::<NveHostStatus>() == 4);
    assert!(size_of::<NveValueTag>() == 4);
    assert!(align_of::<NveValueTag>() == 4);

    // ---- Core POD structs (ffi.rs) ----
    assert!(size_of::<NveBufferSlice>() == 2 * PTR_SIZE);
    assert!(align_of::<NveBufferSlice>() == PTR_ALIGN);
    assert!(offset_of!(NveBufferSlice, ptr) == 0);
    assert!(offset_of!(NveBufferSlice, len) == PTR_SIZE);

    assert!(size_of::<NveSizeConstraints>() == 16);
    assert!(align_of::<NveSizeConstraints>() == 4);
    assert!(offset_of!(NveSizeConstraints, min_w) == 0);
    assert!(offset_of!(NveSizeConstraints, min_h) == 4);
    assert!(offset_of!(NveSizeConstraints, max_w) == 8);
    assert!(offset_of!(NveSizeConstraints, max_h) == 12);

    assert!(size_of::<NveIntrinsicSize>() == 8);
    assert!(align_of::<NveIntrinsicSize>() == 4);
    assert!(offset_of!(NveIntrinsicSize, width) == 0);
    assert!(offset_of!(NveIntrinsicSize, height) == 4);

    // ---- Host capability structs (vtable slots locked by #[derive(NveAbi)]) ----
    assert!(size_of::<NveHostCallbacks>() == 3 * PTR_SIZE);
    assert!(align_of::<NveHostCallbacks>() == PTR_ALIGN);
    assert!(offset_of!(NveHostCallbacks, host_ctx) == 0);
    assert!(offset_of!(NveHostCallbacks, invoke) == PTR_SIZE);
    assert!(offset_of!(NveHostCallbacks, channel_publish) == 2 * PTR_SIZE);

    assert!(size_of::<NveHostEventBatchHeader>() == 8);
    assert!(offset_of!(NveHostEventBatchHeader, magic) == 0);
    assert!(offset_of!(NveHostEventBatchHeader, version) == 4);
    assert!(offset_of!(NveHostEventBatchHeader, count) == 6);
    assert!(size_of::<NveHostEventRecordHeader>() == 32);
    assert!(align_of::<NveHostEventRecordHeader>() == 8);
    assert!(offset_of!(NveHostEventRecordHeader, kind) == 0);
    assert!(offset_of!(NveHostEventRecordHeader, flags) == 1);
    assert!(offset_of!(NveHostEventRecordHeader, _pad) == 2);
    assert!(offset_of!(NveHostEventRecordHeader, status) == 4);
    assert!(offset_of!(NveHostEventRecordHeader, service_id) == 8);
    assert!(offset_of!(NveHostEventRecordHeader, method_or_event_or_channel) == 12);
    assert!(offset_of!(NveHostEventRecordHeader, request_id) == 16);
    assert!(offset_of!(NveHostEventRecordHeader, len) == 24);
    assert!(offset_of!(NveHostEventRecordHeader, _pad2) == 28);

    // ---- Direct FFI value & event types (value/ffi_types.rs) ----
    assert!(size_of::<NveStrSlice>() == 2 * PTR_SIZE);
    assert!(align_of::<NveStrSlice>() == PTR_ALIGN);
    assert!(offset_of!(NveStrSlice, ptr) == 0);
    assert!(offset_of!(NveStrSlice, len) == PTR_SIZE);

    assert!(size_of::<NveValueArray>() == 2 * PTR_SIZE);
    assert!(align_of::<NveValueArray>() == PTR_ALIGN);
    assert!(offset_of!(NveValueArray, ptr) == 0);
    assert!(offset_of!(NveValueArray, len) == PTR_SIZE);

    assert!(size_of::<NveValueMap>() == 2 * PTR_SIZE);
    assert!(align_of::<NveValueMap>() == PTR_ALIGN);
    assert!(offset_of!(NveValueMap, ptr) == 0);
    assert!(offset_of!(NveValueMap, len) == PTR_SIZE);

    assert!(offset_of!(NveValueData, bool_val) == 0);
    assert!(offset_of!(NveValueData, i64_val) == 0);
    assert!(offset_of!(NveValueData, f64_val) == 0);
    assert!(offset_of!(NveValueData, str_val) == 0);
    assert!(offset_of!(NveValueData, bytes_val) == 0);
    assert!(offset_of!(NveValueData, array_val) == 0);
    assert!(offset_of!(NveValueData, map_val) == 0);

    assert!(offset_of!(NveValue, tag) == 0);
    assert!(offset_of!(NveValue, data) == align_of::<NveValueData>());
    assert!(
        size_of::<NveValue>()
            == align_up(
                align_of::<NveValueData>() + size_of::<NveValueData>(),
                align_of::<NveValue>()
            )
    );

    assert!(offset_of!(NveMapEntry, key) == 0);
    assert!(offset_of!(NveMapEntry, value) == align_up(size_of::<NveStrSlice>(), align_of::<NveValue>()));
    assert!(
        size_of::<NveMapEntry>()
            == align_up(
                offset_of!(NveMapEntry, value) + size_of::<NveValue>(),
                align_of::<NveMapEntry>()
            )
    );

    assert!(size_of::<RawEventBytes>() == 8);
    assert!(align_of::<RawEventBytes>() == 4);
    assert!(offset_of!(RawEventBytes, offset) == 0);
    assert!(offset_of!(RawEventBytes, len) == 4);

    assert!(size_of::<NveEvent>() == NVE_EVENT_SIZE);
    assert!(align_of::<NveEvent>() == NVE_EVENT_ALIGN);
    assert!(offset_of!(NveEvent, timestamp) == 0);
    assert!(offset_of!(NveEvent, table) == 8);
    assert!(offset_of!(NveEvent, surface) == 16);
    assert!(offset_of!(NveEvent, node) == 20);
    assert!(offset_of!(NveEvent, event) == 24);
    assert!(offset_of!(NveEvent, flags) == 28);
    assert!(offset_of!(NveEvent, payload) == 32);
    assert!(offset_of!(NveEvent, bytes) == 56);

    assert!(offset_of!(NveEventBatch, events) == 0);
    assert!(offset_of!(NveEventBatch, bytes) == PTR_SIZE);
    assert!(offset_of!(NveEventBatch, event_count) == 2 * PTR_SIZE);
    assert!(offset_of!(NveEventBatch, bytes_len) == 2 * PTR_SIZE + 4);
    assert!(align_of::<NveEventBatch>() == PTR_ALIGN);
    assert!(size_of::<NveEventBatch>() == 2 * PTR_SIZE + 8);

    assert!(offset_of!(NvePropEntry, field_id) == 0);
    assert!(offset_of!(NvePropEntry, _pad) == 2);
    assert!(offset_of!(NvePropEntry, value) == 8);
    assert!(align_of::<NvePropEntry>() == align_of::<NveValue>());
    assert!(size_of::<NvePropEntry>() == 8 + size_of::<NveValue>());

    assert!(offset_of!(NvePropSlice, entries) == 0);
    assert!(offset_of!(NvePropSlice, len) == PTR_SIZE);
    assert!(offset_of!(NvePropSlice, _pad) == PTR_SIZE + 4);
    assert!(align_of::<NvePropSlice>() == PTR_ALIGN);
    assert!(size_of::<NvePropSlice>() == PTR_SIZE + 8);

    assert!(offset_of!(NveCommandSlice, opcode) == 0);
    assert!(offset_of!(NveCommandSlice, arg_count) == 2);
    assert!(offset_of!(NveCommandSlice, _pad) == 4);
    assert!(offset_of!(NveCommandSlice, args) == 8);
    assert!(align_of::<NveCommandSlice>() == PTR_ALIGN);
    assert!(size_of::<NveCommandSlice>() == 8 + PTR_SIZE);

    // ---- Word-table view (table.rs) ----
    assert!(offset_of!(crate::table::NveTableView, key_lo) == 0);
    assert!(offset_of!(crate::table::NveTableView, key_hi) == 4);
    assert!(offset_of!(crate::table::NveTableView, slot) == 8);
    assert!(offset_of!(crate::table::NveTableView, words) == 12);
    assert!(offset_of!(crate::table::NveTableView, row_count) == 16);
    assert!(offset_of!(crate::table::NveTableView, dirty_count) == 20);
    assert!(offset_of!(crate::table::NveTableView, row_index_dir_len) == 24);
    assert!(offset_of!(crate::table::NveTableView, row_index_page_count) == 28);
    assert!(offset_of!(crate::table::NveTableView, rows) == 32);
    assert!(offset_of!(crate::table::NveTableView, owners) == 32 + PTR_SIZE);
    assert!(offset_of!(crate::table::NveTableView, dirty_rows) == 32 + 2 * PTR_SIZE);
    assert!(offset_of!(crate::table::NveTableView, row_index_dir) == 32 + 3 * PTR_SIZE);
    assert!(offset_of!(crate::table::NveTableView, row_index_pages) == 32 + 4 * PTR_SIZE);
    assert!(
        size_of::<crate::table::NveTableView>()
            == align_up(32 + 5 * PTR_SIZE, align_of::<crate::table::NveTableView>())
    );

    // ---- Zero-copy runtime ABI types (runtime.rs) ----
    assert!(size_of::<NveRuntimeStatus>() == 4);
    assert!(align_of::<NveRuntimeStatus>() == 4);

    assert!(size_of::<NveLayoutRect>() == NVE_LAYOUT_RECT_SIZE);
    assert!(align_of::<NveLayoutRect>() == 4);
    assert!(offset_of!(NveLayoutRect, x) == 0);
    assert!(offset_of!(NveLayoutRect, y) == 4);
    assert!(offset_of!(NveLayoutRect, width) == 8);
    assert!(offset_of!(NveLayoutRect, height) == 12);

    assert!(size_of::<NveSurfaceMeasure>() == 16);
    assert!(align_of::<NveSurfaceMeasure>() == 4);
    assert!(offset_of!(NveSurfaceMeasure, width) == 0);
    assert!(offset_of!(NveSurfaceMeasure, height) == 4);
    assert!(offset_of!(NveSurfaceMeasure, first_baseline) == 8);
    assert!(offset_of!(NveSurfaceMeasure, last_baseline) == 12);

    assert!(size_of::<NveNodeRow>() == NVE_NODE_ROW_SIZE);
    assert!(align_of::<NveNodeRow>() == 4);
    assert!(offset_of!(NveNodeRow, table_set) == 0);
    assert!(offset_of!(NveNodeRow, parent) == 16);
    assert!(offset_of!(NveNodeRow, child_off) == 20);
    assert!(offset_of!(NveNodeRow, child_len) == 24);
    assert!(offset_of!(NveNodeRow, generation) == 28);
    assert!(offset_of!(NveNodeRow, primary_slot) == 29);
    assert!(offset_of!(NveNodeRow, effects) == 30);
    assert!(offset_of!(NveNodeRow, flags) == 31);

    assert!(size_of::<NveRuntimeConfig>() == 24);
    assert!(align_of::<NveRuntimeConfig>() == 4);
    assert!(offset_of!(NveRuntimeConfig, surface_id) == 0);
    assert!(offset_of!(NveRuntimeConfig, width) == 4);
    assert!(offset_of!(NveRuntimeConfig, height) == 8);
    assert!(offset_of!(NveRuntimeConfig, scale_factor) == 12);
    assert!(offset_of!(NveRuntimeConfig, flags) == 16);
    assert!(offset_of!(NveRuntimeConfig, _reserved) == 20);

    assert!(size_of::<NveRuntimeMetrics>() == 64);
    assert!(align_of::<NveRuntimeMetrics>() == align_of::<u64>()); // 4 on i386 System V
    assert!(offset_of!(NveRuntimeMetrics, frame_id) == 0);
    assert!(offset_of!(NveRuntimeMetrics, reconcile_us) == 8);
    assert!(offset_of!(NveRuntimeMetrics, layout_us) == 16);
    assert!(offset_of!(NveRuntimeMetrics, publish_us) == 24);
    assert!(offset_of!(NveRuntimeMetrics, wire_bytes_rx) == 32);
    assert!(offset_of!(NveRuntimeMetrics, wire_bytes_tx) == 40);
    assert!(offset_of!(NveRuntimeMetrics, live_nodes) == 48);
    assert!(offset_of!(NveRuntimeMetrics, dirty_nodes) == 52);
    assert!(offset_of!(NveRuntimeMetrics, pair_connected) == 56);
    assert!(offset_of!(NveRuntimeMetrics, _reserved) == 60);

    assert!(offset_of!(NveSceneView, schema_hash) == 0);
    assert!(offset_of!(NveSceneView, frame_id) == 8);
    assert!(offset_of!(NveSceneView, node_capacity) == 16);
    assert!(offset_of!(NveSceneView, dirty_count) == 20);
    assert!(offset_of!(NveSceneView, removed_count) == 24);
    assert!(offset_of!(NveSceneView, text_slab_len) == 28);
    assert!(offset_of!(NveSceneView, children_slab_len) == 32);
    assert!(offset_of!(NveSceneView, table_count) == 36);
    assert!(offset_of!(NveSceneView, scroll_count) == 40);
    assert!(offset_of!(NveSceneView, scene_epoch) == 44);
    assert!(offset_of!(NveSceneView, dirty_resource_count) == 48);
    assert!(offset_of!(NveSceneView, moved_count) == 52);
    assert!(offset_of!(NveSceneView, rows) == 56);
    assert!(offset_of!(NveSceneView, rects) == 56 + PTR_SIZE);
    assert!(offset_of!(NveSceneView, text_slab) == 56 + 2 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, children_slab) == 56 + 3 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, dirty_indices) == 56 + 4 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, removed_ids) == 56 + 5 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, tables) == 56 + 6 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, scroll_offsets) == 56 + 7 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, dirty_resources) == 56 + 8 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, moved_indices) == 56 + 9 * PTR_SIZE);
    assert!(
        size_of::<NveSceneView>()
            == align_up(56 + 10 * PTR_SIZE, align_of::<NveSceneView>())
    );
    assert!(size_of::<NveScrollOffset>() == 12);
    assert!(offset_of!(NveScrollOffset, node_id) == 0);
    assert!(offset_of!(NveScrollOffset, x) == 4);
    assert!(offset_of!(NveScrollOffset, y) == 8);

    // `NveRuntimeApi` slots & ILP32-aligned size are locked by `#[derive(NveAbi)]` in `runtime/vtable.rs`.
};

// Exact 64-bit ABI verification matching the checked-in C/Swift/Dart/TS/Java artifacts.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<NveBufferSlice>() == 16);
    assert!(align_of::<NveBufferSlice>() == 8);
    assert!(size_of::<NveStrSlice>() == 16);
    assert!(align_of::<NveStrSlice>() == 8);
    assert!(size_of::<NveValueArray>() == 16);
    assert!(align_of::<NveValueArray>() == 8);
    assert!(size_of::<NveValueMap>() == 16);
    assert!(align_of::<NveValueMap>() == 8);
    assert!(size_of::<NveValueData>() == 16);
    assert!(align_of::<NveValueData>() == 8);

    assert!(size_of::<NveValue>() == 24);
    assert!(align_of::<NveValue>() == 8);
    assert!(offset_of!(NveValue, data) == 8);

    assert!(size_of::<NveMapEntry>() == 40);
    assert!(align_of::<NveMapEntry>() == 8);
    assert!(offset_of!(NveMapEntry, value) == 16);

    assert!(size_of::<NveEvent>() == 64);
    assert!(align_of::<NveEvent>() == 64);
    assert!(size_of::<NveEventBatch>() == 24);
    assert!(align_of::<NveEventBatch>() == 8);
    assert!(offset_of!(NveEventBatch, bytes) == 8);
    assert!(offset_of!(NveEventBatch, event_count) == 16);
    assert!(offset_of!(NveEventBatch, bytes_len) == 20);

    assert!(size_of::<NvePropEntry>() == 32);
    assert!(align_of::<NvePropEntry>() == 8);
    assert!(offset_of!(NvePropEntry, field_id) == 0);
    assert!(offset_of!(NvePropEntry, _pad) == 2);
    assert!(offset_of!(NvePropEntry, value) == 8);

    assert!(size_of::<NvePropSlice>() == 16);
    assert!(align_of::<NvePropSlice>() == 8);
    assert!(offset_of!(NvePropSlice, entries) == 0);
    assert!(offset_of!(NvePropSlice, len) == 8);
    assert!(offset_of!(NvePropSlice, _pad) == 12);

    assert!(size_of::<NveCommandSlice>() == 16);
    assert!(align_of::<NveCommandSlice>() == 8);
    assert!(offset_of!(NveCommandSlice, opcode) == 0);
    assert!(offset_of!(NveCommandSlice, arg_count) == 2);
    assert!(offset_of!(NveCommandSlice, _pad) == 4);
    assert!(offset_of!(NveCommandSlice, args) == 8);

    assert!(size_of::<NveHostCallbacks>() == 24);

    assert!(size_of::<crate::table::NveTableView>() == 72);
    assert!(align_of::<crate::table::NveTableView>() == 8);

    assert!(size_of::<NveSceneView>() == 136);
    assert!(align_of::<NveSceneView>() == 8);
    assert!(size_of::<crate::runtime::NveRuntimeApi>() == 16 + 37 * 8);
    assert!(align_of::<crate::runtime::NveRuntimeApi>() == 8);
};
