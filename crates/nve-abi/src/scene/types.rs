use std::fmt;
use std::mem::{align_of, offset_of, size_of};

use crate::NodeId;

/// Exact byte size of one `NveRenderNode` record (one 64-byte CPU cache line).
pub const NVE_RENDER_NODE_SIZE: usize = 64;
/// Required alignment of `NveRenderNode` (64 bytes).
pub const NVE_RENDER_NODE_ALIGN: usize = 64;

/// Node is a leaf text node (`kind == Text` and `child_count == 0`).
pub const NVE_NODE_LEAF_TEXT: u8 = 1 << 0;
/// Node handles interactive pointer/press events (`kind == Pressable` or `event_mask != 0`).
pub const NVE_NODE_INTERACTIVE: u8 = 1 << 1;
/// Node clips its children to its rounded/rectangular bounds.
pub const NVE_NODE_CLIP_BOUNDS: u8 = 1 << 2;

/// Compact `u8` component kind discriminants stored in `NveRenderNode::kind`.
pub const NVE_NODE_KIND_ROOT: u8 = 0;
pub const NVE_NODE_KIND_VIEW: u8 = 1;
pub const NVE_NODE_KIND_TEXT: u8 = 2;
pub const NVE_NODE_KIND_SCROLL_VIEW: u8 = 3;
pub const NVE_NODE_KIND_IMAGE: u8 = 4;
pub const NVE_NODE_KIND_TEXT_INPUT: u8 = 5;
pub const NVE_NODE_KIND_PRESSABLE: u8 = 6;
pub const NVE_NODE_KIND_SKIA_CANVAS: u8 = 7;
pub const NVE_NODE_KIND_CUSTOM: u8 = 255;

/// Converts a `ComponentKind` `u16` discriminant to its compact `u8` scene-node kind.
#[inline]
pub const fn component_kind_to_scene_u8(kind: u16) -> u8 {
    match kind {
        0..=7 => kind as u8,
        _ => NVE_NODE_KIND_CUSTOM,
    }
}

/// One 64-byte (single cache line) zero-decode render node shared across all host ABIs.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NveRenderNode {
    pub node_id: u32,
    pub kind: u8,
    pub flags: u8,
    pub opacity_u8: u8,
    pub _pad0: u8,
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
    pub text_offset: u32,
    pub text_len: u16,
    pub child_count: u16,
    pub child_offset: u32,
    pub z_index: i32,
}

// Compile-time ABI layout verification for every single field of `NveRenderNode`.
const _: () = {
    assert!(size_of::<NveRenderNode>() == NVE_RENDER_NODE_SIZE);
    assert!(align_of::<NveRenderNode>() == NVE_RENDER_NODE_ALIGN);
    assert!(offset_of!(NveRenderNode, node_id) == 0);
    assert!(offset_of!(NveRenderNode, kind) == 4);
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
            node_id: 0,
            kind: NVE_NODE_KIND_VIEW,
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
            text_len: 0,
            child_count: 0,
            child_offset: 0,
            z_index: 0,
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
            && self.kind == other.kind
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
#[derive(Debug, Clone, Copy)]
pub struct NveSceneView {
    pub schema_hash: u64,
    pub frame_id: u64,
    pub node_count: u32,
    pub dirty_count: u32,
    pub removed_count: u32,
    pub text_slab_len: u32,
    pub children_slab_len: u32,
    pub _pad: u32,
    pub nodes: *const NveRenderNode,
    pub text_slab: *const u8,
    pub children_slab: *const u32,
    pub dirty_indices: *const u32,
    pub removed_ids: *const u32,
}

const _: () = {
    assert!(offset_of!(NveSceneView, schema_hash) == 0);
    assert!(offset_of!(NveSceneView, frame_id) == 8);
    assert!(offset_of!(NveSceneView, node_count) == 16);
    assert!(offset_of!(NveSceneView, dirty_count) == 20);
    assert!(offset_of!(NveSceneView, removed_count) == 24);
    assert!(offset_of!(NveSceneView, text_slab_len) == 28);
    assert!(offset_of!(NveSceneView, children_slab_len) == 32);
    assert!(offset_of!(NveSceneView, _pad) == 36);
    assert!(offset_of!(NveSceneView, nodes) == 40);
};

// ============================================================================
// Deterministic FNV-1a 64-bit Schema Hash (must match `nve-codegen::abi_schema`)
// ============================================================================

const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

const fn fnv1a_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(FNV1A_PRIME);
        i += 1;
    }
    h
}

const fn fnv1a_u64(h: u64, v: u64) -> u64 {
    fnv1a_bytes(h, &v.to_le_bytes())
}

const fn hash_fixed_field(h: u64, name: &str, ty: &str, offset: usize) -> u64 {
    let h = fnv1a_bytes(h, b"|");
    let h = fnv1a_bytes(h, name.as_bytes());
    let h = fnv1a_bytes(h, b":");
    let h = fnv1a_bytes(h, ty.as_bytes());
    let h = fnv1a_bytes(h, b"@");
    fnv1a_u64(h, offset as u64)
}

const fn hash_ptr_field(h: u64, name: &str, ty: &str, field_index: usize) -> u64 {
    let h = fnv1a_bytes(h, b"|");
    let h = fnv1a_bytes(h, name.as_bytes());
    let h = fnv1a_bytes(h, b":");
    let h = fnv1a_bytes(h, ty.as_bytes());
    let h = fnv1a_bytes(h, b"#");
    fnv1a_u64(h, field_index as u64)
}

const fn compute_scene_schema_hash() -> u64 {
    let mut h = FNV1A_OFFSET_BASIS;
    h = fnv1a_bytes(h, b"NveRenderNode");
    h = fnv1a_u64(h, size_of::<NveRenderNode>() as u64);
    h = hash_fixed_field(h, "node_id", "u32", offset_of!(NveRenderNode, node_id));
    h = hash_fixed_field(h, "kind", "u8", offset_of!(NveRenderNode, kind));
    h = hash_fixed_field(h, "flags", "u8", offset_of!(NveRenderNode, flags));
    h = hash_fixed_field(h, "opacity_u8", "u8", offset_of!(NveRenderNode, opacity_u8));
    h = hash_fixed_field(h, "_pad0", "u8", offset_of!(NveRenderNode, _pad0));
    h = hash_fixed_field(h, "x", "f32", offset_of!(NveRenderNode, x));
    h = hash_fixed_field(h, "y", "f32", offset_of!(NveRenderNode, y));
    h = hash_fixed_field(h, "width", "f32", offset_of!(NveRenderNode, width));
    h = hash_fixed_field(h, "height", "f32", offset_of!(NveRenderNode, height));
    h = hash_fixed_field(h, "bg_rgba", "u32", offset_of!(NveRenderNode, bg_rgba));
    h = hash_fixed_field(h, "text_rgba", "u32", offset_of!(NveRenderNode, text_rgba));
    h = hash_fixed_field(h, "border_rgba", "u32", offset_of!(NveRenderNode, border_rgba));
    h = hash_fixed_field(h, "border_radius", "f32", offset_of!(NveRenderNode, border_radius));
    h = hash_fixed_field(h, "border_width", "f32", offset_of!(NveRenderNode, border_width));
    h = hash_fixed_field(h, "font_size", "f32", offset_of!(NveRenderNode, font_size));
    h = hash_fixed_field(h, "text_offset", "u32", offset_of!(NveRenderNode, text_offset));
    h = hash_fixed_field(h, "text_len", "u16", offset_of!(NveRenderNode, text_len));
    h = hash_fixed_field(h, "child_count", "u16", offset_of!(NveRenderNode, child_count));
    h = hash_fixed_field(h, "child_offset", "u32", offset_of!(NveRenderNode, child_offset));
    h = hash_fixed_field(h, "z_index", "i32", offset_of!(NveRenderNode, z_index));

    h = fnv1a_bytes(h, b";NveSceneView");
    h = hash_fixed_field(h, "schema_hash", "u64", offset_of!(NveSceneView, schema_hash));
    h = hash_fixed_field(h, "frame_id", "u64", offset_of!(NveSceneView, frame_id));
    h = hash_fixed_field(h, "node_count", "u32", offset_of!(NveSceneView, node_count));
    h = hash_fixed_field(h, "dirty_count", "u32", offset_of!(NveSceneView, dirty_count));
    h = hash_fixed_field(h, "removed_count", "u32", offset_of!(NveSceneView, removed_count));
    h = hash_fixed_field(h, "text_slab_len", "u32", offset_of!(NveSceneView, text_slab_len));
    h = hash_fixed_field(
        h,
        "children_slab_len",
        "u32",
        offset_of!(NveSceneView, children_slab_len),
    );
    h = hash_fixed_field(h, "_pad", "u32", offset_of!(NveSceneView, _pad));
    h = hash_ptr_field(h, "nodes", "*const NveRenderNode", 8);
    h = hash_ptr_field(h, "text_slab", "*const u8", 9);
    h = hash_ptr_field(h, "children_slab", "*const u32", 10);
    h = hash_ptr_field(h, "dirty_indices", "*const u32", 11);
    h = hash_ptr_field(h, "removed_ids", "*const u32", 12);
    h
}

/// Deterministic FNV-1a 64-bit hash of the `NveRenderNode` + `NveSceneView` schema layout.
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
    TextOutOfBounds {
        node_id: u32,
        offset: u32,
        len: u16,
        slab_len: usize,
    },
    InvalidUtf8 {
        node_id: u32,
    },
    ChildrenOutOfBounds {
        node_id: u32,
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
    pub kind: u8,
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
    pub z_index: i32,
    pub text: &'a str,
    pub children: &'a [NodeId],
}

impl<'a> Default for SceneNodeInput<'a> {
    fn default() -> Self {
        Self {
            node_id: 0,
            kind: NVE_NODE_KIND_VIEW,
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
            z_index: 0,
            text: "",
            children: &[],
        }
    }
}
