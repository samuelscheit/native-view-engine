//! Declarative single-source specification for NVE's zero-decode 64-byte
//! `NveRenderNode` + `NveSceneView` ABI (`nve_scene_abi.h`, `nve_scene_abi_gen.dart`,
//! `NveSceneBuffer.java`, `nve_scene_view.ts`).
//!
//! Dogfoods NVE's own Nitro-style Rust-first Hybrid FFI parser & generator
//! (`crates/nve-codegen/src/hybrid/`).

#[nve_hybrid_namespace(name = "NveSceneAbi")]

/// Exact byte stride of one `NveRenderNode` record (one 64-byte cache line).
#[nve_hybrid_const]
pub const NVE_RENDER_NODE_SIZE: u32 = 64;

/// Required cache-line alignment of `NveRenderNode` backing slabs in Rust.
#[nve_hybrid_const]
pub const NVE_RENDER_NODE_ALIGN: u32 = 64;

/// Bit flags on `NveRenderNode::flags`.
#[nve_hybrid_const]
pub const NVE_NODE_LEAF_TEXT: u32 = 1;

#[nve_hybrid_const]
pub const NVE_NODE_INTERACTIVE: u32 = 2;

#[nve_hybrid_const]
pub const NVE_NODE_CLIP_BOUNDS: u32 = 4;

/// Compact `uint8_t` `NveRenderNode::kind` discriminants.
#[nve_hybrid_const]
pub const NVE_NODE_KIND_ROOT: u32 = 0;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_VIEW: u32 = 1;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_TEXT: u32 = 2;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_SCROLL_VIEW: u32 = 3;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_IMAGE: u32 = 4;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_TEXT_INPUT: u32 = 5;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_PRESSABLE: u32 = 6;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_SKIA_CANVAS: u32 = 7;

#[nve_hybrid_const]
pub const NVE_NODE_KIND_CUSTOM: u32 = 255;

/// 64-byte zero-decode render node (one cache line; Rust aligns backing slabs to 64 bytes).
#[repr(C, align(64))]
#[nve_hybrid_struct(size = 64)]
pub struct NveRenderNode {
    /// Stable node identifier (`NVE_ROOT_NODE_ID == 1`).
    #[offset(0)]
    pub node_id: u32,
    /// Compact `NVE_NODE_KIND_*` component discriminant.
    #[offset(4)]
    pub kind: u8,
    /// Bitmask of `NVE_NODE_LEAF_TEXT`, `NVE_NODE_INTERACTIVE`, `NVE_NODE_CLIP_BOUNDS`.
    #[offset(5)]
    pub flags: u8,
    /// Normalized opacity quantized to `0..=255` (`255 == 1.0`).
    #[offset(6)]
    pub opacity_u8: u8,
    /// Explicit zero padding byte.
    #[offset(7)]
    pub _pad0: u8,
    /// Parent-relative X origin in logical points / DIPs.
    #[offset(8)]
    pub x: f32,
    /// Parent-relative Y origin in logical points / DIPs.
    #[offset(12)]
    pub y: f32,
    /// Computed layout width in logical points / DIPs.
    #[offset(16)]
    pub width: f32,
    /// Computed layout height in logical points / DIPs.
    #[offset(20)]
    pub height: f32,
    /// Packed RGBA8888 background color (`r | (g<<8) | (b<<16) | (a<<24)`).
    #[offset(24)]
    pub bg_rgba: u32,
    /// Packed RGBA8888 foreground text color.
    #[offset(28)]
    pub text_rgba: u32,
    /// Packed RGBA8888 border stroke color.
    #[offset(32)]
    pub border_rgba: u32,
    /// Corner radius in logical points / DIPs.
    #[offset(36)]
    pub border_radius: f32,
    /// Border stroke thickness in logical points / DIPs.
    #[offset(40)]
    pub border_width: f32,
    /// Font size in logical points / DIPs.
    #[offset(44)]
    pub font_size: f32,
    /// Byte offset into `text_slab`.
    #[offset(48)]
    pub text_offset: u32,
    /// UTF-8 byte length inside `text_slab`.
    #[offset(52)]
    pub text_len: u16,
    /// Number of child `u32` IDs inside `children_slab`.
    #[offset(54)]
    pub child_count: u16,
    /// Element offset (in `u32` entries) into `children_slab`.
    #[offset(56)]
    pub child_offset: u32,
    /// Stacking z-index order.
    #[offset(60)]
    pub z_index: i32,
}

/// Zero-decode scene snapshot header + contiguous slab pointers.
#[repr(C)]
#[nve_hybrid_struct]
pub struct NveSceneView {
    /// Must equal `NVE_SCENE_SCHEMA_HASH`.
    #[offset(0)]
    pub schema_hash: u64,
    /// Monotonic frame counter.
    #[offset(8)]
    pub frame_id: u64,
    /// Total live nodes in `nodes`.
    #[offset(16)]
    pub node_count: u32,
    /// Count of changed node indices in `dirty_indices`.
    #[offset(20)]
    pub dirty_count: u32,
    /// Count of removed node IDs in `removed_ids`.
    #[offset(24)]
    pub removed_count: u32,
    /// Byte length of `text_slab`.
    #[offset(28)]
    pub text_slab_len: u32,
    /// Element count (`u32` entries) of `children_slab`.
    #[offset(32)]
    pub children_slab_len: u32,
    /// Explicit 8-byte pointer alignment padding.
    #[offset(36)]
    pub _pad: u32,
    /// Contiguous array of `node_count` `NveRenderNode` entries.
    pub nodes: *const NveRenderNode,
    /// Contiguous UTF-8 text byte slab.
    pub text_slab: *const u8,
    /// Contiguous `u32` child node ID slab.
    pub children_slab: *const u32,
    /// Indices into `nodes` modified during `frame_id`.
    pub dirty_indices: *const u32,
    /// Node IDs removed during `frame_id`.
    pub removed_ids: *const u32,
}
