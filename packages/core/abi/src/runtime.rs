//! Zero-copy host runtime C ABI (`nve_runtime_get_interface`, `NveRuntimeApi`,
//! `NveRuntimeSceneView`, `NveNodeRow`, `NveLayoutRect`, `NveResourceView`, `NveRuntimeConfig`,
//! `NveRuntimeMetrics`, `NveRuntimeStatus`, `NveTextMeasureFn`).
//!
//! Coexists cleanly alongside legacy `NveSceneView`/`NveRenderNode` during Track 1B-2 Slice 2A
//! until Slice 2B rebinds all hosts and deletes the legacy view.

use core::ffi::c_void;
use core::mem::{align_of, size_of};

use nve_abi_macros::NveAbi;

use crate::ffi::{NveBufferSlice, NveGetInterfaceFn};
use crate::table::NveTableView;
use crate::value::{NveEventBatch, NveStrSlice};

mod schema;

pub use schema::{runtime_abi_descriptor, NVE_RUNTIME_SCHEMA_HASH};

/// Current C ABI version expected by `nve_runtime_get_interface(uint32_t abi_version)`.
///
/// `2`: published resources (`NveResourceView`) and the host text measurer (`set_text_measurer`).
pub const NVE_RUNTIME_ABI_VERSION: u32 = 2;

/// Exact byte size of one `NveLayoutRect` record (16 bytes).
pub const NVE_LAYOUT_RECT_SIZE: usize = 16;

/// Exact byte size of one `NveNodeRow` record (32 bytes, matching `nve_store::NodeRow`).
pub const NVE_NODE_ROW_SIZE: usize = 32;

/// Flag bit in `NveNodeRow::flags` indicating an allocated live node slot (`nve_store::LIVE`).
pub const NVE_NODE_ROW_FLAG_LIVE: u8 = 1 << 0;

/// Flag bit in `NveResourceView::flags`: the resource is ephemeral (rewritten in place, e.g. a
/// 120 Hz canvas draw list), so hosts should not cache decoded forms across frames by id alone.
pub const NVE_RESOURCE_FLAG_EPHEMERAL: u32 = 1 << 0;

/// Opaque handle to a live `nve-runtime` surface instance.
pub type NveRuntimeHandle = *mut c_void;

/// Asynchronous wake callback registered by a host to be notified when frames or pairing work
/// become ready.
pub type NveRuntimeWakeFn = unsafe extern "C" fn(ctx: *mut c_void);

/// Host text measurer installed with [`NveRuntimeApi::set_text_measurer`].
///
/// Measures UTF-8 `text` at `font_size` logical points and CSS `font_weight` (`100..=900`) under
/// `max_width`: `+INFINITY` = unconstrained, `0` = min-content probe, `> 0` = wrap width. Writes
/// the size in logical points to `out_width` / `out_height` and returns `0`; any other return
/// value (or a non-finite / negative size) falls back to the runtime's heuristic measurer.
/// Called synchronously on the surface's owner thread during layout (inside `tick`,
/// `set_viewport`, `submit_nve2`, `dispatch_events`, `scroll_by`, or `set_text_measurer` itself).
pub type NveTextMeasureFn = unsafe extern "C" fn(
    ctx: *mut c_void,
    text: NveStrSlice,
    font_size: f32,
    font_weight: u32,
    max_width: f32,
    out_width: *mut f32,
    out_height: *mut f32,
) -> i32;

/// Status codes returned by [`NveRuntimeApi`] entry points.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveAbi)]
#[nve_abi(c_prefix = "NVE_RUNTIME_")]
pub enum NveRuntimeStatus {
    Ok = 0,
    InvalidArg = 1,
    Unsupported = 2,
    Busy = 3,
    Io = 4,
    Panic = 5,
    NotFound = 6,
    Error = 7,
}

impl NveRuntimeStatus {
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    #[inline]
    pub const fn from_i32(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::InvalidArg,
            2 => Self::Unsupported,
            3 => Self::Busy,
            4 => Self::Io,
            5 => Self::Panic,
            6 => Self::NotFound,
            _ => Self::Error,
        }
    }
}

/// Zero-copy 16-byte absolute layout rectangle indexed by `NodeIndex` (`1..node_capacity`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, NveAbi)]
#[nve_abi(size = 16)]
pub struct NveLayoutRect {
    #[nve_abi(offset = 0)]
    pub x: f32,
    #[nve_abi(offset = 4)]
    pub y: f32,
    #[nve_abi(offset = 8)]
    pub width: f32,
    #[nve_abi(offset = 12)]
    pub height: f32,
}

/// Zero-copy 32-byte per-node header mirroring `nve_store::NodeRow` directly across C FFI.
///
/// Uses `[u32; 4]` instead of `u128` so C11/C++20/MSVC/32-bit/Swift ClangImporter all agree on
/// exact byte offsets with zero target ABI variance.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, NveAbi)]
#[nve_abi(size = 32)]
pub struct NveNodeRow {
    /// 128-bit attached-table bitset in little-endian 32-bit words (`bit s = word[s/32] & (1<<(s%32))`).
    #[nve_abi(offset = 0)]
    pub table_set: [u32; 4],
    /// Parent `NodeId` (`0` = root / detached).
    #[nve_abi(offset = 16)]
    pub parent: u32,
    /// Element offset (in `u32` entries) into `NveRuntimeSceneView::children_slab`.
    #[nve_abi(offset = 20)]
    pub child_off: u32,
    /// Number of child `u32` `NodeId`s in `NveRuntimeSceneView::children_slab`.
    #[nve_abi(offset = 24)]
    pub child_len: u32,
    /// Slot generation counter (`1..=255`).
    #[nve_abi(offset = 28)]
    pub generation: u8,
    /// Highest-priority realizing `TableSlot` (`0xFF` = plain container).
    #[nve_abi(offset = 29)]
    pub primary_slot: u8,
    /// Union of `effect::*` bits dirtied on this node during the latest frame.
    #[nve_abi(offset = 30)]
    pub effects: u8,
    /// Node row flags (`NVE_NODE_ROW_FLAG_LIVE` = bit 0).
    #[nve_abi(offset = 31)]
    pub flags: u8,
}

impl NveNodeRow {
    #[inline]
    pub const fn is_live(&self) -> bool {
        (self.flags & NVE_NODE_ROW_FLAG_LIVE) != 0
    }

    #[inline]
    pub const fn has_table(&self, slot: u8) -> bool {
        if slot >= 128 {
            return false;
        }
        let word = (slot as usize) >> 5;
        let bit = (slot as u32) & 31;
        (self.table_set[word] & (1u32 << bit)) != 0
    }
}

/// Borrowed wire bytes of one live resource, indexed by raw `ResourceId` in
/// `NveRuntimeSceneView::resources` (index `0` = `ResourceId::NONE`, always empty).
///
/// `kind` is the resource kind `Key64` (e.g. `nve.res.image_src`, `nve.res.draw_list`); `bytes`
/// holds `len` bytes in that kind's wire encoding. Free slots have `kind == 0`, `len == 0`, and a
/// null `bytes`.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 24)]
pub struct NveResourceView {
    /// Low 32 bits of the resource kind `Key64`.
    #[nve_abi(offset = 0)]
    pub kind_lo: u32,
    /// High 32 bits of the resource kind `Key64`.
    #[nve_abi(offset = 4)]
    pub kind_hi: u32,
    /// Byte length of `bytes`.
    #[nve_abi(offset = 8)]
    pub len: u32,
    /// `NVE_RESOURCE_FLAG_*` bits.
    #[nve_abi(offset = 12)]
    pub flags: u32,
    /// Wire-encoded payload (null when `len == 0`).
    pub bytes: *const u8,
}

impl NveResourceView {
    /// The empty view of a free (or never defined) resource slot.
    pub const EMPTY: Self = Self {
        kind_lo: 0,
        kind_hi: 0,
        len: 0,
        flags: 0,
        bytes: core::ptr::null(),
    };
}

impl Default for NveResourceView {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// Configuration passed to [`NveRuntimeApi::create`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, NveAbi)]
#[nve_abi(size = 24)]
pub struct NveRuntimeConfig {
    #[nve_abi(offset = 0)]
    pub surface_id: u32,
    #[nve_abi(offset = 4)]
    pub width: f32,
    #[nve_abi(offset = 8)]
    pub height: f32,
    #[nve_abi(offset = 12)]
    pub scale_factor: f32,
    #[nve_abi(offset = 16)]
    pub flags: u32,
    #[nve_abi(offset = 20)]
    pub _reserved: u32,
}

impl Default for NveRuntimeConfig {
    fn default() -> Self {
        Self {
            surface_id: 1,
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
            flags: 0,
            _reserved: 0,
        }
    }
}

/// Live telemetry counters snapshot returned by [`NveRuntimeApi::get_metrics`].
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, NveAbi)]
#[nve_abi(size = 64)]
pub struct NveRuntimeMetrics {
    #[nve_abi(offset = 0)]
    pub frame_id: u64,
    #[nve_abi(offset = 8)]
    pub reconcile_us: u64,
    #[nve_abi(offset = 16)]
    pub layout_us: u64,
    #[nve_abi(offset = 24)]
    pub publish_us: u64,
    #[nve_abi(offset = 32)]
    pub wire_bytes_rx: u64,
    #[nve_abi(offset = 40)]
    pub wire_bytes_tx: u64,
    #[nve_abi(offset = 48)]
    pub live_nodes: u32,
    #[nve_abi(offset = 52)]
    pub dirty_nodes: u32,
    #[nve_abi(offset = 56)]
    pub pair_connected: u32,
    #[nve_abi(offset = 60)]
    pub _reserved: u32,
}

/// Zero-copy borrowed runtime scene view guarded by `scene_acquire` / `scene_release`.
///
/// Exposes `NodeStore`'s dense `NveNodeRow` array, dense `NveLayoutRect` array, `text_slab`,
/// `children_slab`, per-frame `dirty_indices` / `removed_ids`, published `NveTableView`s, and
/// every live resource's wire bytes (`resources`, plus the ids defined this frame in
/// `dirty_resources`) with zero intermediate `NveRenderNode` materialization.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
pub struct NveRuntimeSceneView {
    /// Must equal [`NVE_RUNTIME_SCHEMA_HASH`].
    #[nve_abi(offset = 0)]
    pub schema_hash: u64,
    /// Monotonic frame counter.
    #[nve_abi(offset = 8)]
    pub frame_id: u64,
    /// Slot capacity of `rows` and `rects` (slot 0 is reserved null; live indices are `1..node_capacity`).
    #[nve_abi(offset = 16)]
    pub node_capacity: u32,
    /// Count of modified node slot indices in `dirty_indices`.
    #[nve_abi(offset = 20)]
    pub dirty_count: u32,
    /// Count of removed `NodeId` values in `removed_ids`.
    #[nve_abi(offset = 24)]
    pub removed_count: u32,
    /// Byte length of UTF-8 `text_slab`.
    #[nve_abi(offset = 28)]
    pub text_slab_len: u32,
    /// Element count (`u32` entries) of `children_slab`.
    #[nve_abi(offset = 32)]
    pub children_slab_len: u32,
    /// Number of published [`NveTableView`] descriptors in `tables`.
    #[nve_abi(offset = 36)]
    pub table_count: u32,
    /// Entry count of `resources` (every live `ResourceId` is below it).
    #[nve_abi(offset = 40)]
    pub resource_capacity: u32,
    /// Count of resource ids (re)defined on `frame_id` in `dirty_resources`.
    #[nve_abi(offset = 44)]
    pub dirty_resource_count: u32,
    /// Borrowed slice of `node_capacity` [`NveNodeRow`] headers directly from `NodeStore`.
    pub rows: *const NveNodeRow,
    /// Borrowed slice of `node_capacity` [`NveLayoutRect`] boxes indexed by `NodeIndex`.
    pub rects: *const NveLayoutRect,
    /// Borrowed contiguous UTF-8 text byte slab directly from `NodeStore`.
    pub text_slab: *const u8,
    /// Borrowed contiguous `u32` child `NodeId` slab directly from `NodeStore`.
    pub children_slab: *const u32,
    /// Borrowed slice of `dirty_count` node indices modified on `frame_id`.
    pub dirty_indices: *const u32,
    /// Borrowed slice of `removed_count` raw `NodeId`s removed on `frame_id`.
    pub removed_ids: *const u32,
    /// Borrowed slice of `table_count` [`NveTableView`] table views.
    pub tables: *const NveTableView,
    /// Borrowed slice of `resource_capacity` [`NveResourceView`]s indexed by raw `ResourceId`.
    pub resources: *const NveResourceView,
    /// Borrowed slice of `dirty_resource_count` unique raw `ResourceId`s (re)defined on `frame_id`.
    pub dirty_resources: *const u32,
}

impl Default for NveRuntimeSceneView {
    fn default() -> Self {
        Self {
            schema_hash: NVE_RUNTIME_SCHEMA_HASH,
            frame_id: 0,
            node_capacity: 0,
            dirty_count: 0,
            removed_count: 0,
            text_slab_len: 0,
            children_slab_len: 0,
            table_count: 0,
            resource_capacity: 0,
            dirty_resource_count: 0,
            rows: core::ptr::null(),
            rects: core::ptr::null(),
            text_slab: core::ptr::null(),
            children_slab: core::ptr::null(),
            dirty_indices: core::ptr::null(),
            removed_ids: core::ptr::null(),
            tables: core::ptr::null(),
            resources: core::ptr::null(),
            dirty_resources: core::ptr::null(),
        }
    }
}

/// Single exported C ABI vtable returned by `nve_runtime_get_interface(uint32_t abi_version)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NveRuntimeApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub schema_hash: u64,
    pub create: unsafe extern "C" fn(config: *const NveRuntimeConfig) -> NveRuntimeHandle,
    pub destroy: unsafe extern "C" fn(handle: NveRuntimeHandle),
    pub set_wake_callback: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        cb: Option<NveRuntimeWakeFn>,
        ctx: *mut c_void,
    ) -> i32,
    pub set_viewport: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        width: f32,
        height: f32,
        scale_factor: f32,
    ) -> i32,
    pub attach_app_interface: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        get_iface: Option<NveGetInterfaceFn>,
    ) -> i32,
    pub submit_nve2:
        unsafe extern "C" fn(handle: NveRuntimeHandle, frame: NveBufferSlice) -> i32,
    pub tick:
        unsafe extern "C" fn(handle: NveRuntimeHandle, dt_s: f32, out_changed: *mut u32) -> i32,
    pub dispatch_events: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        batch: NveEventBatch,
        out_changed: *mut u32,
    ) -> i32,
    pub hit_test: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        x: f32,
        y: f32,
        out_node_id: *mut u32,
    ) -> i32,
    pub scroll_by: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        node_id: u32,
        dx: f32,
        dy: f32,
        out_x: *mut f32,
        out_y: *mut f32,
    ) -> i32,
    pub scene_acquire:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_view: *mut NveRuntimeSceneView) -> i32,
    pub scene_release: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    pub get_metrics:
        unsafe extern "C" fn(handle: NveRuntimeHandle, out_metrics: *mut NveRuntimeMetrics) -> i32,
    pub pair_connect: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        uri: NveStrSlice,
        token: NveStrSlice,
    ) -> i32,
    pub pair_disconnect: unsafe extern "C" fn(handle: NveRuntimeHandle) -> i32,
    pub pair_is_connected: unsafe extern "C" fn(handle: NveRuntimeHandle) -> u32,
    pub last_error: unsafe extern "C" fn(out_buf: *mut u8, cap: usize) -> usize,
    /// Installs (`Some`) or removes (`None`, restoring the heuristic) the host text measurer and
    /// relayouts every node. `ctx` must stay valid until replaced, removed, or `destroy` returns.
    pub set_text_measurer: unsafe extern "C" fn(
        handle: NveRuntimeHandle,
        measure: Option<NveTextMeasureFn>,
        ctx: *mut c_void,
    ) -> i32,
}

/// Entry-point signature exported by `libnve_runtime` (`nve_runtime_get_interface`).
pub type NveRuntimeGetInterfaceFn = unsafe extern "C" fn(abi_version: u32) -> *const NveRuntimeApi;


const _: () = {
    assert!(size_of::<NveLayoutRect>() == NVE_LAYOUT_RECT_SIZE);
    assert!(align_of::<NveLayoutRect>() == 4);
    assert!(size_of::<NveNodeRow>() == NVE_NODE_ROW_SIZE);
    assert!(align_of::<NveNodeRow>() == 4);
    assert!(size_of::<NveRuntimeConfig>() == 24);
    assert!(align_of::<NveRuntimeConfig>() == 4);
    assert!(size_of::<NveRuntimeMetrics>() == 64);
    assert!(align_of::<NveRuntimeMetrics>() == 8);
};
