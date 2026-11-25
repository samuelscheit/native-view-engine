use std::collections::HashMap;

use crate::{FrameId, NodeId};

use super::raw_view::NveSceneRef;
use super::types::{
    NveRenderNode, NveSceneView, SceneError, SceneNodeInput, NVE_SCENE_SCHEMA_HASH,
};

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
