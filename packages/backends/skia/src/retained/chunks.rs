//! Node-local paint op chunks and flat string/gradient/child arenas with amortized compaction.
//!
//! All [`LocalPaintOp`] rects are stored in node-local coordinates relative to `(0, 0)` of the
//! node's own `base_rect`, so scrolling ancestors, moving siblings, or animating transforms and
//! opacity never re-emit any paint op.

use std::cell::Cell;

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;
use nve_store::{NodeId, Rgba};
use nve_tables::paint::BackgroundFill;
use nve_tables::tables::ImageFit;

use super::property_tree::ChunkIdx;
use crate::image_cache::ImageSlotHandle;
use crate::text::TextAlign;

pub(crate) const CHUNK_LIVE: u16 = 1 << 0;

/// Resolved typography fields cached per chunk for inherited-typography stop-pruning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedChunkText {
    pub font_weight: u16,
    pub text_align: TextAlign,
    pub text_color: ColorRgba,
    pub font_size: f32,
}

impl Default for ResolvedChunkText {
    fn default() -> Self {
        Self {
            font_weight: 400,
            text_align: TextAlign::Left,
            text_color: ColorRgba::BLACK,
            font_size: 16.0,
        }
    }
}

/// Strictly `Copy` node-local paint op (all coordinates relative to `(0, 0)` of the owning node).
///
/// Variable-length strings and gradients live in [`ChunkArenas`], so chunks contain zero heap
/// pointers or per-op allocations.
#[derive(Debug, Clone, Copy)]
pub enum LocalPaintOp {
    DrawShadow {
        radius: f32,
        offset_x: f32,
        offset_y: f32,
        blur_radius: f32,
        spread_radius: f32,
        color: ColorRgba,
        border_width: f32,
        inset: bool,
    },
    FillRoundedRect {
        local_rect: LayoutRect,
        radius: f32,
        color: ColorRgba,
    },
    /// Caret rectangle whose translated world coordinates snap to the physical pixel grid at
    /// replay time (`(v * scale).round() / scale`), clamped horizontally inside `[min_local_x,
    /// max_local_x]` so snapped carets never overhang their clip box.
    FillSnappedRect {
        local_rect: LayoutRect,
        min_local_x: f32,
        max_local_x: f32,
        color: ColorRgba,
    },
    FillRoundedRectGradient {
        local_rect: LayoutRect,
        radius: f32,
        gradient_idx: u32,
    },
    StrokeRoundedRect {
        local_rect: LayoutRect,
        radius: f32,
        width: f32,
        color: ColorRgba,
    },
    PushLocalClip {
        local_rect: LayoutRect,
        radius: f32,
    },
    PopLocalClip,
    DrawText {
        local_rect: LayoutRect,
        str_off: u32,
        str_len: u32,
        font_size: f32,
        font_weight: u16,
        text_align: TextAlign,
        color: ColorRgba,
    },
    DrawImage {
        local_rect: LayoutRect,
        radius: f32,
        str_off: u32,
        str_len: u32,
        fit: ImageFit,
        /// Index into [`ChunkArenas::image_handles`] (`Cell<Option<ImageSlotHandle>>`), enabling
        /// lock-free warm fast-path hits AND 1-frame self-healing backfill after cold decode or
        /// eviction while keeping `LocalPaintOp: Copy` and replay on `&CompositorScene`.
        handle_idx: u32,
    },
    Plugin {
        node_id: NodeId,
        local_rect: LayoutRect,
        inherited_text_color: Rgba,
        plugin_epoch: u64,
    },
}

/// Per-node retained chunk header (strictly `Copy` POD, zero heap fields).
///
/// Parallel transform, clip, and effect nodes live at the same [`ChunkIdx`] in `PropertyTrees`.
#[derive(Debug, Clone, Copy)]
pub struct NodeChunkMeta {
    pub node_id: NodeId,
    pub parent_chunk: ChunkIdx,
    pub shadow_start: u32,
    pub shadow_len: u32,
    pub op_start: u32,
    pub op_len: u32,
    pub children_start: u32,
    pub children_len: u32,
    pub z_index: i32,
    /// DOM child index inside `parent_chunk`, used as the stable tie-breaker for zero-allocation
    /// in-place z-order sorting (`sort_unstable_by_key((z_index, dom_order))`).
    pub dom_order: u32,
    pub plugin_epoch: u64,
    pub child_inherit_hash: u64,
    pub resolved_text: ResolvedChunkText,
    pub flags: u16,
}

impl Default for NodeChunkMeta {
    fn default() -> Self {
        Self {
            node_id: NodeId::NULL,
            parent_chunk: ChunkIdx::NONE,
            shadow_start: 0,
            shadow_len: 0,
            op_start: 0,
            op_len: 0,
            children_start: 0,
            children_len: 0,
            z_index: 0,
            dom_order: 0,
            plugin_epoch: 0,
            child_inherit_hash: 0,
            resolved_text: ResolvedChunkText::default(),
            flags: 0,
        }
    }
}

impl NodeChunkMeta {
    #[inline]
    pub const fn is_live(&self) -> bool {
        (self.flags & CHUNK_LIVE) != 0
    }
}

const _: () = assert!(std::mem::size_of::<LocalPaintOp>() <= 44);
const _: () = assert!(std::mem::size_of::<NodeChunkMeta>() <= 80);

/// Contiguous backing arenas for all retained node-local paint ops, strings, gradients,
/// self-healing image slot handles, and z-sorted child spans, with amortized dead-space compaction.
#[derive(Debug, Clone, Default)]
pub struct ChunkArenas {
    pub ops: Vec<LocalPaintOp>,
    pub str_arena: String,
    pub grad_arena: Vec<Option<BackgroundFill>>,
    pub image_handles: Vec<Cell<Option<ImageSlotHandle>>>,
    pub ordered_children: Vec<ChunkIdx>,
    pub dead_ops: usize,
    pub dead_str_bytes: usize,
    pub dead_grads: usize,
    pub dead_image_handles: usize,
    pub live_op_count: usize,
    pub compactions: u64,
}

impl ChunkArenas {
    pub fn clear(&mut self) {
        self.ops.clear();
        self.str_arena.clear();
        self.grad_arena.clear();
        self.image_handles.clear();
        self.ordered_children.clear();
        self.dead_ops = 0;
        self.dead_str_bytes = 0;
        self.dead_grads = 0;
        self.dead_image_handles = 0;
        self.live_op_count = 0;
    }

    /// Resets `ordered_children` before a full-tree sync rebuilds every parent's child span,
    /// keeping `ordered_children.len()` exact with zero dead spans across `LAYOUT` commits.
    #[inline]
    pub fn reset_children_arena(&mut self) {
        self.ordered_children.clear();
    }

    #[inline]
    pub fn push_str(&mut self, s: &str) -> (u32, u32) {
        let off = self.str_arena.len() as u32;
        self.str_arena.push_str(s);
        (off, s.len() as u32)
    }

    #[inline]
    pub fn str_slice(&self, off: u32, len: u32) -> &str {
        let start = off as usize;
        let end = start.saturating_add(len as usize);
        self.str_arena.get(start..end).unwrap_or("")
    }

    #[inline]
    pub fn push_gradient(&mut self, fill: BackgroundFill) -> u32 {
        let idx = self.grad_arena.len() as u32;
        self.grad_arena.push(Some(fill));
        idx
    }

    #[inline]
    pub fn gradient(&self, idx: u32) -> Option<&BackgroundFill> {
        self.grad_arena.get(idx as usize).and_then(Option::as_ref)
    }

    #[inline]
    pub fn push_image_handle(&mut self, handle: Option<ImageSlotHandle>) -> u32 {
        let idx = self.image_handles.len() as u32;
        self.image_handles.push(Cell::new(handle));
        idx
    }

    #[inline]
    pub fn image_handle(&self, idx: u32) -> Option<ImageSlotHandle> {
        self.image_handles.get(idx as usize).and_then(Cell::get)
    }

    #[inline]
    pub fn set_image_handle(&self, idx: u32, handle: Option<ImageSlotHandle>) {
        if let Some(cell) = self.image_handles.get(idx as usize) {
            cell.set(handle);
        }
    }

    #[inline]
    pub fn shadow_slice(&self, meta: &NodeChunkMeta) -> &[LocalPaintOp] {
        let start = meta.shadow_start as usize;
        let end = start.saturating_add(meta.shadow_len as usize);
        self.ops.get(start..end).unwrap_or(&[])
    }

    #[inline]
    pub fn op_slice(&self, meta: &NodeChunkMeta) -> &[LocalPaintOp] {
        let start = meta.op_start as usize;
        let end = start.saturating_add(meta.op_len as usize);
        self.ops.get(start..end).unwrap_or(&[])
    }

    #[inline]
    pub fn children_slice(&self, meta: &NodeChunkMeta) -> &[ChunkIdx] {
        let start = meta.children_start as usize;
        let end = start.saturating_add(meta.children_len as usize);
        self.ordered_children.get(start..end).unwrap_or(&[])
    }

    /// Accounts for the ops, strings, and gradients freed when replacing or deleting a chunk's ops.
    pub fn retire_chunk_ops(&mut self, meta: &mut NodeChunkMeta) {
        let op_range = meta.op_start as usize
            ..(meta.op_start as usize).saturating_add(meta.op_len as usize);
        let total_ops = (meta.shadow_len as usize) + (meta.op_len as usize);
        if total_ops > 0 {
            if let Some(slice) = self.ops.get(op_range) {
                for op in slice {
                    match *op {
                        LocalPaintOp::DrawText { str_len, .. } => {
                            self.dead_str_bytes += str_len as usize;
                        }
                        LocalPaintOp::DrawImage { str_len, .. } => {
                            self.dead_str_bytes += str_len as usize;
                            self.dead_image_handles += 1;
                        }
                        LocalPaintOp::FillRoundedRectGradient { gradient_idx, .. } => {
                            if let Some(slot) = self.grad_arena.get_mut(gradient_idx as usize) {
                                *slot = None;
                            }
                            self.dead_grads += 1;
                        }
                        _ => {}
                    }
                }
            }
            self.dead_ops += total_ops;
            self.live_op_count = self.live_op_count.saturating_sub(total_ops);
        }
        meta.shadow_start = 0;
        meta.shadow_len = 0;
        meta.op_start = 0;
        meta.op_len = 0;
    }

    /// Compacts dead spans in-place once dead usage exceeds 50% and a minimum floor, keeping
    /// arena memory strictly bounded under sustained paint/text churn. Moves gradients via
    /// `Option::take` so stop `Vec`s are never cloned during compaction.
    pub fn maybe_compact(&mut self, chunks: &mut [NodeChunkMeta]) {
        let compact_ops = self.dead_ops >= 256 && self.dead_ops * 2 >= self.ops.len();
        let compact_str =
            self.dead_str_bytes >= 4096 && self.dead_str_bytes * 2 >= self.str_arena.len();
        let compact_grad = self.dead_grads >= 64 && self.dead_grads * 2 >= self.grad_arena.len();
        let compact_img_handles = self.dead_image_handles >= 64
            && self.dead_image_handles * 2 >= self.image_handles.len();

        if !(compact_ops || compact_str || compact_grad || compact_img_handles) {
            return;
        }
        self.compactions = self.compactions.wrapping_add(1);

        let mut new_ops = Vec::with_capacity(self.live_op_count);
            let mut new_str =
                String::with_capacity(self.str_arena.len().saturating_sub(self.dead_str_bytes));
            let mut new_grads =
                Vec::with_capacity(self.grad_arena.len().saturating_sub(self.dead_grads));
            let mut new_image_handles = Vec::with_capacity(
                self.image_handles
                    .len()
                    .saturating_sub(self.dead_image_handles),
            );

            for meta in chunks.iter_mut() {
                if !meta.is_live() {
                    meta.shadow_start = 0;
                    meta.shadow_len = 0;
                    meta.op_start = 0;
                    meta.op_len = 0;
                    continue;
                }
                let sh_start = new_ops.len() as u32;
                let old_sh_s = meta.shadow_start as usize;
                let old_sh_e = old_sh_s.saturating_add(meta.shadow_len as usize);
                if let Some(slice) = self.ops.get(old_sh_s..old_sh_e) {
                    new_ops.extend_from_slice(slice);
                }
                meta.shadow_start = sh_start;

                let op_start = new_ops.len() as u32;
                let old_op_s = meta.op_start as usize;
                let old_op_e = old_op_s.saturating_add(meta.op_len as usize);
                if let Some(slice) = self.ops.get(old_op_s..old_op_e) {
                    for &op in slice {
                        let rewritten = match op {
                            LocalPaintOp::DrawText {
                                local_rect,
                                str_off,
                                str_len,
                                font_size,
                                font_weight,
                                text_align,
                                color,
                            } => {
                                let text = self.str_slice(str_off, str_len);
                                let off = new_str.len() as u32;
                                new_str.push_str(text);
                                LocalPaintOp::DrawText {
                                    local_rect,
                                    str_off: off,
                                    str_len: text.len() as u32,
                                    font_size,
                                    font_weight,
                                    text_align,
                                    color,
                                }
                            }
                            LocalPaintOp::DrawImage {
                                local_rect,
                                radius,
                                str_off,
                                str_len,
                                fit,
                                handle_idx,
                            } => {
                                let src = self.str_slice(str_off, str_len);
                                let off = new_str.len() as u32;
                                new_str.push_str(src);
                                let next_hidx = new_image_handles.len() as u32;
                                new_image_handles.push(Cell::new(self.image_handle(handle_idx)));
                                LocalPaintOp::DrawImage {
                                    local_rect,
                                    radius,
                                    str_off: off,
                                    str_len: src.len() as u32,
                                    fit,
                                    handle_idx: next_hidx,
                                }
                            }
                            LocalPaintOp::FillRoundedRectGradient {
                                local_rect,
                                radius,
                                gradient_idx,
                            } => {
                                let next_idx = if let Some(slot) =
                                    self.grad_arena.get_mut(gradient_idx as usize)
                                    && let Some(g) = slot.take()
                                {
                                    let idx = new_grads.len() as u32;
                                    new_grads.push(Some(g));
                                    idx
                                } else {
                                    u32::MAX
                                };
                                LocalPaintOp::FillRoundedRectGradient {
                                    local_rect,
                                    radius,
                                    gradient_idx: next_idx,
                                }
                            }
                            other => other,
                        };
                        new_ops.push(rewritten);
                    }
                }
                meta.op_start = op_start;
            }

        self.ops = new_ops;
        self.str_arena = new_str;
        self.grad_arena = new_grads;
        self.image_handles = new_image_handles;
        self.dead_ops = 0;
        self.dead_str_bytes = 0;
        self.dead_grads = 0;
        self.dead_image_handles = 0;
        self.live_op_count = self.ops.len();
    }
}
