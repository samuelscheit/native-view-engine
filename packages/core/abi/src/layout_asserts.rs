//! Unconditional compile-time ABI layout verification for every `#[repr(C)]` vtable,
//! POD struct, union, and FFI enum in `nve-abi`.
//!
//! Mirrors the C11/C++20 `NVE_STATIC_ASSERT` checks emitted into `nve_abi.generated.h`
//! and `nve_scene_abi.generated.h` so any Rust/C layout divergence fails compilation
//! immediately across both 32-bit and 64-bit targets.

use core::mem::{align_of, offset_of, size_of};

use crate::ffi::{
    NativeEventKind, NveBufferSlice, NveComponentApi, NveDevApi, NveFrameStats,
    NveGestureDisposition, NveIntrinsicSize, NveLayoutApi, NveSizeConstraints, NveSurfaceApi,
    NveSurfaceConfig,
};
use crate::host::{NveHostApi, NveHostCallbacks, NveHostStatus};
use crate::runtime::{
    NveLayoutRect, NveNodeRow, NveRuntimeApi, NveRuntimeConfig, NveRuntimeMetrics,
    NveRuntimeSceneView, NveRuntimeStatus, NVE_LAYOUT_RECT_SIZE, NVE_NODE_ROW_SIZE,
};
use crate::scene::{
    NveNodeMeta, NveRenderNode, NveSceneView, NVE_NODE_META_ALIGN, NVE_NODE_META_SIZE,
    NVE_RENDER_NODE_ALIGN, NVE_RENDER_NODE_SIZE,
};
use crate::value::{
    NveCommandSlice, NveDirectEvent, NveEventBatch, NveMapEntry, NvePropEntry, NvePropSlice,
    NveStrSlice, NveValue, NveValueArray, NveValueData, NveValueMap, NveValueTag,
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
    assert!(size_of::<NativeEventKind>() == 1);
    assert!(align_of::<NativeEventKind>() == 1);
    assert!(size_of::<NveHostStatus>() == 4);
    assert!(align_of::<NveHostStatus>() == 4);
    assert!(size_of::<NveValueTag>() == 4);
    assert!(align_of::<NveValueTag>() == 4);

    // ---- Core POD structs (ffi.rs) ----
    assert!(size_of::<NveBufferSlice>() == 2 * PTR_SIZE);
    assert!(align_of::<NveBufferSlice>() == PTR_ALIGN);
    assert!(offset_of!(NveBufferSlice, ptr) == 0);
    assert!(offset_of!(NveBufferSlice, len) == PTR_SIZE);

    assert!(size_of::<NveSurfaceConfig>() == 16);
    assert!(align_of::<NveSurfaceConfig>() == 4);
    assert!(offset_of!(NveSurfaceConfig, surface_id) == 0);
    assert!(offset_of!(NveSurfaceConfig, width) == 4);
    assert!(offset_of!(NveSurfaceConfig, height) == 8);
    assert!(offset_of!(NveSurfaceConfig, scale_factor) == 12);

    assert!(size_of::<NveFrameStats>() == 40);
    assert!(align_of::<NveFrameStats>() == align_of::<u64>());
    assert!(offset_of!(NveFrameStats, op_count) == 0);
    assert!(offset_of!(NveFrameStats, mutation_count) == 4);
    assert!(offset_of!(NveFrameStats, decode_us) == 8);
    assert!(offset_of!(NveFrameStats, layout_us) == 16);
    assert!(offset_of!(NveFrameStats, diff_us) == 24);
    assert!(offset_of!(NveFrameStats, mount_us) == 32);

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

    // ---- Core modular interface vtables (ffi.rs) ----
    assert!(size_of::<NveSurfaceApi>() == 4 * PTR_SIZE);
    assert!(align_of::<NveSurfaceApi>() == PTR_ALIGN);
    assert!(offset_of!(NveSurfaceApi, create) == 0);
    assert!(offset_of!(NveSurfaceApi, destroy) == PTR_SIZE);
    assert!(offset_of!(NveSurfaceApi, render) == 2 * PTR_SIZE);
    assert!(offset_of!(NveSurfaceApi, dispatch_events) == 3 * PTR_SIZE);

    assert!(size_of::<NveLayoutApi>() == 2 * PTR_SIZE);
    assert!(align_of::<NveLayoutApi>() == PTR_ALIGN);
    assert!(offset_of!(NveLayoutApi, measure) == 0);
    assert!(offset_of!(NveLayoutApi, hit_test) == PTR_SIZE);

    assert!(size_of::<NveComponentApi>() == 2 * PTR_SIZE);
    assert!(align_of::<NveComponentApi>() == PTR_ALIGN);
    assert!(offset_of!(NveComponentApi, set_props) == 0);
    assert!(offset_of!(NveComponentApi, drain_emitted_events) == PTR_SIZE);

    assert!(size_of::<NveDevApi>() == 3 * PTR_SIZE);
    assert!(align_of::<NveDevApi>() == PTR_ALIGN);
    assert!(offset_of!(NveDevApi, apply_hot_patch) == 0);
    assert!(offset_of!(NveDevApi, snapshot_state) == PTR_SIZE);
    assert!(offset_of!(NveDevApi, restore_state) == 2 * PTR_SIZE);

    // ---- Host capability structs & vtable (host/mod.rs) ----
    assert!(size_of::<NveHostCallbacks>() == 3 * PTR_SIZE);
    assert!(align_of::<NveHostCallbacks>() == PTR_ALIGN);
    assert!(offset_of!(NveHostCallbacks, host_ctx) == 0);
    assert!(offset_of!(NveHostCallbacks, invoke) == PTR_SIZE);
    assert!(offset_of!(NveHostCallbacks, channel_publish) == 2 * PTR_SIZE);

    assert!(size_of::<NveHostApi>() == 2 * PTR_SIZE);
    assert!(align_of::<NveHostApi>() == PTR_ALIGN);
    assert!(offset_of!(NveHostApi, bind_host) == 0);
    assert!(offset_of!(NveHostApi, dispatch_host_events) == PTR_SIZE);

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

    assert!(offset_of!(NveDirectEvent, surface_id) == 0);
    assert!(offset_of!(NveDirectEvent, node_id) == 4);
    assert!(offset_of!(NveDirectEvent, timestamp_us) == 8);
    assert!(offset_of!(NveDirectEvent, kind) == 16);
    assert!(offset_of!(NveDirectEvent, payload_tag) == 17);
    assert!(offset_of!(NveDirectEvent, modifiers) == 18);
    assert!(offset_of!(NveDirectEvent, event_flags) == 19);
    assert!(offset_of!(NveDirectEvent, key_code) == 20);
    assert!(offset_of!(NveDirectEvent, name) == 24);
    assert!(offset_of!(NveDirectEvent, x) == offset_of!(NveDirectEvent, name) + size_of::<NveStrSlice>());
    assert!(offset_of!(NveDirectEvent, y) == offset_of!(NveDirectEvent, x) + 4);
    assert!(offset_of!(NveDirectEvent, width) == offset_of!(NveDirectEvent, y) + 4);
    assert!(offset_of!(NveDirectEvent, height) == offset_of!(NveDirectEvent, width) + 4);
    assert!(
        offset_of!(NveDirectEvent, text)
            == align_up(offset_of!(NveDirectEvent, height) + 4, align_of::<NveStrSlice>())
    );
    assert!(
        offset_of!(NveDirectEvent, value)
            == align_up(
                offset_of!(NveDirectEvent, text) + size_of::<NveStrSlice>(),
                align_of::<NveValue>()
            )
    );
    assert!(size_of::<NveDirectEvent>() == if PTR_SIZE == 8 { 96 } else { 72 });
    assert!(align_of::<NveDirectEvent>() == 8);

    assert!(offset_of!(NveEventBatch, ptr) == 0);
    assert!(offset_of!(NveEventBatch, len) == PTR_SIZE);
    assert!(offset_of!(NveEventBatch, _pad) == PTR_SIZE + 4);
    assert!(align_of::<NveEventBatch>() == PTR_ALIGN);
    assert!(size_of::<NveEventBatch>() == PTR_SIZE + 8);

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

    // ---- Scene ABI types (scene/types.rs) ----
    assert!(size_of::<NveRenderNode>() == NVE_RENDER_NODE_SIZE);
    assert!(align_of::<NveRenderNode>() == NVE_RENDER_NODE_ALIGN);
    assert!(offset_of!(NveRenderNode, node_id) == 0);
    assert!(offset_of!(NveRenderNode, primary_slot) == 4);
    assert!(offset_of!(NveRenderNode, flags) == 5);
    assert!(offset_of!(NveRenderNode, opacity_u8) == 6);
    assert!(offset_of!(NveRenderNode, _pad0) == 7);
    assert!(offset_of!(NveRenderNode, x) == 8);
    assert!(offset_of!(NveRenderNode, y) == 12);
    assert!(offset_of!(NveRenderNode, width) == 16);
    assert!(offset_of!(NveRenderNode, height) == 20);
    assert!(offset_of!(NveRenderNode, bg_rgba) == 24);
    assert!(offset_of!(NveRenderNode, text_rgba) == 28);
    assert!(offset_of!(NveRenderNode, border_rgba) == 32);
    assert!(offset_of!(NveRenderNode, border_radius) == 36);
    assert!(offset_of!(NveRenderNode, border_width) == 40);
    assert!(offset_of!(NveRenderNode, font_size) == 44);
    assert!(offset_of!(NveRenderNode, text_offset) == 48);
    assert!(offset_of!(NveRenderNode, text_len) == 52);
    assert!(offset_of!(NveRenderNode, child_count) == 54);
    assert!(offset_of!(NveRenderNode, child_offset) == 56);
    assert!(offset_of!(NveRenderNode, z_index) == 60);

    assert!(offset_of!(NveSceneView, schema_hash) == 0);
    assert!(offset_of!(NveSceneView, frame_id) == 8);
    assert!(offset_of!(NveSceneView, node_count) == 16);
    assert!(offset_of!(NveSceneView, dirty_count) == 20);
    assert!(offset_of!(NveSceneView, removed_count) == 24);
    assert!(offset_of!(NveSceneView, text_slab_len) == 28);
    assert!(offset_of!(NveSceneView, children_slab_len) == 32);
    assert!(offset_of!(NveSceneView, table_count) == 36);
    assert!(offset_of!(NveSceneView, nodes) == 40);
    assert!(offset_of!(NveSceneView, text_slab) == 40 + PTR_SIZE);
    assert!(offset_of!(NveSceneView, children_slab) == 40 + 2 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, dirty_indices) == 40 + 3 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, removed_ids) == 40 + 4 * PTR_SIZE);
    assert!(offset_of!(NveSceneView, tables) == 40 + 5 * PTR_SIZE);
    assert!(
        size_of::<NveSceneView>() == align_up(40 + 6 * PTR_SIZE, align_of::<NveSceneView>())
    );

    assert!(offset_of!(crate::table::NveTableView, key_lo) == 0);
    assert!(offset_of!(crate::table::NveTableView, key_hi) == 4);
    assert!(offset_of!(crate::table::NveTableView, slot) == 8);
    assert!(offset_of!(crate::table::NveTableView, words) == 12);
    assert!(offset_of!(crate::table::NveTableView, row_count) == 16);
    assert!(offset_of!(crate::table::NveTableView, dirty_count) == 20);
    assert!(offset_of!(crate::table::NveTableView, rows) == 24);
    assert!(offset_of!(crate::table::NveTableView, owners) == 24 + PTR_SIZE);
    assert!(offset_of!(crate::table::NveTableView, dirty_rows) == 24 + 2 * PTR_SIZE);
    assert!(
        size_of::<crate::table::NveTableView>()
            == align_up(24 + 3 * PTR_SIZE, align_of::<crate::table::NveTableView>())
    );

    assert!(size_of::<NveNodeMeta>() == NVE_NODE_META_SIZE);
    assert!(align_of::<NveNodeMeta>() == NVE_NODE_META_ALIGN);
    assert!(offset_of!(NveNodeMeta, node_id) == 0);
    assert!(offset_of!(NveNodeMeta, flags) == 4);
    assert!(offset_of!(NveNodeMeta, label_offset) == 8);
    assert!(offset_of!(NveNodeMeta, label_len) == 12);
    assert!(offset_of!(NveNodeMeta, role_len) == 14);
    assert!(offset_of!(NveNodeMeta, role_offset) == 16);
    assert!(offset_of!(NveNodeMeta, hint_offset) == 20);
    assert!(offset_of!(NveNodeMeta, hint_len) == 24);
    assert!(offset_of!(NveNodeMeta, state_desc_len) == 26);
    assert!(offset_of!(NveNodeMeta, state_desc_offset) == 28);
    assert!(offset_of!(NveNodeMeta, placeholder_offset) == 32);
    assert!(offset_of!(NveNodeMeta, placeholder_len) == 36);
    assert!(offset_of!(NveNodeMeta, test_id_len) == 38);
    assert!(offset_of!(NveNodeMeta, test_id_offset) == 40);
    assert!(offset_of!(NveNodeMeta, _reserved) == 44);

    // ---- Zero-copy runtime ABI types (runtime.rs) ----
    assert!(size_of::<NveRuntimeStatus>() == 4);
    assert!(align_of::<NveRuntimeStatus>() == 4);

    assert!(size_of::<NveLayoutRect>() == NVE_LAYOUT_RECT_SIZE);
    assert!(align_of::<NveLayoutRect>() == 4);
    assert!(offset_of!(NveLayoutRect, x) == 0);
    assert!(offset_of!(NveLayoutRect, y) == 4);
    assert!(offset_of!(NveLayoutRect, width) == 8);
    assert!(offset_of!(NveLayoutRect, height) == 12);

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
    assert!(align_of::<NveRuntimeMetrics>() == 8);
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

    assert!(offset_of!(NveRuntimeSceneView, schema_hash) == 0);
    assert!(offset_of!(NveRuntimeSceneView, frame_id) == 8);
    assert!(offset_of!(NveRuntimeSceneView, node_capacity) == 16);
    assert!(offset_of!(NveRuntimeSceneView, dirty_count) == 20);
    assert!(offset_of!(NveRuntimeSceneView, removed_count) == 24);
    assert!(offset_of!(NveRuntimeSceneView, text_slab_len) == 28);
    assert!(offset_of!(NveRuntimeSceneView, children_slab_len) == 32);
    assert!(offset_of!(NveRuntimeSceneView, table_count) == 36);
    assert!(offset_of!(NveRuntimeSceneView, rows) == 40);
    assert!(offset_of!(NveRuntimeSceneView, rects) == 40 + PTR_SIZE);
    assert!(offset_of!(NveRuntimeSceneView, text_slab) == 40 + 2 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeSceneView, children_slab) == 40 + 3 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeSceneView, dirty_indices) == 40 + 4 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeSceneView, removed_ids) == 40 + 5 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeSceneView, tables) == 40 + 6 * PTR_SIZE);
    assert!(
        size_of::<NveRuntimeSceneView>()
            == align_up(40 + 7 * PTR_SIZE, align_of::<NveRuntimeSceneView>())
    );

    assert!(offset_of!(NveRuntimeApi, abi_version) == 0);
    assert!(offset_of!(NveRuntimeApi, struct_size) == 4);
    assert!(offset_of!(NveRuntimeApi, schema_hash) == 8);
    assert!(offset_of!(NveRuntimeApi, create) == 16);
    assert!(offset_of!(NveRuntimeApi, destroy) == 16 + PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, set_wake_callback) == 16 + 2 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, set_viewport) == 16 + 3 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, attach_app_interface) == 16 + 4 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, submit_nve2) == 16 + 5 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, tick) == 16 + 6 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, dispatch_events) == 16 + 7 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, hit_test) == 16 + 8 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, scroll_by) == 16 + 9 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, scene_acquire) == 16 + 10 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, scene_release) == 16 + 11 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, get_metrics) == 16 + 12 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, pair_connect) == 16 + 13 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, pair_disconnect) == 16 + 14 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, pair_is_connected) == 16 + 15 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, set_props) == 16 + 16 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, drain_emitted_events) == 16 + 17 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, measure) == 16 + 18 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, apply_hot_patch) == 16 + 19 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, snapshot_state) == 16 + 20 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, restore_state) == 16 + 21 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, set_text_measurer) == 16 + 22 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, resource_bytes) == 16 + 23 * PTR_SIZE);
    assert!(offset_of!(NveRuntimeApi, last_error) == 16 + 24 * PTR_SIZE);
    assert!(
        size_of::<NveRuntimeApi>() == align_up(16 + 25 * PTR_SIZE, align_of::<NveRuntimeApi>())
    );
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

    assert!(size_of::<NveDirectEvent>() == 96);
    assert!(align_of::<NveDirectEvent>() == 8);
    assert!(offset_of!(NveDirectEvent, name) == 24);
    assert!(offset_of!(NveDirectEvent, x) == 40);
    assert!(offset_of!(NveDirectEvent, y) == 44);
    assert!(offset_of!(NveDirectEvent, width) == 48);
    assert!(offset_of!(NveDirectEvent, height) == 52);
    assert!(offset_of!(NveDirectEvent, text) == 56);
    assert!(offset_of!(NveDirectEvent, value) == 72);

    assert!(size_of::<NveEventBatch>() == 16);
    assert!(align_of::<NveEventBatch>() == 8);
    assert!(offset_of!(NveEventBatch, len) == 8);
    assert!(offset_of!(NveEventBatch, _pad) == 12);

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

    assert!(size_of::<NveSurfaceApi>() == 32);
    assert!(size_of::<NveLayoutApi>() == 16);
    assert!(size_of::<NveComponentApi>() == 16);
    assert!(size_of::<NveDevApi>() == 24);
    assert!(size_of::<NveHostCallbacks>() == 24);
    assert!(size_of::<NveHostApi>() == 16);

    assert!(size_of::<NveSceneView>() == 88);
    assert!(align_of::<NveSceneView>() == 8);
    assert!(offset_of!(NveSceneView, nodes) == 40);
    assert!(offset_of!(NveSceneView, text_slab) == 48);
    assert!(offset_of!(NveSceneView, children_slab) == 56);
    assert!(offset_of!(NveSceneView, dirty_indices) == 64);
    assert!(offset_of!(NveSceneView, removed_ids) == 72);
    assert!(offset_of!(NveSceneView, tables) == 80);

    assert!(size_of::<crate::table::NveTableView>() == 48);
    assert!(align_of::<crate::table::NveTableView>() == 8);

    assert!(size_of::<NveRuntimeSceneView>() == 96);
    assert!(align_of::<NveRuntimeSceneView>() == 8);
    assert!(size_of::<NveRuntimeApi>() == 216);
    assert!(align_of::<NveRuntimeApi>() == 8);
};
