use crate::FrameId;

use super::types::{NveRenderNode, NveSceneView, SceneError, NVE_SCENE_SCHEMA_HASH};

/// Safe, zero-copy borrowed reader over an [`NveSceneView`].
#[derive(Debug, Clone, Copy)]
pub struct NveSceneRef<'a> {
    pub(super) frame_id: FrameId,
    pub(super) nodes: &'a [NveRenderNode],
    pub(super) text_slab: &'a [u8],
    pub(super) children_slab: &'a [u32],
    pub(super) dirty_indices: &'a [u32],
    pub(super) removed_ids: &'a [u32],
    pub(super) tables: &'a [crate::table::NveTableView],
}

/// Validates non-null, alignment, and `isize::MAX` byte-length invariants before forming a slice.
///
/// # Safety
/// When `len > 0`, `ptr` must point to `len` initialized, immutable `T` elements for `'a`.
#[inline]
unsafe fn guarded_slice<'a, T>(
    ptr: *const T,
    len: u32,
    field: &'static str,
) -> Result<&'a [T], SceneError> {
    let count = len as usize;
    if count == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(SceneError::NullBufferPointer(field));
    }
    if !(ptr as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return Err(SceneError::UnalignedBufferPointer(field));
    }
    let byte_len = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or(SceneError::BufferLengthOverflow(field))?;
    if byte_len > isize::MAX as usize {
        return Err(SceneError::BufferLengthOverflow(field));
    }
    Ok(unsafe { std::slice::from_raw_parts(ptr, count) })
}

impl<'a> NveSceneRef<'a> {
    /// Validates `view.schema_hash`, alignment, byte-length bounds, and non-null pointer
    /// invariants, returning a zero-copy reader.
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
        let nodes = unsafe { guarded_slice(view.nodes, view.node_count, "nodes")? };
        let text_slab = unsafe { guarded_slice(view.text_slab, view.text_slab_len, "text_slab")? };
        let children_slab =
            unsafe { guarded_slice(view.children_slab, view.children_slab_len, "children_slab")? };
        let dirty_indices =
            unsafe { guarded_slice(view.dirty_indices, view.dirty_count, "dirty_indices")? };
        let removed_ids =
            unsafe { guarded_slice(view.removed_ids, view.removed_count, "removed_ids")? };
        let tables = unsafe { guarded_slice(view.tables, view.table_count, "tables")? };

        Ok(Self {
            frame_id: view.frame_id,
            nodes,
            text_slab,
            children_slab,
            dirty_indices,
            removed_ids,
            tables,
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
    pub const fn tables(&self) -> &'a [crate::table::NveTableView] {
        self.tables
    }

    #[inline]
    pub fn table_by_key(&self, key: crate::Key64) -> Option<&'a crate::table::NveTableView> {
        let (lo, hi) = (key.lo(), key.hi());
        self.tables
            .iter()
            .find(|t| t.key_lo == lo && t.key_hi == hi)
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
