use std::fmt;
use std::mem::{align_of, offset_of, size_of};

use nve_abi_macros::NveAbi;
use nve_hash::Xxh64Hasher;

use crate::{ByteLen, FrameId, NodeId, PrimarySlot, ShortLen, ZIndex};

/// Exact byte size of one `NveRenderNode` record (one 64-byte CPU cache line).
pub const NVE_RENDER_NODE_SIZE: usize = 64;
/// Required alignment of `NveRenderNode` (64 bytes).
pub const NVE_RENDER_NODE_ALIGN: usize = 64;

/// Exact byte size of one fixed-stride `NveNodeMeta` record (64 bytes).
pub const NVE_NODE_META_SIZE: usize = 64;
/// Natural alignment of `NveNodeMeta` (4 bytes).
pub const NVE_NODE_META_ALIGN: usize = 4;

/// Bit flags packed into `NveNodeMeta::flags`.
pub const NVE_META_FLAG_DISABLED: u32 = 1 << 0;
pub const NVE_META_FLAG_SECURE_TEXT_ENTRY: u32 = 1 << 1;
pub const NVE_META_FLAG_MULTILINE: u32 = 1 << 2;
pub const NVE_META_FLAG_ACCESSIBILITY_HIDDEN: u32 = 1 << 5;
pub const NVE_META_RESIZE_MODE_SHIFT: u32 = 8;
pub const NVE_META_RESIZE_MODE_MASK: u32 = 0x3;

/// Node is a leaf text node (`kind == Text` and `child_count == 0`).
pub const NVE_NODE_LEAF_TEXT: u8 = 1 << 0;
/// Node handles interactive pointer/press events (`kind == Pressable` or `event_mask != 0`).
pub const NVE_NODE_INTERACTIVE: u8 = 1 << 1;
/// Node clips its children to its rounded/rectangular bounds.
pub const NVE_NODE_CLIP_BOUNDS: u8 = 1 << 2;

/// Sentinel `NveRenderNode::primary_slot` value meaning plain container with no realizing table.
/// Any value in `0..128` is a uniform `TableSlot` index into `NveSceneView::tables` / `factories[]`.
pub const NVE_PRIMARY_SLOT_NONE: u8 = 0xFF;

/// One 64-byte (single cache line) zero-decode render node shared across all host ABIs.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq, NveAbi)]
#[nve_abi(size = 64)]
pub struct NveRenderNode {
    /// Stable node identifier (`NVE_ROOT_NODE_ID == 1`).
    #[nve_abi(offset = 0, c_type = "u32")]
    pub node_id: NodeId,
    /// Highest-priority realizing `TableSlot` (`0..127`), or `NVE_PRIMARY_SLOT_NONE` (`0xFF`) for a plain container.
    #[nve_abi(offset = 4, c_type = "u8")]
    pub primary_slot: PrimarySlot,
    /// Bitmask of `NVE_NODE_LEAF_TEXT`, `NVE_NODE_INTERACTIVE`, `NVE_NODE_CLIP_BOUNDS`.
    #[nve_abi(offset = 5)]
    pub flags: u8,
    /// Normalized opacity quantized to `0..=255` (`255 == 1.0`).
    #[nve_abi(offset = 6)]
    pub opacity_u8: u8,
    /// Explicit zero padding byte.
    #[nve_abi(offset = 7)]
    pub _pad0: u8,
    /// Parent-relative X origin in logical points / DIPs.
    #[nve_abi(offset = 8)]
    pub x: f32,
    /// Parent-relative Y origin in logical points / DIPs.
    #[nve_abi(offset = 12)]
    pub y: f32,
    /// Computed layout width in logical points / DIPs.
    #[nve_abi(offset = 16)]
    pub width: f32,
    /// Computed layout height in logical points / DIPs.
    #[nve_abi(offset = 20)]
    pub height: f32,
    /// Packed RGBA8888 background color (`r | (g<<8) | (b<<16) | (a<<24)`).
    #[nve_abi(offset = 24)]
    pub bg_rgba: u32,
    /// Packed RGBA8888 foreground text color.
    #[nve_abi(offset = 28)]
    pub text_rgba: u32,
    /// Packed RGBA8888 border stroke color.
    #[nve_abi(offset = 32)]
    pub border_rgba: u32,
    /// Corner radius in logical points / DIPs.
    #[nve_abi(offset = 36)]
    pub border_radius: f32,
    /// Border stroke thickness in logical points / DIPs.
    #[nve_abi(offset = 40)]
    pub border_width: f32,
    /// Font size in logical points / DIPs.
    #[nve_abi(offset = 44)]
    pub font_size: f32,
    /// Byte offset into `text_slab`.
    #[nve_abi(offset = 48)]
    pub text_offset: u32,
    /// UTF-8 byte length inside `text_slab`.
    #[nve_abi(offset = 52, c_type = "u16")]
    pub text_len: ShortLen,
    /// Number of child `u32` IDs inside `children_slab`.
    #[nve_abi(offset = 54)]
    pub child_count: u16,
    /// Element offset (in `u32` entries) into `children_slab`.
    #[nve_abi(offset = 56)]
    pub child_offset: u32,
    /// Stacking z-index order.
    #[nve_abi(offset = 60, c_type = "i32")]
    pub z_index: ZIndex,
}

// Compile-time ABI layout verification for every single field of `NveRenderNode`.
const _: () = {
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
};

impl NveRenderNode {
    /// Returns a zero-initialized `NveRenderNode` with full opacity (`255`) and `16.0` font size.
    #[inline]
    pub const fn zeroed() -> Self {
        Self {
            node_id: NodeId::NULL,
            primary_slot: PrimarySlot::NONE,
            flags: 0,
            opacity_u8: 255,
            _pad0: 0,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            bg_rgba: 0,
            text_rgba: 0,
            border_rgba: 0,
            border_radius: 0.0,
            border_width: 0.0,
            font_size: 16.0,
            text_offset: 0,
            text_len: ShortLen::ZERO,
            child_count: 0,
            child_offset: 0,
            z_index: ZIndex::ZERO,
        }
    }

    #[inline]
    pub const fn is_leaf_text(&self) -> bool {
        (self.flags & NVE_NODE_LEAF_TEXT) != 0
    }

    #[inline]
    pub const fn is_interactive(&self) -> bool {
        (self.flags & NVE_NODE_INTERACTIVE) != 0
    }

    #[inline]
    pub const fn clips_bounds(&self) -> bool {
        (self.flags & NVE_NODE_CLIP_BOUNDS) != 0
    }

    #[inline]
    pub fn opacity_f32(&self) -> f32 {
        f32::from(self.opacity_u8) / 255.0
    }

    #[inline]
    pub fn encode_opacity_u8(opacity: f32) -> u8 {
        if !opacity.is_finite() {
            return 255;
        }
        (opacity.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// Exact bitwise equality across all 64 bytes (treats identical float bit patterns as equal).
    #[inline]
    pub fn bit_eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
            && self.primary_slot == other.primary_slot
            && self.flags == other.flags
            && self.opacity_u8 == other.opacity_u8
            && self._pad0 == other._pad0
            && self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.width.to_bits() == other.width.to_bits()
            && self.height.to_bits() == other.height.to_bits()
            && self.bg_rgba == other.bg_rgba
            && self.text_rgba == other.text_rgba
            && self.border_rgba == other.border_rgba
            && self.border_radius.to_bits() == other.border_radius.to_bits()
            && self.border_width.to_bits() == other.border_width.to_bits()
            && self.font_size.to_bits() == other.font_size.to_bits()
            && self.text_offset == other.text_offset
            && self.text_len == other.text_len
            && self.child_count == other.child_count
            && self.child_offset == other.child_offset
            && self.z_index == other.z_index
    }
}

/// Zero-decode flat scene view handed across the FFI / JNI / WASM boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
pub struct NveSceneView {
    /// Must equal `NVE_SCENE_SCHEMA_HASH`.
    #[nve_abi(offset = 0)]
    pub schema_hash: u64,
    /// Monotonic frame counter.
    #[nve_abi(offset = 8, c_type = "u64")]
    pub frame_id: FrameId,
    /// Total live nodes in `nodes`.
    #[nve_abi(offset = 16)]
    pub node_count: u32,
    /// Count of changed node indices in `dirty_indices`.
    #[nve_abi(offset = 20)]
    pub dirty_count: u32,
    /// Count of removed node IDs in `removed_ids`.
    #[nve_abi(offset = 24)]
    pub removed_count: u32,
    /// Byte length of `text_slab`.
    #[nve_abi(offset = 28, c_type = "u32")]
    pub text_slab_len: ByteLen,
    /// Element count (`u32` entries) of `children_slab`.
    #[nve_abi(offset = 32, c_type = "u32")]
    pub children_slab_len: ByteLen,
    /// Number of published `NveTableView` entries in `tables`.
    #[nve_abi(offset = 36)]
    pub table_count: u32,
    /// Contiguous array of `node_count` `NveRenderNode` entries.
    pub nodes: *const NveRenderNode,
    /// Contiguous UTF-8 text byte slab.
    pub text_slab: *const u8,
    /// Contiguous `u32` child node ID slab.
    #[nve_abi(c_type = "*const u32")]
    pub children_slab: *const NodeId,
    /// Indices into `nodes` modified during `frame_id`.
    pub dirty_indices: *const u32,
    /// Node IDs removed during `frame_id`.
    #[nve_abi(c_type = "*const u32")]
    pub removed_ids: *const NodeId,
    /// Contiguous array of `table_count` zero-copy [`crate::table::NveTableView`] entries.
    pub tables: *const crate::table::NveTableView,
}

const _: () = {
    assert!(offset_of!(NveSceneView, schema_hash) == 0);
    assert!(offset_of!(NveSceneView, frame_id) == 8);
    assert!(offset_of!(NveSceneView, node_count) == 16);
    assert!(offset_of!(NveSceneView, dirty_count) == 20);
    assert!(offset_of!(NveSceneView, removed_count) == 24);
    assert!(offset_of!(NveSceneView, text_slab_len) == 28);
    assert!(offset_of!(NveSceneView, children_slab_len) == 32);
    assert!(offset_of!(NveSceneView, table_count) == 36);
    assert!(offset_of!(NveSceneView, nodes) == 40);
};

/// Fixed-stride 64-byte per-node accessibility, input, image, and draw-list metadata record
/// indexing into a flat companion UTF-8/binary blob slab.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, NveAbi)]
#[nve_abi(size = 64)]
pub struct NveNodeMeta {
    #[nve_abi(offset = 0, c_type = "u32")]
    pub node_id: NodeId,
    #[nve_abi(offset = 4)]
    pub flags: u32,
    #[nve_abi(offset = 8)]
    pub label_offset: u32,
    #[nve_abi(offset = 12, c_type = "u16")]
    pub label_len: ShortLen,
    #[nve_abi(offset = 14, c_type = "u16")]
    pub role_len: ShortLen,
    #[nve_abi(offset = 16)]
    pub role_offset: u32,
    #[nve_abi(offset = 20)]
    pub hint_offset: u32,
    #[nve_abi(offset = 24, c_type = "u16")]
    pub hint_len: ShortLen,
    #[nve_abi(offset = 26, c_type = "u16")]
    pub state_desc_len: ShortLen,
    #[nve_abi(offset = 28)]
    pub state_desc_offset: u32,
    #[nve_abi(offset = 32)]
    pub placeholder_offset: u32,
    #[nve_abi(offset = 36, c_type = "u16")]
    pub placeholder_len: ShortLen,
    #[nve_abi(offset = 38, c_type = "u16")]
    pub test_id_len: ShortLen,
    #[nve_abi(offset = 40)]
    pub test_id_offset: u32,
    #[nve_abi(offset = 44)]
    pub _reserved: [u32; 5],
}

const _: () = {
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
};

// ============================================================================
// Deterministic XXH64 Schema Hash (must match `nve-codegen::compute_scene_schema_hash`)
// ============================================================================

const fn hash_field(h: &mut Xxh64Hasher, name: &str, ty: &str, sep: &[u8], v: usize) {
    h.update(b"|");
    h.update(name.as_bytes());
    h.update(b":");
    h.update(ty.as_bytes());
    h.update(sep);
    h.update_u64(v as u64);
}

const fn hash_fixed_field(h: &mut Xxh64Hasher, name: &str, ty: &str, offset: usize) {
    hash_field(h, name, ty, b"@", offset);
}

const fn hash_ptr_field(h: &mut Xxh64Hasher, name: &str, ty: &str, field_index: usize) {
    hash_field(h, name, ty, b"#", field_index);
}

const fn compute_scene_schema_hash() -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(b"NveRenderNode");
    h.update_u64(size_of::<NveRenderNode>() as u64);
    hash_fixed_field(&mut h, "node_id", "u32", offset_of!(NveRenderNode, node_id));
    hash_fixed_field(&mut h, "primary_slot", "u8", offset_of!(NveRenderNode, primary_slot));
    hash_fixed_field(&mut h, "flags", "u8", offset_of!(NveRenderNode, flags));
    hash_fixed_field(&mut h, "opacity_u8", "u8", offset_of!(NveRenderNode, opacity_u8));
    hash_fixed_field(&mut h, "_pad0", "u8", offset_of!(NveRenderNode, _pad0));
    hash_fixed_field(&mut h, "x", "f32", offset_of!(NveRenderNode, x));
    hash_fixed_field(&mut h, "y", "f32", offset_of!(NveRenderNode, y));
    hash_fixed_field(&mut h, "width", "f32", offset_of!(NveRenderNode, width));
    hash_fixed_field(&mut h, "height", "f32", offset_of!(NveRenderNode, height));
    hash_fixed_field(&mut h, "bg_rgba", "u32", offset_of!(NveRenderNode, bg_rgba));
    hash_fixed_field(&mut h, "text_rgba", "u32", offset_of!(NveRenderNode, text_rgba));
    hash_fixed_field(&mut h, "border_rgba", "u32", offset_of!(NveRenderNode, border_rgba));
    hash_fixed_field(&mut h, "border_radius", "f32", offset_of!(NveRenderNode, border_radius));
    hash_fixed_field(&mut h, "border_width", "f32", offset_of!(NveRenderNode, border_width));
    hash_fixed_field(&mut h, "font_size", "f32", offset_of!(NveRenderNode, font_size));
    hash_fixed_field(&mut h, "text_offset", "u32", offset_of!(NveRenderNode, text_offset));
    hash_fixed_field(&mut h, "text_len", "u16", offset_of!(NveRenderNode, text_len));
    hash_fixed_field(&mut h, "child_count", "u16", offset_of!(NveRenderNode, child_count));
    hash_fixed_field(&mut h, "child_offset", "u32", offset_of!(NveRenderNode, child_offset));
    hash_fixed_field(&mut h, "z_index", "i32", offset_of!(NveRenderNode, z_index));

    h.update(b";NveSceneView");
    hash_fixed_field(&mut h, "schema_hash", "u64", offset_of!(NveSceneView, schema_hash));
    hash_fixed_field(&mut h, "frame_id", "u64", offset_of!(NveSceneView, frame_id));
    hash_fixed_field(&mut h, "node_count", "u32", offset_of!(NveSceneView, node_count));
    hash_fixed_field(&mut h, "dirty_count", "u32", offset_of!(NveSceneView, dirty_count));
    hash_fixed_field(&mut h, "removed_count", "u32", offset_of!(NveSceneView, removed_count));
    hash_fixed_field(&mut h, "text_slab_len", "u32", offset_of!(NveSceneView, text_slab_len));
    hash_fixed_field(
        &mut h,
        "children_slab_len",
        "u32",
        offset_of!(NveSceneView, children_slab_len),
    );
    hash_fixed_field(&mut h, "table_count", "u32", offset_of!(NveSceneView, table_count));
    hash_ptr_field(&mut h, "nodes", "*const NveRenderNode", 8);
    hash_ptr_field(&mut h, "text_slab", "*const u8", 9);
    hash_ptr_field(&mut h, "children_slab", "*const u32", 10);
    hash_ptr_field(&mut h, "dirty_indices", "*const u32", 11);
    hash_ptr_field(&mut h, "removed_ids", "*const u32", 12);
    hash_ptr_field(&mut h, "tables", "*const NveTableView", 13);

    h.update(b";NveTableView");
    hash_fixed_field(&mut h, "key_lo", "u32", offset_of!(crate::table::NveTableView, key_lo));
    hash_fixed_field(&mut h, "key_hi", "u32", offset_of!(crate::table::NveTableView, key_hi));
    hash_fixed_field(&mut h, "slot", "u32", offset_of!(crate::table::NveTableView, slot));
    hash_fixed_field(&mut h, "words", "u32", offset_of!(crate::table::NveTableView, words));
    hash_fixed_field(&mut h, "row_count", "u32", offset_of!(crate::table::NveTableView, row_count));
    hash_fixed_field(&mut h, "dirty_count", "u32", offset_of!(crate::table::NveTableView, dirty_count));
    hash_ptr_field(&mut h, "rows", "*const u32", 6);
    hash_ptr_field(&mut h, "owners", "*const u32", 7);
    hash_ptr_field(&mut h, "dirty_rows", "*const u32", 8);

    h.update(b";NveNodeMeta");
    h.update_u64(size_of::<NveNodeMeta>() as u64);
    hash_fixed_field(&mut h, "node_id", "u32", offset_of!(NveNodeMeta, node_id));
    hash_fixed_field(&mut h, "flags", "u32", offset_of!(NveNodeMeta, flags));
    hash_fixed_field(&mut h, "label_offset", "u32", offset_of!(NveNodeMeta, label_offset));
    hash_fixed_field(&mut h, "label_len", "u16", offset_of!(NveNodeMeta, label_len));
    hash_fixed_field(&mut h, "role_len", "u16", offset_of!(NveNodeMeta, role_len));
    hash_fixed_field(&mut h, "role_offset", "u32", offset_of!(NveNodeMeta, role_offset));
    hash_fixed_field(&mut h, "hint_offset", "u32", offset_of!(NveNodeMeta, hint_offset));
    hash_fixed_field(&mut h, "hint_len", "u16", offset_of!(NveNodeMeta, hint_len));
    hash_fixed_field(&mut h, "state_desc_len", "u16", offset_of!(NveNodeMeta, state_desc_len));
    hash_fixed_field(&mut h, "state_desc_offset", "u32", offset_of!(NveNodeMeta, state_desc_offset));
    hash_fixed_field(&mut h, "placeholder_offset", "u32", offset_of!(NveNodeMeta, placeholder_offset));
    hash_fixed_field(&mut h, "placeholder_len", "u16", offset_of!(NveNodeMeta, placeholder_len));
    hash_fixed_field(&mut h, "test_id_len", "u16", offset_of!(NveNodeMeta, test_id_len));
    hash_fixed_field(&mut h, "test_id_offset", "u32", offset_of!(NveNodeMeta, test_id_offset));
    hash_fixed_field(&mut h, "_reserved", "[u32;5]", offset_of!(NveNodeMeta, _reserved));
    h.digest()
}

/// Deterministic XXH64 hash of the `NveRenderNode` + `NveSceneView` schema layout.
/// Verified at runtime across every language boundary to make ABI drift impossible.
pub const NVE_SCENE_SCHEMA_HASH: u64 = compute_scene_schema_hash();

/// Errors returned while mutating or validating an `NveSceneView`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    SchemaHashMismatch {
        expected: u64,
        actual: u64,
    },
    NullBufferPointer(&'static str),
    UnalignedBufferPointer(&'static str),
    BufferLengthOverflow(&'static str),
    TextOutOfBounds {
        node_id: NodeId,
        offset: u32,
        len: ShortLen,
        slab_len: usize,
    },
    InvalidUtf8 {
        node_id: NodeId,
    },
    ChildrenOutOfBounds {
        node_id: NodeId,
        offset: u32,
        count: u16,
        slab_len: usize,
    },
    TextTooLong {
        len: usize,
    },
    ChildrenTooMany {
        count: usize,
    },
    SlabOverflow(&'static str),
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for SceneError {}

/// Borrowed input describing one node when updating [`super::NveSceneTable`].
#[derive(Debug, Clone, Copy)]
pub struct SceneNodeInput<'a> {
    pub node_id: NodeId,
    pub primary_slot: PrimarySlot,
    pub flags: u8,
    pub opacity_u8: u8,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub bg_rgba: u32,
    pub text_rgba: u32,
    pub border_rgba: u32,
    pub border_radius: f32,
    pub border_width: f32,
    pub font_size: f32,
    pub z_index: ZIndex,
    pub text: &'a str,
    pub children: &'a [NodeId],
}

impl<'a> Default for SceneNodeInput<'a> {
    fn default() -> Self {
        Self {
            node_id: NodeId::NULL,
            primary_slot: PrimarySlot::NONE,
            flags: 0,
            opacity_u8: 255,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            bg_rgba: 0,
            text_rgba: 0,
            border_rgba: 0,
            border_radius: 0.0,
            border_width: 0.0,
            font_size: 16.0,
            z_index: ZIndex::ZERO,
            text: "",
            children: &[],
        }
    }
}
