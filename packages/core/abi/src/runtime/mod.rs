//! Zero-copy host runtime C ABI (`nve_runtime_get_interface`, `NveRuntimeApi`,
//! `NveSceneView`, `NveNodeRow`, `NveLayoutRect`, `NveRuntimeConfig`,
//! `NveRuntimeMetrics`, `NveRuntimeStatus`): the one scene ABI every host reads.

mod vtable;

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

use nve_abi_macros::NveAbi;

use crate::schema::{
    concat_abi_slices, hash_abi_descriptor_into, hash_tag_str, AbiCallbackDescriptor,
    AbiEnumDescriptor, AbiFileDescriptor, AbiRecordDescriptor, AbiVtableDescriptor, NveEnumSchema,
    NveStructSchema, NveVtableSchema,
};
use crate::table::NveTableView;
use crate::value::NveStrSlice;

pub use vtable::{NveRuntimeApi, NveRuntimeGetInterfaceFn};

nve_abi_macros::abi_consts! {
    const RUNTIME_CONSTS;

    /// Current C ABI version expected by `nve_runtime_get_interface(uint32_t abi_version)`.
    pub const NVE_RUNTIME_ABI_VERSION: u32 = 6;
    /// Exact byte size of one `NveLayoutRect` record (16 bytes).
    pub const NVE_LAYOUT_RECT_SIZE: usize = 16;
    /// Exact byte size of one `NveNodeRow` record (32 bytes, matching `nve_store::NodeRow`).
    pub const NVE_NODE_ROW_SIZE: usize = 32;
    /// Flag bit in `NveNodeRow::flags` indicating an allocated live node slot (`nve_store::LIVE`).
    pub const NVE_NODE_ROW_FLAG_LIVE: u8 = 1 << 0;
    /// `NveRuntimeTextStyle::line_height_unit`: absolute line height in points.
    pub const NVE_LINE_HEIGHT_POINTS: u8 = 0;
    /// `NveRuntimeTextStyle::line_height_unit`: multiplier × 100 of `font_size`.
    pub const NVE_LINE_HEIGHT_PERCENT: u8 = 1;
    /// `NveRuntimeTextStyle::line_height_unit`: `normal` (the font's own line height).
    pub const NVE_LINE_HEIGHT_AUTO: u8 = 2;
    /// Flag bit in `measure_surface`: pure sizing pass; never mutates committed viewport/rects or publishes.
    pub const NVE_MEASURE_FLAG_DRY_RUN: u32 = 1 << 0;
    /// Flag bit in `measure_surface`: intrinsic/unconstrained sizing pass.
    pub const NVE_MEASURE_FLAG_IDEAL: u32 = 1 << 1;
}

nve_abi_macros::abi_aliases! {
    const RUNTIME_ALIASES;

    /// Opaque handle to a live `nve-runtime` surface instance.
    pub type NveRuntimeHandle = *mut c_void;
}

nve_abi_macros::abi_callbacks! {
    const RUNTIME_HOOK_CALLBACKS;

    /// Wake callback; may fire on any thread.
    pub type NveRuntimeWakeFn = unsafe extern "C" fn(ctx: *mut c_void);

    /// Native text measurer; owner thread, during layout; measures `text` with the node's resolved
    /// typography (`style`, borrowed for the call); `max_width` is INFINITY when unconstrained.
    pub type NveRuntimeMeasureTextFn = unsafe extern "C" fn(
        ctx: *mut c_void,
        text: NveStrSlice,
        style: *const NveRuntimeTextStyle,
        max_width: f32,
        out_width: *mut f32,
        out_height: *mut f32,
    );

    /// Intrinsic-size-invalidated callback; owner thread, invoked outside internal locks after
    /// layout-affecting commits.
    pub type NveRuntimeIntrinsicSizeInvalidatedFn = unsafe extern "C" fn(ctx: *mut c_void);
}

/// Fully resolved (inherited) typography of one text node, handed to the host text measurer so
/// it measures exactly what it paints. Enum fields carry the `nve.typography` values
/// (`NVE_FONT_STYLE_*`, `NVE_WHITE_SPACE_*`, `NVE_WORD_BREAK_*`, `NVE_OVERFLOW_WRAP_*`).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
pub struct NveRuntimeTextStyle {
    /// Logical points, finite and `> 0`.
    #[nve_abi(offset = 0)]
    pub font_size: f32,
    /// Extra advance after every character, logical points (`0` = none).
    #[nve_abi(offset = 4)]
    pub letter_spacing: f32,
    /// Meaning per `line_height_unit`: points (absolute line height), percent (multiplier × 100
    /// of `font_size`, i.e. CSS unit-less `line-height`), or auto (`normal`, value `0`).
    #[nve_abi(offset = 8)]
    pub line_height: f32,
    /// Maximum painted lines, ellipsized after the last (`0` = unlimited).
    #[nve_abi(offset = 12)]
    pub max_lines: u32,
    /// CSS weight, `100..=900`.
    #[nve_abi(offset = 16)]
    pub font_weight: u16,
    /// `NveLengthUnit` of `line_height`: `0` points, `1` percent, `2` auto.
    #[nve_abi(offset = 18)]
    pub line_height_unit: u8,
    #[nve_abi(offset = 19)]
    pub font_style: u8,
    /// `NoWrap` / `Pre` never wrap at `max_width`; `Pre*` / `BreakSpaces` keep spaces.
    #[nve_abi(offset = 20)]
    pub white_space: u8,
    #[nve_abi(offset = 21)]
    pub word_break: u8,
    #[nve_abi(offset = 22)]
    pub overflow_wrap: u8,
    #[nve_abi(offset = 23)]
    pub _reserved: u8,
    /// CSS font-family list as authored (UTF-8, may be empty = the host's default UI font).
    pub font_family: NveStrSlice,
}

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

/// Measured surface dimensions and optional baselines returned by [`NveRuntimeApi::measure_surface`].
///
/// `first_baseline` and `last_baseline` are `NaN` when absent.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, NveAbi)]
#[nve_abi(size = 16)]
pub struct NveSurfaceMeasure {
    #[nve_abi(offset = 0)]
    pub width: f32,
    #[nve_abi(offset = 4)]
    pub height: f32,
    /// Distance from the top edge to the first alphabetic baseline (`NaN` = absent).
    #[nve_abi(offset = 8)]
    pub first_baseline: f32,
    /// Distance from the top edge to the last alphabetic baseline (`NaN` = absent).
    #[nve_abi(offset = 12)]
    pub last_baseline: f32,
}

impl NveSurfaceMeasure {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
        first_baseline: f32::NAN,
        last_baseline: f32::NAN,
    };

    #[inline]
    pub const fn from_size(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            first_baseline: f32::NAN,
            last_baseline: f32::NAN,
        }
    }
}

impl Default for NveSurfaceMeasure {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

/// Published 12-byte scroll state of one scroll container: the visual child shift (scroll offset
/// minus any `Scroll::virtual_top_spacer`) that hosts subtract from descendants' absolute
/// (pre-scroll) [`NveLayoutRect`]s.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, NveAbi)]
#[nve_abi(size = 12)]
pub struct NveScrollOffset {
    /// Raw `NodeId` of the scroll container.
    #[nve_abi(offset = 0)]
    pub node_id: u32,
    #[nve_abi(offset = 4)]
    pub x: f32,
    #[nve_abi(offset = 8)]
    pub y: f32,
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
    /// Element offset (in `u32` entries) into `NveSceneView::children_slab`.
    #[nve_abi(offset = 20)]
    pub child_off: u32,
    /// Number of child `u32` `NodeId`s in `NveSceneView::children_slab`.
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

/// Zero-copy borrowed scene view guarded by `scene_acquire` / `scene_release`.
///
/// Exposes `NodeStore`'s dense `NveNodeRow` array, dense absolute pre-scroll `NveLayoutRect`
/// array, `text_slab`, `children_slab`, per-frame `dirty_indices` / `removed_ids`, published
/// `NveTableView`s, and per-container `NveScrollOffset`s straight from the store's slabs (no
/// per-node intermediate record).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
pub struct NveSceneView {
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
    /// Number of [`NveScrollOffset`]s in `scroll_offsets`.
    #[nve_abi(offset = 40)]
    pub scroll_count: u32,
    /// Bumped whenever the runtime discards its scene (app attach / swap / detach, pairing
    /// connect / disconnect, keyframe or wire-error reset): every node id, resource id, and host
    /// cache from an earlier epoch is stale, even if the same ids reappear.
    #[nve_abi(offset = 44)]
    pub scene_epoch: u32,
    /// Count of resource ids in `dirty_resources`.
    #[nve_abi(offset = 48)]
    pub dirty_resource_count: u32,
    /// Count of moved node slot indices in `moved_indices`.
    #[nve_abi(offset = 52)]
    pub moved_count: u32,
    /// Borrowed slice of `node_capacity` [`NveNodeRow`] headers directly from `NodeStore`.
    pub rows: *const NveNodeRow,
    /// Borrowed slice of `node_capacity` [`NveLayoutRect`] boxes indexed by `NodeIndex`.
    pub rects: *const NveLayoutRect,
    /// Borrowed contiguous UTF-8 text byte slab directly from `NodeStore`.
    pub text_slab: *const u8,
    /// Borrowed contiguous `u32` child `NodeId` slab directly from `NodeStore`.
    pub children_slab: *const u32,
    /// Borrowed slice of `dirty_count` live node indices whose content changed on `frame_id`:
    /// created, or row (incl. parent), any table row, or child list changed. Disjoint from
    /// `moved_indices`; a dirty node's rect may have changed too.
    pub dirty_indices: *const u32,
    /// Borrowed slice of `removed_count` raw `NodeId`s removed on `frame_id`.
    pub removed_ids: *const u32,
    /// Borrowed slice of `table_count` [`NveTableView`] table views.
    pub tables: *const NveTableView,
    /// Borrowed slice of `scroll_count` [`NveScrollOffset`]s sorted by `node_id`; scroll
    /// containers that are absent have a zero shift.
    pub scroll_offsets: *const NveScrollOffset,
    /// Borrowed slice of `dirty_resource_count` live resource ids (re)defined on `frame_id`,
    /// strictly ascending: created, recycled, or rewritten in place. Host caches keyed by
    /// resource id (decoded images, draw lists) MUST drop these entries; ids absent here kept
    /// their bytes since the previous frame of the same `scene_epoch`.
    pub dirty_resources: *const u32,
    /// Borrowed slice of `moved_count` live node indices not in `dirty_indices` whose box
    /// relative to their parent's rect (origin, width, height) changed on `frame_id`. A host
    /// re-applies only position / size to these. Descendants of a moved node keep their
    /// parent-relative boxes and are not listed, though their absolute `rects` change: hosts
    /// position views relative to their parent (or recompute absolute rects from `rects`).
    pub moved_indices: *const u32,
}

impl Default for NveSceneView {
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
            scroll_count: 0,
            scene_epoch: 0,
            dirty_resource_count: 0,
            moved_count: 0,
            rows: core::ptr::null(),
            rects: core::ptr::null(),
            text_slab: core::ptr::null(),
            children_slab: core::ptr::null(),
            dirty_indices: core::ptr::null(),
            removed_ids: core::ptr::null(),
            tables: core::ptr::null(),
            scroll_offsets: core::ptr::null(),
            dirty_resources: core::ptr::null(),
            moved_indices: core::ptr::null(),
        }
    }
}

const _: () = {
    assert!(size_of::<NveLayoutRect>() == NVE_LAYOUT_RECT_SIZE);
    assert!(align_of::<NveLayoutRect>() == 4);
    assert!(size_of::<NveSurfaceMeasure>() == 16);
    assert!(align_of::<NveSurfaceMeasure>() == 4);
    assert!(offset_of!(NveSurfaceMeasure, width) == 0);
    assert!(offset_of!(NveSurfaceMeasure, height) == 4);
    assert!(offset_of!(NveSurfaceMeasure, first_baseline) == 8);
    assert!(offset_of!(NveSurfaceMeasure, last_baseline) == 12);
    assert!(size_of::<NveScrollOffset>() == 12);
    assert!(align_of::<NveScrollOffset>() == 4);
    assert!(size_of::<NveNodeRow>() == NVE_NODE_ROW_SIZE);
    assert!(align_of::<NveNodeRow>() == 4);
    assert!(size_of::<NveRuntimeConfig>() == 24);
    assert!(align_of::<NveRuntimeConfig>() == 4);
    // Scalars sit at fixed offsets; the trailing slice starts at 24 on every data model.
    assert!(offset_of!(NveRuntimeTextStyle, font_family) == 24);
    assert!(size_of::<NveRuntimeTextStyle>() == 24 + size_of::<NveStrSlice>());
    assert!(size_of::<NveRuntimeMetrics>() == 64);
    // 8 on every data model except i386 System V, where `u64` fields are 4-byte aligned.
    assert!(align_of::<NveRuntimeMetrics>() == align_of::<u64>());
};

concat_abi_slices!(RUNTIME_CALLBACKS: [AbiCallbackDescriptor] = [
    RUNTIME_HOOK_CALLBACKS,
    vtable::RUNTIME_ENTRY_CALLBACKS,
]);

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

const RUNTIME_VTABLES: &[AbiVtableDescriptor] = &[<NveRuntimeApi as NveVtableSchema>::SCHEMA];

/// Returns the single-source-of-truth Runtime ABI descriptor derived directly from the compiled
/// `#[derive(NveAbi)]` types in `packages/core/abi/src/runtime/`.
pub const fn runtime_abi_descriptor() -> AbiFileDescriptor {
    AbiFileDescriptor {
        namespace: Some("NveRuntimeAbi"),
        consts: RUNTIME_CONSTS,
        aliases: RUNTIME_ALIASES,
        callbacks: RUNTIME_CALLBACKS,
        enums: RUNTIME_ENUMS,
        records: RUNTIME_RECORDS,
        vtables: RUNTIME_VTABLES,
    }
}

/// Computes the deterministic, target-independent compile-time XXH64 fingerprint of
/// [`runtime_abi_descriptor`] chained with [`crate::ffi::NVE_CORE_ABI_SCHEMA_HASH`].
pub const fn compute_runtime_schema_hash() -> u64 {
    let mut h = crate::Xxh64Hasher::new(0);
    hash_tag_str(&mut h, "nve.runtime_abi.v2");
    h.update_u64(crate::ffi::NVE_CORE_ABI_SCHEMA_HASH);
    hash_abi_descriptor_into(&mut h, &runtime_abi_descriptor());
    h.digest()
}

/// Deterministic XXH64 fingerprint of the zero-copy runtime scene and vtable ABI.
pub const NVE_RUNTIME_SCHEMA_HASH: u64 = compute_runtime_schema_hash();
