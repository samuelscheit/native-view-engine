//! Single-source 64-byte cache-line `NveRenderNode` and zero-decode `NveSceneView` ABI.
//!
//! Instead of decoding a variable-length `CommandBuffer` or serializing JSON on every host
//! frame, runtimes publish a flat contiguous array of 64-byte `#[repr(C, align(64))]`
//! `NveRenderNode` records alongside three flat slabs (`text_slab`, `children_slab`,
//! `dirty_indices`) plus `removed_ids`. Native hosts (C/C++, Swift, Dart `dart:ffi`,
//! Android `DirectByteBuffer`, and Web WASM `DataView`) read dirty nodes directly in
//! `O(changed)` with zero decoding and zero per-frame allocations.

use std::collections::HashMap;
use std::fmt;
use std::mem::{align_of, offset_of, size_of};

use crate::{FrameId, NodeId};

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

/// Borrowed input describing one node when updating [`NveSceneTable`].
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

/// Incremental `O(changed)` scene table backing an [`NveSceneView`].
///
/// Tracks per-frame `dirty_indices` and `removed_ids`, deduplicates unchanged text/children slab
/// slices in place, and compacts dead slab space automatically when dead bytes exceed live bytes.
#[derive(Debug, Clone, Default)]
pub struct NveSceneTable {
    frame_id: FrameId,
    nodes: Vec<NveRenderNode>,
    id_to_index: HashMap<NodeId, usize>,
    text_slab: Vec<u8>,
    children_slab: Vec<u32>,
    dirty_indices: Vec<u32>,
    dirty_marks: Vec<u64>,
    seen_marks: Vec<u64>,
    removed_ids: Vec<u32>,
    mark_epoch: u64,
    live_text_bytes: usize,
    live_children_len: usize,
}

impl NveSceneTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(node_capacity: usize) -> Self {
        Self {
            frame_id: 0,
            nodes: Vec::with_capacity(node_capacity),
            id_to_index: HashMap::with_capacity(node_capacity),
            text_slab: Vec::with_capacity(node_capacity * 16),
            children_slab: Vec::with_capacity(node_capacity),
            dirty_indices: Vec::with_capacity(node_capacity.min(64)),
            dirty_marks: Vec::with_capacity(node_capacity),
            seen_marks: Vec::with_capacity(node_capacity),
            removed_ids: Vec::new(),
            mark_epoch: 0,
            live_text_bytes: 0,
            live_children_len: 0,
        }
    }

    #[inline]
    pub fn frame_id(&self) -> FrameId {
        self.frame_id
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn index_of(&self, node_id: NodeId) -> Option<usize> {
        self.id_to_index.get(&node_id).copied()
    }

    /// Starts a fresh frame, resetting `dirty_indices` and `removed_ids`.
    pub fn begin_frame(&mut self, frame_id: FrameId) {
        self.frame_id = frame_id;
        self.mark_epoch = self.mark_epoch.wrapping_add(1);
        if self.mark_epoch == 0 {
            self.dirty_marks.fill(0);
            self.seen_marks.fill(0);
            self.mark_epoch = 1;
        }
        self.dirty_indices.clear();
        self.removed_ids.clear();
    }

    /// Clears every node and slab.
    pub fn clear(&mut self) {
        self.frame_id = 0;
        self.nodes.clear();
        self.id_to_index.clear();
        self.text_slab.clear();
        self.children_slab.clear();
        self.dirty_indices.clear();
        self.dirty_marks.clear();
        self.seen_marks.clear();
        self.removed_ids.clear();
        self.live_text_bytes = 0;
        self.live_children_len = 0;
    }

    #[inline]
    fn mark_dirty(&mut self, idx: usize) {
        if self.dirty_marks[idx] != self.mark_epoch {
            self.dirty_marks[idx] = self.mark_epoch;
            self.dirty_indices.push(idx as u32);
        }
    }

    /// Inserts or updates a single node in `O(1)` amortized time, recording its index in
    /// `dirty_indices` only when at least one field, text byte, or child ID changed.
    pub fn upsert_node(&mut self, input: SceneNodeInput<'_>) -> Result<u32, SceneError> {
        if self.mark_epoch == 0 {
            self.begin_frame(self.frame_id.wrapping_add(1));
        }
        let text_bytes = input.text.as_bytes();
        let text_len_u16 =
            u16::try_from(text_bytes.len()).map_err(|_| SceneError::TextTooLong {
                len: text_bytes.len(),
            })?;
        let child_count_u16 =
            u16::try_from(input.children.len()).map_err(|_| SceneError::ChildrenTooMany {
                count: input.children.len(),
            })?;

        if let Some(&idx) = self.id_to_index.get(&input.node_id) {
            let prev = self.nodes[idx];
            self.seen_marks[idx] = self.mark_epoch;

            let (text_offset, text_len) = {
                let prev_start = prev.text_offset as usize;
                let prev_end = prev_start + prev.text_len as usize;
                let unchanged = prev_end <= self.text_slab.len()
                    && &self.text_slab[prev_start..prev_end] == text_bytes;
                if unchanged {
                    (prev.text_offset, prev.text_len)
                } else {
                    self.live_text_bytes = self
                        .live_text_bytes
                        .saturating_sub(prev.text_len as usize)
                        + text_bytes.len();
                    if text_bytes.is_empty() {
                        (0u32, 0u16)
                    } else {
                        let off = u32::try_from(self.text_slab.len())
                            .map_err(|_| SceneError::SlabOverflow("text_slab"))?;
                        self.text_slab.extend_from_slice(text_bytes);
                        (off, text_len_u16)
                    }
                }
            };

            let (child_offset, child_count) = {
                let prev_start = prev.child_offset as usize;
                let prev_end = prev_start + prev.child_count as usize;
                let unchanged = prev_end <= self.children_slab.len()
                    && &self.children_slab[prev_start..prev_end] == input.children;
                if unchanged {
                    (prev.child_offset, prev.child_count)
                } else {
                    self.live_children_len = self
                        .live_children_len
                        .saturating_sub(prev.child_count as usize)
                        + input.children.len();
                    if input.children.is_empty() {
                        (0u32, 0u16)
                    } else {
                        let off = u32::try_from(self.children_slab.len())
                            .map_err(|_| SceneError::SlabOverflow("children_slab"))?;
                        self.children_slab.extend_from_slice(input.children);
                        (off, child_count_u16)
                    }
                }
            };

            let next = NveRenderNode {
                node_id: input.node_id,
                kind: input.kind,
                flags: input.flags,
                opacity_u8: input.opacity_u8,
                _pad0: 0,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
                bg_rgba: input.bg_rgba,
                text_rgba: input.text_rgba,
                border_rgba: input.border_rgba,
                border_radius: input.border_radius,
                border_width: input.border_width,
                font_size: input.font_size,
                text_offset,
                text_len,
                child_count,
                child_offset,
                z_index: input.z_index,
            };

            if !prev.bit_eq(&next) {
                self.nodes[idx] = next;
                self.mark_dirty(idx);
            }
            Ok(idx as u32)
        } else {
            let text_offset = if text_bytes.is_empty() {
                0
            } else {
                let off = u32::try_from(self.text_slab.len())
                    .map_err(|_| SceneError::SlabOverflow("text_slab"))?;
                self.text_slab.extend_from_slice(text_bytes);
                self.live_text_bytes += text_bytes.len();
                off
            };

            let child_offset = if input.children.is_empty() {
                0
            } else {
                let off = u32::try_from(self.children_slab.len())
                    .map_err(|_| SceneError::SlabOverflow("children_slab"))?;
                self.children_slab.extend_from_slice(input.children);
                self.live_children_len += input.children.len();
                off
            };

            let idx = self.nodes.len();
            let idx_u32 =
                u32::try_from(idx).map_err(|_| SceneError::SlabOverflow("nodes"))?;
            self.nodes.push(NveRenderNode {
                node_id: input.node_id,
                kind: input.kind,
                flags: input.flags,
                opacity_u8: input.opacity_u8,
                _pad0: 0,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
                bg_rgba: input.bg_rgba,
                text_rgba: input.text_rgba,
                border_rgba: input.border_rgba,
                border_radius: input.border_radius,
                border_width: input.border_width,
                font_size: input.font_size,
                text_offset,
                text_len: text_len_u16,
                child_count: child_count_u16,
                child_offset,
                z_index: input.z_index,
            });
            self.dirty_marks.push(0);
            self.seen_marks.push(self.mark_epoch);
            self.id_to_index.insert(input.node_id, idx);
            self.mark_dirty(idx);
            Ok(idx_u32)
        }
    }

    /// Removes `node_id` via O(1) swap-remove, appending `node_id` to `removed_ids` and marking
    /// the destination slot dirty if another node moved into it.
    pub fn remove_node(&mut self, node_id: NodeId) -> bool {
        let Some(idx) = self.id_to_index.remove(&node_id) else {
            return false;
        };
        if self.mark_epoch == 0 {
            self.begin_frame(self.frame_id.wrapping_add(1));
        }

        let removed = self.nodes[idx];
        self.live_text_bytes = self
            .live_text_bytes
            .saturating_sub(removed.text_len as usize);
        self.live_children_len = self
            .live_children_len
            .saturating_sub(removed.child_count as usize);
        self.removed_ids.push(node_id);

        let last_idx = self.nodes.len() - 1;
        if idx != last_idx {
            let moved = self.nodes[last_idx];
            self.nodes[idx] = moved;
            self.seen_marks[idx] = self.seen_marks[last_idx];
            self.dirty_marks[idx] = 0;
            self.id_to_index.insert(moved.node_id, idx);
        }
        self.nodes.pop();
        self.dirty_marks.pop();
        self.seen_marks.pop();

        // Filter any out-of-range dirty index (`last_idx`) and ensure swapped slot `idx` is dirty.
        let new_len = self.nodes.len() as u32;
        self.dirty_indices.retain(|&di| di < new_len);
        if idx < self.nodes.len() {
            self.mark_dirty(idx);
        }
        true
    }

    /// Reconciles a complete frame snapshot in one pass, computing exact `dirty_indices` and
    /// `removed_ids` and compacting dead slab memory when needed.
    pub fn sync_frame<'a>(
        &mut self,
        frame_id: FrameId,
        inputs: impl IntoIterator<Item = SceneNodeInput<'a>>,
    ) -> Result<(), SceneError> {
        self.begin_frame(frame_id);
        for input in inputs {
            self.upsert_node(input)?;
        }

        // Remove any previously live node not seen in this frame.
        let epoch = self.mark_epoch;
        let mut i = 0;
        while i < self.nodes.len() {
            if self.seen_marks[i] != epoch {
                let dead_id = self.nodes[i].node_id;
                self.remove_node(dead_id);
                // Re-check index `i` because swap-remove moved the last element here.
            } else {
                i += 1;
            }
        }

        self.maybe_compact_slabs();
        Ok(())
    }

    /// Compacts `text_slab` and `children_slab` if dead bytes significantly exceed live bytes.
    pub fn maybe_compact_slabs(&mut self) {
        if self.text_slab.len() > 64 * 1024 && self.text_slab.len() > self.live_text_bytes * 2 {
            self.compact_text_slab();
        }
        if self.children_slab.len() > 16 * 1024
            && self.children_slab.len() > self.live_children_len * 2
        {
            self.compact_children_slab();
        }
    }

    /// Unconditionally compacts `text_slab` in place, rewriting live node text offsets.
    pub fn compact_text_slab(&mut self) {
        let mut new_slab = Vec::with_capacity(self.live_text_bytes);
        for node in &mut self.nodes {
            if node.text_len == 0 {
                node.text_offset = 0;
                continue;
            }
            let start = node.text_offset as usize;
            let end = start + node.text_len as usize;
            let new_off = new_slab.len() as u32;
            new_slab.extend_from_slice(&self.text_slab[start..end]);
            node.text_offset = new_off;
        }
        self.live_text_bytes = new_slab.len();
        self.text_slab = new_slab;
    }

    /// Unconditionally compacts `children_slab` in place, rewriting live node child offsets.
    pub fn compact_children_slab(&mut self) {
        let mut new_slab = Vec::with_capacity(self.live_children_len);
        for node in &mut self.nodes {
            if node.child_count == 0 {
                node.child_offset = 0;
                continue;
            }
            let start = node.child_offset as usize;
            let end = start + node.child_count as usize;
            let new_off = new_slab.len() as u32;
            new_slab.extend_from_slice(&self.children_slab[start..end]);
            node.child_offset = new_off;
        }
        self.live_children_len = new_slab.len();
        self.children_slab = new_slab;
    }

    /// Returns a zero-copy C ABI [`NveSceneView`] pointing directly into this table's slabs.
    ///
    /// The returned pointers remain valid until the next mutating call on `self`.
    #[inline]
    pub fn as_view(&self) -> NveSceneView {
        NveSceneView {
            schema_hash: NVE_SCENE_SCHEMA_HASH,
            frame_id: self.frame_id,
            node_count: self.nodes.len() as u32,
            dirty_count: self.dirty_indices.len() as u32,
            removed_count: self.removed_ids.len() as u32,
            text_slab_len: self.text_slab.len() as u32,
            children_slab_len: self.children_slab.len() as u32,
            _pad: 0,
            nodes: if self.nodes.is_empty() {
                std::ptr::null()
            } else {
                self.nodes.as_ptr()
            },
            text_slab: if self.text_slab.is_empty() {
                std::ptr::null()
            } else {
                self.text_slab.as_ptr()
            },
            children_slab: if self.children_slab.is_empty() {
                std::ptr::null()
            } else {
                self.children_slab.as_ptr()
            },
            dirty_indices: if self.dirty_indices.is_empty() {
                std::ptr::null()
            } else {
                self.dirty_indices.as_ptr()
            },
            removed_ids: if self.removed_ids.is_empty() {
                std::ptr::null()
            } else {
                self.removed_ids.as_ptr()
            },
        }
    }

    /// Returns a safe zero-copy borrowed [`NveSceneRef`] over this table.
    #[inline]
    pub fn as_ref(&self) -> NveSceneRef<'_> {
        NveSceneRef {
            frame_id: self.frame_id,
            nodes: &self.nodes,
            text_slab: &self.text_slab,
            children_slab: &self.children_slab,
            dirty_indices: &self.dirty_indices,
            removed_ids: &self.removed_ids,
        }
    }
}

/// Safe, zero-copy borrowed reader over an [`NveSceneView`].
#[derive(Debug, Clone, Copy)]
pub struct NveSceneRef<'a> {
    frame_id: FrameId,
    nodes: &'a [NveRenderNode],
    text_slab: &'a [u8],
    children_slab: &'a [u32],
    dirty_indices: &'a [u32],
    removed_ids: &'a [u32],
}

impl<'a> NveSceneRef<'a> {
    /// Validates `view.schema_hash` and non-null pointer invariants, returning a zero-copy reader.
    ///
    /// # Safety
    /// Every non-empty pointer in `view` must remain valid and immutable for `'a` and cover at
    /// least its declared element count.
    pub unsafe fn from_raw(view: NveSceneView) -> Result<Self, SceneError> {
        if view.schema_hash != NVE_SCENE_SCHEMA_HASH {
            return Err(SceneError::SchemaHashMismatch {
                expected: NVE_SCENE_SCHEMA_HASH,
                actual: view.schema_hash,
            });
        }
        let nodes = if view.node_count == 0 {
            &[]
        } else {
            if view.nodes.is_null() {
                return Err(SceneError::NullBufferPointer("nodes"));
            }
            unsafe { std::slice::from_raw_parts(view.nodes, view.node_count as usize) }
        };
        let text_slab = if view.text_slab_len == 0 {
            &[]
        } else {
            if view.text_slab.is_null() {
                return Err(SceneError::NullBufferPointer("text_slab"));
            }
            unsafe { std::slice::from_raw_parts(view.text_slab, view.text_slab_len as usize) }
        };
        let children_slab = if view.children_slab_len == 0 {
            &[]
        } else {
            if view.children_slab.is_null() {
                return Err(SceneError::NullBufferPointer("children_slab"));
            }
            unsafe {
                std::slice::from_raw_parts(view.children_slab, view.children_slab_len as usize)
            }
        };
        let dirty_indices = if view.dirty_count == 0 {
            &[]
        } else {
            if view.dirty_indices.is_null() {
                return Err(SceneError::NullBufferPointer("dirty_indices"));
            }
            unsafe { std::slice::from_raw_parts(view.dirty_indices, view.dirty_count as usize) }
        };
        let removed_ids = if view.removed_count == 0 {
            &[]
        } else {
            if view.removed_ids.is_null() {
                return Err(SceneError::NullBufferPointer("removed_ids"));
            }
            unsafe { std::slice::from_raw_parts(view.removed_ids, view.removed_count as usize) }
        };

        Ok(Self {
            frame_id: view.frame_id,
            nodes,
            text_slab,
            children_slab,
            dirty_indices,
            removed_ids,
        })
    }

    #[inline]
    pub const fn frame_id(&self) -> FrameId {
        self.frame_id
    }

    #[inline]
    pub const fn nodes(&self) -> &'a [NveRenderNode] {
        self.nodes
    }

    #[inline]
    pub fn node(&self, index: usize) -> Option<&'a NveRenderNode> {
        self.nodes.get(index)
    }

    #[inline]
    pub const fn dirty_indices(&self) -> &'a [u32] {
        self.dirty_indices
    }

    #[inline]
    pub fn dirty_nodes(&self) -> impl Iterator<Item = (usize, &'a NveRenderNode)> + 'a {
        let nodes = self.nodes;
        self.dirty_indices.iter().filter_map(move |&idx| {
            let i = idx as usize;
            nodes.get(i).map(|n| (i, n))
        })
    }

    #[inline]
    pub const fn removed_ids(&self) -> &'a [u32] {
        self.removed_ids
    }

    #[inline]
    pub const fn text_slab(&self) -> &'a [u8] {
        self.text_slab
    }

    #[inline]
    pub const fn children_slab(&self) -> &'a [u32] {
        self.children_slab
    }

    #[inline]
    pub fn node_text_bytes(&self, node: &NveRenderNode) -> Result<&'a [u8], SceneError> {
        if node.text_len == 0 {
            return Ok(&[]);
        }
        let start = node.text_offset as usize;
        let end = start
            .checked_add(node.text_len as usize)
            .ok_or(SceneError::TextOutOfBounds {
                node_id: node.node_id,
                offset: node.text_offset,
                len: node.text_len,
                slab_len: self.text_slab.len(),
            })?;
        self.text_slab
            .get(start..end)
            .ok_or(SceneError::TextOutOfBounds {
                node_id: node.node_id,
                offset: node.text_offset,
                len: node.text_len,
                slab_len: self.text_slab.len(),
            })
    }

    #[inline]
    pub fn node_text(&self, node: &NveRenderNode) -> Result<&'a str, SceneError> {
        let bytes = self.node_text_bytes(node)?;
        std::str::from_utf8(bytes).map_err(|_| SceneError::InvalidUtf8 {
            node_id: node.node_id,
        })
    }

    #[inline]
    pub fn node_children(&self, node: &NveRenderNode) -> Result<&'a [u32], SceneError> {
        if node.child_count == 0 {
            return Ok(&[]);
        }
        let start = node.child_offset as usize;
        let end = start
            .checked_add(node.child_count as usize)
            .ok_or(SceneError::ChildrenOutOfBounds {
                node_id: node.node_id,
                offset: node.child_offset,
                count: node.child_count,
                slab_len: self.children_slab.len(),
            })?;
        self.children_slab
            .get(start..end)
            .ok_or(SceneError::ChildrenOutOfBounds {
                node_id: node.node_id,
                offset: node.child_offset,
                count: node.child_count,
                slab_len: self.children_slab.len(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn render_node_is_exactly_one_64_byte_cache_line() {
        assert_eq!(size_of::<NveRenderNode>(), 64);
        assert_eq!(align_of::<NveRenderNode>(), 64);
        assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
    }

    #[test]
    fn incremental_table_tracks_only_changed_and_removed_nodes() {
        let mut table = NveSceneTable::with_capacity(1000);
        table.begin_frame(1);
        for id in 1..=1000u32 {
            let label = if id % 2 == 0 { "even" } else { "odd" };
            let children: &[u32] = if id == 1 { &[2, 3, 4] } else { &[] };
            table
                .upsert_node(SceneNodeInput {
                    node_id: id,
                    kind: if id == 1 {
                        NVE_NODE_KIND_ROOT
                    } else if id % 3 == 0 {
                        NVE_NODE_KIND_PRESSABLE
                    } else {
                        NVE_NODE_KIND_TEXT
                    },
                    flags: if id % 3 == 0 {
                        NVE_NODE_INTERACTIVE
                    } else {
                        NVE_NODE_LEAF_TEXT
                    },
                    opacity_u8: 255,
                    x: id as f32,
                    y: (id * 2) as f32,
                    width: 120.0,
                    height: 36.0,
                    bg_rgba: 0xFF11_2233,
                    text_rgba: 0xFFFF_FFFF,
                    border_rgba: 0,
                    border_radius: 8.0,
                    border_width: 1.0,
                    font_size: 16.0,
                    z_index: 0,
                    text: label,
                    children,
                })
                .unwrap();
        }
        assert_eq!(table.len(), 1000);
        assert_eq!(table.as_ref().dirty_indices().len(), 1000);

        // Frame 2: touch only 3 nodes out of 1000.
        table.begin_frame(2);
        let slab_before = table.as_ref().text_slab().len();
        for &id in &[10u32, 42, 999] {
            table
                .upsert_node(SceneNodeInput {
                    node_id: id,
                    kind: NVE_NODE_KIND_TEXT,
                    flags: NVE_NODE_LEAF_TEXT,
                    opacity_u8: 200,
                    x: id as f32 + 5.0,
                    y: (id * 2) as f32,
                    width: 120.0,
                    height: 36.0,
                    bg_rgba: 0xFF11_2233,
                    text_rgba: 0xFFFF_FFFF,
                    border_rgba: 0,
                    border_radius: 8.0,
                    border_width: 1.0,
                    font_size: 16.0,
                    z_index: 1,
                    text: "updated",
                    children: &[],
                })
                .unwrap();
        }
        // Also upsert an identical node: must NOT be marked dirty or grow text_slab.
        table
            .upsert_node(SceneNodeInput {
                node_id: 2,
                kind: NVE_NODE_KIND_TEXT,
                flags: NVE_NODE_LEAF_TEXT,
                opacity_u8: 255,
                x: 2.0,
                y: 4.0,
                width: 120.0,
                height: 36.0,
                bg_rgba: 0xFF11_2233,
                text_rgba: 0xFFFF_FFFF,
                border_rgba: 0,
                border_radius: 8.0,
                border_width: 1.0,
                font_size: 16.0,
                z_index: 0,
                text: "even",
                children: &[],
            })
            .unwrap();

        let view = table.as_view();
        assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
        assert_eq!(view.frame_id, 2);
        assert_eq!(view.node_count, 1000);
        assert_eq!(view.dirty_count, 3);
        assert_eq!(view.removed_count, 0);
        assert_eq!(view.text_slab_len as usize, slab_before + 3 * "updated".len());

        let reader = unsafe { NveSceneRef::from_raw(view) }.expect("valid raw view");
        let dirty_ids: Vec<u32> = reader.dirty_nodes().map(|(_, n)| n.node_id).collect();
        assert_eq!(dirty_ids, vec![10, 42, 999]);
        for (_, node) in reader.dirty_nodes() {
            assert_eq!(reader.node_text(node).unwrap(), "updated");
            assert_eq!(node.opacity_u8, 200);
        }
        let root = reader.node(table.index_of(1).unwrap()).unwrap();
        assert_eq!(reader.node_children(root).unwrap(), &[2, 3, 4]);

        // Frame 3: remove nodes 10 and 1000 via swap-remove.
        table.begin_frame(3);
        assert!(table.remove_node(10));
        assert!(table.remove_node(1000));
        assert!(!table.remove_node(10));
        assert_eq!(table.len(), 998);
        let reader3 = table.as_ref();
        assert_eq!(reader3.removed_ids(), &[10, 1000]);
        assert!(table.index_of(10).is_none());
        assert!(table.index_of(1000).is_none());
        // Node 999 was swapped into slot of 10 (after 1000 moved into slot of 10, then 1000 removed).
        let idx999 = table.index_of(999).unwrap();
        assert_eq!(reader3.node(idx999).unwrap().node_id, 999);
        assert_eq!(reader3.node_text(reader3.node(idx999).unwrap()).unwrap(), "updated");
    }

    #[test]
    fn sync_frame_and_compaction_preserve_live_data() {
        let mut table = NveSceneTable::new();
        table
            .sync_frame(
                1,
                [
                    SceneNodeInput {
                        node_id: 1,
                        kind: NVE_NODE_KIND_ROOT,
                        children: &[2, 3],
                        ..Default::default()
                    },
                    SceneNodeInput {
                        node_id: 2,
                        kind: NVE_NODE_KIND_TEXT,
                        flags: NVE_NODE_LEAF_TEXT,
                        text: "hello",
                        ..Default::default()
                    },
                    SceneNodeInput {
                        node_id: 3,
                        kind: NVE_NODE_KIND_TEXT,
                        flags: NVE_NODE_LEAF_TEXT,
                        text: "world",
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
        assert_eq!(table.as_ref().dirty_indices().len(), 3);

        // Frame 2: remove node 3, update node 2 text.
        table
            .sync_frame(
                2,
                [
                    SceneNodeInput {
                        node_id: 1,
                        kind: NVE_NODE_KIND_ROOT,
                        children: &[2],
                        ..Default::default()
                    },
                    SceneNodeInput {
                        node_id: 2,
                        kind: NVE_NODE_KIND_TEXT,
                        flags: NVE_NODE_LEAF_TEXT,
                        text: "updated 🚀",
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.as_ref().removed_ids(), &[3]);

        table.compact_text_slab();
        table.compact_children_slab();
        let reader = table.as_ref();
        assert_eq!(reader.text_slab(), "updated 🚀".as_bytes());
        assert_eq!(reader.children_slab(), &[2]);
        let n2 = reader.node(table.index_of(2).unwrap()).unwrap();
        assert_eq!(reader.node_text(n2).unwrap(), "updated 🚀");
    }

    #[test]
    fn raw_scene_ref_rejects_corrupt_views() {
        let mut view = NveSceneTable::new().as_view();
        view.schema_hash ^= 1;
        assert_eq!(
            unsafe { NveSceneRef::from_raw(view) }.unwrap_err(),
            SceneError::SchemaHashMismatch {
                expected: NVE_SCENE_SCHEMA_HASH,
                actual: NVE_SCENE_SCHEMA_HASH ^ 1,
            }
        );

        let mut null_nodes = NveSceneTable::new().as_view();
        null_nodes.node_count = 1;
        assert_eq!(
            unsafe { NveSceneRef::from_raw(null_nodes) }.unwrap_err(),
            SceneError::NullBufferPointer("nodes")
        );

        // Out-of-bounds text & invalid UTF-8 checks.
        let mut node = NveRenderNode::zeroed();
        node.node_id = 7;
        node.text_offset = 0;
        node.text_len = 2;
        let bad_utf8: [u8; 2] = [0xFF, 0xFE];
        let valid_view = NveSceneView {
            schema_hash: NVE_SCENE_SCHEMA_HASH,
            frame_id: 1,
            node_count: 1,
            dirty_count: 0,
            removed_count: 0,
            text_slab_len: 2,
            children_slab_len: 0,
            _pad: 0,
            nodes: &node,
            text_slab: bad_utf8.as_ptr(),
            children_slab: std::ptr::null(),
            dirty_indices: std::ptr::null(),
            removed_ids: std::ptr::null(),
        };
        let reader = unsafe { NveSceneRef::from_raw(valid_view) }.unwrap();
        assert_eq!(
            reader.node_text(&reader.nodes()[0]).unwrap_err(),
            SceneError::InvalidUtf8 { node_id: 7 }
        );

        node.text_len = 10;
        let oob_view = NveSceneView {
            nodes: &node,
            ..valid_view
        };
        let oob_reader = unsafe { NveSceneRef::from_raw(oob_view) }.unwrap();
        assert_eq!(
            oob_reader.node_text(&oob_reader.nodes()[0]).unwrap_err(),
            SceneError::TextOutOfBounds {
                node_id: 7,
                offset: 0,
                len: 10,
                slab_len: 2,
            }
        );
    }

    #[test]
    fn checked_in_c_header_matches_compile_time_schema_hash() {
        let header_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cxx/nve-fabric-bridge/include/nve_scene_abi.h");
        let content = std::fs::read_to_string(&header_path)
            .expect("read generated cxx/nve-fabric-bridge/include/nve_scene_abi.h");
        let expected_macro = format!("0x{NVE_SCENE_SCHEMA_HASH:016X}ULL");
        assert!(
            content.contains(&expected_macro),
            "nve_scene_abi.h schema hash drift! Expected {expected_macro} in {}",
            header_path.display()
        );
    }
}
