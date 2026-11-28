//! Local-space `TextInput` emission, generation-checked chunk allocation, and zero-allocation
//! in-place z-order sorting helpers for [`super::emit`] and [`super::sync`].

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;
use nve_layout::StoreLayoutEngine;
use nve_store::{NodeId, NodeStore};
use nve_tables::tables::TextInput;

use super::chunks::{ChunkArenas, LocalPaintOp, NodeChunkMeta};
use super::emit::SceneSyncEnv;
use super::property_tree::{ChunkIdx, ClipNode, EffectNode, PropertyTrees, TransformNode};
use crate::text::TextAlign;
use crate::text_grapheme::{clamp_grapheme_boundary, grapheme_count};
use crate::text_input::{
    compute_caret_raw_rect, compute_text_input_inner_rect, compute_text_input_scroll_x,
    is_secure_node, measure_caret_advance, raw_text_input_value, selection_rects_for_range,
};
use crate::{FOCUS_RING_COLOR, FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR};

impl<L: StoreLayoutEngine> SceneSyncEnv<'_, L> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_text_input_local_ops(
        &self,
        id: NodeId,
        local_rect: LayoutRect,
        radius: f32,
        text_color: ColorRgba,
        font_size: f32,
        font_weight: u16,
        arenas: &mut ChunkArenas,
    ) {
        let is_focused = self.focus.focused_node == Some(id);
        let fs = if font_size > 0.0 { font_size } else { 14.0 };
        let text_rect = compute_text_input_inner_rect(self.store, id, local_rect, fs);
        let is_secure = is_secure_node(self.store, id);

        let committed = if is_focused {
            self.focus
                .uncontrolled_value
                .as_deref()
                .unwrap_or_else(|| raw_text_input_value(self.store, id))
        } else {
            raw_text_input_value(self.store, id)
        };
        let preedit = if is_focused {
            self.focus.ime_preedit.as_deref().filter(|p| !p.is_empty())
        } else {
            None
        };

        if is_focused {
            arenas.ops.push(LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: 2.0,
                color: FOCUS_RING_COLOR,
            });
        }

        let placeholder = self
            .store
            .try_row::<TextInput>(id)
            .map(|t| t.placeholder())
            .unwrap_or("");
        let placeholder_color = ColorRgba::rgba(148, 163, 184, 180);

        let caret_byte = clamp_grapheme_boundary(committed, self.focus.caret_byte_index);
        let scroll_x = if is_focused {
            compute_text_input_scroll_x(
                self.text_engine,
                committed,
                caret_byte,
                fs,
                font_weight,
                is_secure,
                text_rect.width,
            )
        } else {
            0.0
        };

        arenas.ops.push(LocalPaintOp::PushLocalClip {
            local_rect: text_rect,
            radius: 0.0,
        });

        if is_focused
            && preedit.is_none()
            && let Some((sel_start, sel_end)) = self.focus.selection_range()
        {
            for sel_rect in selection_rects_for_range(
                self.text_engine,
                committed,
                sel_start,
                sel_end,
                fs,
                font_weight,
                is_secure,
                text_rect,
                scroll_x,
            ) {
                arenas.ops.push(LocalPaintOp::FillRoundedRect {
                    local_rect: sel_rect,
                    radius: 2.0,
                    color: FOCUS_SELECTION_COLOR,
                });
            }
        }

        if committed.is_empty() && preedit.is_none() {
            if !placeholder.is_empty() {
                let (str_off, str_len) = arenas.push_str(placeholder);
                arenas.ops.push(LocalPaintOp::DrawText {
                    local_rect: text_rect,
                    str_off,
                    str_len,
                    font_size: fs,
                    font_weight,
                    text_align: TextAlign::Left,
                    color: placeholder_color,
                });
            }
        } else {
            let (str_off, str_len) = if is_secure {
                let mut s = "•".repeat(grapheme_count(committed));
                if let Some(p) = preedit {
                    s.push_str(p);
                }
                arenas.push_str(&s)
            } else if let Some(p) = preedit {
                let off = arenas.str_arena.len() as u32;
                arenas.str_arena.push_str(&committed[..caret_byte]);
                arenas.str_arena.push_str(p);
                arenas.str_arena.push_str(&committed[caret_byte..]);
                (off, (committed.len() + p.len()) as u32)
            } else {
                arenas.push_str(committed)
            };
            arenas.ops.push(LocalPaintOp::DrawText {
                local_rect: LayoutRect {
                    x: text_rect.x - scroll_x,
                    y: text_rect.y,
                    width: text_rect.width + scroll_x,
                    height: text_rect.height,
                },
                str_off,
                str_len,
                font_size: fs,
                font_weight,
                text_align: TextAlign::Left,
                color: text_color,
            });

            if let Some(p) = preedit {
                let x0 =
                    measure_caret_advance(self.text_engine, committed, caret_byte, fs, font_weight);
                let mut combined = String::with_capacity(committed.len() + p.len());
                combined.push_str(&committed[..caret_byte]);
                combined.push_str(p);
                let x1 = measure_caret_advance(
                    self.text_engine,
                    &combined,
                    combined.len(),
                    fs,
                    font_weight,
                );
                let underline_w = (x1 - x0).max(4.0);
                arenas.ops.push(LocalPaintOp::FillRoundedRect {
                    local_rect: LayoutRect {
                        x: text_rect.x + x0 - scroll_x,
                        y: text_rect.y + (fs * 1.25).min(text_rect.height - 2.0),
                        width: underline_w,
                        height: 2.0,
                    },
                    radius: 0.0,
                    color: IME_PREEDIT_UNDERLINE_COLOR,
                });
            }
        }

        if is_focused {
            let (caret_local, has_preedit) = compute_caret_raw_rect(
                self.text_engine,
                committed,
                caret_byte,
                preedit,
                fs,
                font_weight,
                is_secure,
                text_rect,
                scroll_x,
            );
            if has_preedit {
                arenas.ops.push(LocalPaintOp::FillRoundedRect {
                    local_rect: caret_local,
                    radius: 0.0,
                    color: FOCUS_RING_COLOR,
                });
            } else {
                arenas.ops.push(LocalPaintOp::FillSnappedRect {
                    local_rect: caret_local,
                    min_local_x: text_rect.x,
                    max_local_x: text_rect.x + text_rect.width,
                    color: FOCUS_RING_COLOR,
                });
            }
        }

        arenas.ops.push(LocalPaintOp::PopLocalClip);
    }
}

/// Zero-allocation in-place z-order sort of a parent chunk's existing `ordered_children` span.
/// Uses `(z_index, dom_order)` as a strict total order so `sort_unstable_by_key` allocates zero
/// bytes while preserving exact DOM tie-breaking order.
#[inline]
pub(crate) fn sort_ordered_children_in_place(
    parent_idx: ChunkIdx,
    chunks: &[NodeChunkMeta],
    ordered_children: &mut [ChunkIdx],
) {
    let parent_meta = &chunks[parent_idx.index()];
    if parent_meta.children_len <= 1 {
        return;
    }
    let start = parent_meta.children_start as usize;
    let end = start.saturating_add(parent_meta.children_len as usize);
    if let Some(span) = ordered_children.get_mut(start..end) {
        span.sort_unstable_by_key(|&cidx| {
            let m = &chunks[cidx.index()];
            (m.z_index, m.dom_order)
        });
    }
}

/// Populates a fresh `ordered_children` span for `parent_idx` during `sync_full_tree`, stamping
/// each live child's `dom_order` and sorting in-place when any child carries a non-zero `z_index`.
pub(crate) fn rebuild_ordered_children_for(
    store: &NodeStore,
    parent_id: NodeId,
    parent_idx: ChunkIdx,
    node_to_chunk: &[ChunkIdx],
    chunks: &mut [NodeChunkMeta],
    arenas: &mut ChunkArenas,
) {
    let raw_children = store.children(parent_id);
    if raw_children.is_empty() {
        chunks[parent_idx.index()].children_start = 0;
        chunks[parent_idx.index()].children_len = 0;
        return;
    }

    let start = arenas.ordered_children.len() as u32;
    let mut has_z = false;
    let mut dom_order = 0u32;
    for &cid in raw_children {
        if let Some(&cidx) = node_to_chunk.get(cid.index())
            && cidx.is_some()
        {
            let m = &mut chunks[cidx.index()];
            if m.is_live() && m.node_id == cid {
                m.dom_order = dom_order;
                dom_order += 1;
                if m.z_index != 0 {
                    has_z = true;
                }
                arenas.ordered_children.push(cidx);
            }
        }
    }
    let len = (arenas.ordered_children.len() as u32) - start;
    chunks[parent_idx.index()].children_start = start;
    chunks[parent_idx.index()].children_len = len;

    if has_z && len > 1 {
        sort_ordered_children_in_place(parent_idx, chunks, &mut arenas.ordered_children);
    }
}

/// Ensures `node_to_chunk` is sized for `id` and returns or allocates a [`ChunkIdx`].
///
/// Enforces generation equality (`meta.node_id == id`): if `NodeStore` recycled `id.index()`
/// with a newer generation before the previous chunk was retired, the old chunk's ops, child
/// span, and property slots are retired and reset immediately, returning `(existing, true)`.
pub(crate) fn get_or_alloc_chunk(
    id: NodeId,
    parent_chunk: ChunkIdx,
    node_to_chunk: &mut Vec<ChunkIdx>,
    chunks: &mut Vec<NodeChunkMeta>,
    free_chunks: &mut Vec<ChunkIdx>,
    props: &mut PropertyTrees,
    arenas: &mut ChunkArenas,
) -> (ChunkIdx, bool) {
    let idx = id.index();
    if idx >= node_to_chunk.len() {
        node_to_chunk.resize(idx + 1, ChunkIdx::NONE);
    }
    let existing = node_to_chunk[idx];
    if existing.is_some() {
        let meta = &mut chunks[existing.index()];
        if meta.node_id == id {
            meta.parent_chunk = parent_chunk;
            return (existing, false);
        }
        // ABA recycled index: retire old generation's ops/children and reset property slot.
        arenas.retire_chunk_ops(meta);
        arenas.retire_children_span(meta);
        *meta = NodeChunkMeta {
            node_id: id,
            parent_chunk,
            ..NodeChunkMeta::default()
        };
        props.reset_slot(existing);
        return (existing, true);
    }
    let cidx = if let Some(reused) = free_chunks.pop() {
        chunks[reused.index()] = NodeChunkMeta {
            node_id: id,
            parent_chunk,
            ..NodeChunkMeta::default()
        };
        props.reset_slot(reused);
        reused
    } else {
        props.push_slot(
            TransformNode::default(),
            ClipNode::default(),
            EffectNode::default(),
        );
        let cidx = ChunkIdx(chunks.len() as u32);
        chunks.push(NodeChunkMeta {
            node_id: id,
            parent_chunk,
            ..NodeChunkMeta::default()
        });
        cidx
    };
    node_to_chunk[idx] = cidx;
    (cidx, true)
}
