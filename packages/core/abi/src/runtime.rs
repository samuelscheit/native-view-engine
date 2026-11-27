//! Zero-copy host runtime C ABI (`nve_runtime_get_interface`, `NveRuntimeApi`,
//! `NveRuntimeSceneView`, `NveNodeRow`, `NveLayoutRect`, `NveRuntimeConfig`,
//! `NveRuntimeMetrics`, `NveRuntimeStatus`).
//!
//! Coexists cleanly alongside legacy `NveSceneView`/`NveRenderNode` during Track 1B-2 Slice 2A
//! until Slice 2B rebinds all hosts and deletes the legacy view.

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

use nve_abi_macros::NveAbi;
use nve_hash::Xxh64Hasher;

use crate::ffi::{NveBufferSlice, NveGetInterfaceFn};
use crate::schema::{
    AbiConstDescriptor, AbiEnumDescriptor, AbiFileDescriptor, AbiRecordDescriptor, NveEnumSchema,
    NveStructSchema,
};
use crate::table::NveTableView;
use crate::value::{NveEventBatch, NveStrSlice};

/// Current C ABI version expected by `nve_runtime_get_interface(uint32_t abi_version)`.
pub const NVE_RUNTIME_ABI_VERSION: u32 = 1;

/// Exact byte size of one `NveLayoutRect` record (16 bytes).
pub const NVE_LAYOUT_RECT_SIZE: usize = 16;

/// Exact byte size of one `NveNodeRow` record (32 bytes, matching `nve_store::NodeRow`).
pub const NVE_NODE_ROW_SIZE: usize = 32;

/// Flag bit in `NveNodeRow::flags` indicating an allocated live node slot (`nve_store::LIVE`).
pub const NVE_NODE_ROW_FLAG_LIVE: u8 = 1 << 0;

/// Opaque handle to a live `nve-runtime` surface instance.
pub type NveRuntimeHandle = *mut c_void;

/// Asynchronous wake callback registered by a host to be notified when frames or pairing work
/// become ready.
pub type NveRuntimeWakeFn = unsafe extern "C" fn(ctx: *mut c_void);

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
/// `children_slab`, per-frame `dirty_indices` / `removed_ids`, and published `NveTableView`s
/// with zero intermediate `NveRenderNode` materialization.
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
            rows: core::ptr::null(),
            rects: core::ptr::null(),
            text_slab: core::ptr::null(),
            children_slab: core::ptr::null(),
            dirty_indices: core::ptr::null(),
            removed_ids: core::ptr::null(),
            tables: core::ptr::null(),
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
}

/// Entry-point signature exported by `libnve_runtime` (`nve_runtime_get_interface`).
pub type NveRuntimeGetInterfaceFn = unsafe extern "C" fn(abi_version: u32) -> *const NveRuntimeApi;

const fn hash_rt_field(h: &mut Xxh64Hasher, name: &str, ty: &str, sep: &[u8], v: usize) {
    h.update(b"|");
    h.update(name.as_bytes());
    h.update(b":");
    h.update(ty.as_bytes());
    h.update(sep);
    h.update_u64(v as u64);
}

const fn compute_runtime_schema_hash() -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(b"nve.runtime_abi.v1");
    h.update_u64(NVE_RUNTIME_ABI_VERSION as u64);

    h.update(b";NveLayoutRect");
    h.update_u64(size_of::<NveLayoutRect>() as u64);
    hash_rt_field(&mut h, "x", "f32", b"@", offset_of!(NveLayoutRect, x));
    hash_rt_field(&mut h, "y", "f32", b"@", offset_of!(NveLayoutRect, y));
    hash_rt_field(&mut h, "width", "f32", b"@", offset_of!(NveLayoutRect, width));
    hash_rt_field(&mut h, "height", "f32", b"@", offset_of!(NveLayoutRect, height));

    h.update(b";NveNodeRow");
    h.update_u64(size_of::<NveNodeRow>() as u64);
    hash_rt_field(&mut h, "table_set", "[u32;4]", b"@", offset_of!(NveNodeRow, table_set));
    hash_rt_field(&mut h, "parent", "u32", b"@", offset_of!(NveNodeRow, parent));
    hash_rt_field(&mut h, "child_off", "u32", b"@", offset_of!(NveNodeRow, child_off));
    hash_rt_field(&mut h, "child_len", "u32", b"@", offset_of!(NveNodeRow, child_len));
    hash_rt_field(&mut h, "generation", "u8", b"@", offset_of!(NveNodeRow, generation));
    hash_rt_field(&mut h, "primary_slot", "u8", b"@", offset_of!(NveNodeRow, primary_slot));
    hash_rt_field(&mut h, "effects", "u8", b"@", offset_of!(NveNodeRow, effects));
    hash_rt_field(&mut h, "flags", "u8", b"@", offset_of!(NveNodeRow, flags));

    h.update(b";NveRuntimeSceneView");
    hash_rt_field(&mut h, "schema_hash", "u64", b"@", offset_of!(NveRuntimeSceneView, schema_hash));
    hash_rt_field(&mut h, "frame_id", "u64", b"@", offset_of!(NveRuntimeSceneView, frame_id));
    hash_rt_field(&mut h, "node_capacity", "u32", b"@", offset_of!(NveRuntimeSceneView, node_capacity));
    hash_rt_field(&mut h, "dirty_count", "u32", b"@", offset_of!(NveRuntimeSceneView, dirty_count));
    hash_rt_field(&mut h, "removed_count", "u32", b"@", offset_of!(NveRuntimeSceneView, removed_count));
    hash_rt_field(&mut h, "text_slab_len", "u32", b"@", offset_of!(NveRuntimeSceneView, text_slab_len));
    hash_rt_field(&mut h, "children_slab_len", "u32", b"@", offset_of!(NveRuntimeSceneView, children_slab_len));
    hash_rt_field(&mut h, "table_count", "u32", b"@", offset_of!(NveRuntimeSceneView, table_count));
    hash_rt_field(&mut h, "rows", "*const NveNodeRow", b"#", 8);
    hash_rt_field(&mut h, "rects", "*const NveLayoutRect", b"#", 9);
    hash_rt_field(&mut h, "text_slab", "*const u8", b"#", 10);
    hash_rt_field(&mut h, "children_slab", "*const u32", b"#", 11);
    hash_rt_field(&mut h, "dirty_indices", "*const u32", b"#", 12);
    hash_rt_field(&mut h, "removed_ids", "*const u32", b"#", 13);
    hash_rt_field(&mut h, "tables", "*const NveTableView", b"#", 14);

    h.update(b";NveRuntimeConfig");
    h.update_u64(size_of::<NveRuntimeConfig>() as u64);
    h.update(b";NveRuntimeMetrics");
    h.update_u64(size_of::<NveRuntimeMetrics>() as u64);
    h.update(b";NveRuntimeApi:17");
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
];

const RUNTIME_ENUMS: &[AbiEnumDescriptor] = &[<NveRuntimeStatus as NveEnumSchema>::SCHEMA];

const RUNTIME_RECORDS: &[AbiRecordDescriptor] = &[
    AbiRecordDescriptor::Struct(<NveLayoutRect as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveNodeRow as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeConfig as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveRuntimeMetrics as NveStructSchema>::SCHEMA),
    AbiRecordDescriptor::Struct(<NveTableView as NveStructSchema>::SCHEMA),
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
