//! Zero-allocation iterative traversal engine for [`super::CompositorScene`] plus flat
//! [`DisplayList`] materialization. Software and Skia Canvas sinks live in sibling modules
//! (`replay_software.rs`, `replay_skia.rs`).

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use super::chunks::LocalPaintOp;
use super::property_tree::ChunkIdx;
use super::CompositorScene;
use crate::display_list::{for_each_scrollbar_thumb, intersect_rects, intersects_clip};
use crate::{DisplayList, PaintOp};

const INLINE_REPLAY_DEPTH: usize = 64;
/// Translucent viewport backdrop scrim painted behind modal / `BACKDROP_SCRIM` top-layer overlays.
const OVERLAY_BACKDROP_SCRIM_COLOR: ColorRgba = ColorRgba::rgba(15, 23, 42, 140);

#[inline]
pub(super) fn translate_rect(local: LayoutRect, wx: f32, wy: f32) -> LayoutRect {
    LayoutRect {
        x: local.x + wx,
        y: local.y + wy,
        width: local.width,
        height: local.height,
    }
}

/// Snaps a world-space caret rect to physical device pixels and clamps horizontally inside
/// `[wx + min_local_x, wx + max_local_x - snapped_w]` (assuming axis-aligned ancestor transforms).
#[inline]
pub(crate) fn snap_world_rect(
    local: LayoutRect,
    min_local_x: f32,
    max_local_x: f32,
    wx: f32,
    wy: f32,
    scale: f32,
) -> LayoutRect {
    if !local.x.is_finite()
        || !local.y.is_finite()
        || !local.width.is_finite()
        || !local.height.is_finite()
        || !min_local_x.is_finite()
        || !max_local_x.is_finite()
        || !wx.is_finite()
        || !wy.is_finite()
    {
        return LayoutRect::default();
    }
    let s = crate::text_cache::sanitize_scale(scale);
    let raw_x = local.x + wx;
    let raw_y = local.y + wy;
    let snapped_w = (local.width.max(0.0) * s).round().max(1.0) / s;
    let min_x = wx + min_local_x;
    let max_x = (wx + max_local_x - snapped_w).max(min_x);
    let snapped_x = ((raw_x * s).round() / s).max(min_x).min(max_x);
    LayoutRect {
        x: snapped_x,
        y: (raw_y * s).round() / s,
        width: snapped_w,
        height: (local.height.max(0.0) * s).round().max(1.0) / s,
    }
}

#[derive(Clone, Copy)]
struct ReplayFrame {
    chunk_idx: ChunkIdx,
    child_cursor: u32,
    child_end: u32,
    world_rect: LayoutRect,
    child_offset: (f32, f32),
    child_clip: LayoutRect,
    in_clip: bool,
    any_tf: bool,
    pushed_opacity: bool,
    pushed_tf: bool,
    pushed_clip: bool,
}

impl Default for ReplayFrame {
    fn default() -> Self {
        Self {
            chunk_idx: ChunkIdx::NONE,
            child_cursor: 0,
            child_end: 0,
            world_rect: LayoutRect::default(),
            child_offset: (0.0, 0.0),
            child_clip: LayoutRect::default(),
            in_clip: false,
            any_tf: false,
            pushed_opacity: false,
            pushed_tf: false,
            pushed_clip: false,
        }
    }
}

struct ReplayStack {
    inline: [ReplayFrame; INLINE_REPLAY_DEPTH],
    len: usize,
    overflow: Vec<ReplayFrame>,
}

impl ReplayStack {
    #[inline]
    fn new() -> Self {
        Self {
            inline: [ReplayFrame::default(); INLINE_REPLAY_DEPTH],
            len: 0,
            overflow: Vec::new(),
        }
    }

    #[inline]
    fn push(&mut self, frame: ReplayFrame) {
        if self.len < INLINE_REPLAY_DEPTH {
            self.inline[self.len] = frame;
        } else {
            self.overflow.push(frame);
        }
        self.len += 1;
    }

    #[inline]
    fn top_mut(&mut self) -> Option<&mut ReplayFrame> {
        if self.len == 0 {
            None
        } else if self.len <= INLINE_REPLAY_DEPTH {
            Some(&mut self.inline[self.len - 1])
        } else {
            self.overflow.last_mut()
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<ReplayFrame> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        if self.len < INLINE_REPLAY_DEPTH {
            Some(self.inline[self.len])
        } else {
            self.overflow.pop()
        }
    }
}

pub(super) trait ReplaySink {
    fn push_opacity(&mut self, alpha: f32);
    fn pop_opacity(&mut self);
    fn push_transform(&mut self, matrix: [f32; 6]);
    fn pop_transform(&mut self);
    fn push_clip(&mut self, rect: LayoutRect, radius: f32);
    fn pop_clip(&mut self);
    fn paint_local_op(
        &mut self,
        scene: &CompositorScene,
        op: LocalPaintOp,
        wx: f32,
        wy: f32,
        width: f32,
        height: f32,
    );
    fn paint_scrollbar_thumb(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba);
}

pub(super) fn replay_scene_into<S: ReplaySink>(scene: &CompositorScene, sink: &mut S) {
    let Some(root_idx) = scene.root else {
        return;
    };
    let root_clip = LayoutRect {
        x: 0.0,
        y: 0.0,
        width: scene.viewport_width.max(1.0),
        height: scene.viewport_height.max(1.0),
    };
    let mut stack = ReplayStack::new();
    replay_subtree(scene, root_idx, root_clip, false, sink, &mut stack);
    // Replay deferred top-layer overlays after the document root pops all ancestor clips/transforms.
    for &tl_idx in &scene.props.top_layer_chunks {
        let Some(meta) = scene.chunks.get(tl_idx.index()) else {
            continue;
        };
        if !meta.is_live() {
            continue;
        }
        if scene.props.transforms[tl_idx.index()].has_backdrop_scrim() {
            sink.paint_scrollbar_thumb(root_clip, 0.0, OVERLAY_BACKDROP_SCRIM_COLOR);
        }
        replay_subtree(scene, tl_idx, root_clip, true, sink, &mut stack);
    }
}

fn replay_subtree<S: ReplaySink>(
    scene: &CompositorScene,
    start_idx: ChunkIdx,
    root_clip: LayoutRect,
    allow_start_top_layer: bool,
    sink: &mut S,
    stack: &mut ReplayStack,
) {
    enter_chunk(
        scene,
        start_idx,
        (0.0, 0.0),
        root_clip,
        false,
        allow_start_top_layer,
        sink,
        stack,
    );

    while let Some(top) = stack.top_mut() {
        if top.child_cursor < top.child_end {
            let child_idx = scene.arenas.ordered_children[top.child_cursor as usize];
            top.child_cursor += 1;
            let offset = top.child_offset;
            let clip = top.child_clip;
            let any_tf = top.any_tf;
            enter_chunk(scene, child_idx, offset, clip, any_tf, false, sink, stack);
        } else if let Some(done) = stack.pop() {
            if done.pushed_clip {
                sink.pop_clip();
            }
            let cidx = done.chunk_idx.index();
            let tf = &scene.props.transforms[cidx];
            let eff = &scene.props.effects[cidx];
            if !eff.effective_hidden
                && done.in_clip
                && tf.scroll_idx.is_some()
                && let Some(sc) = scene.props.scrolls.get(tf.scroll_idx.index())
            {
                for_each_scrollbar_thumb(
                    done.world_rect,
                    sc.scroll_raw_x,
                    sc.scroll_raw_y,
                    sc.content_w,
                    sc.content_h,
                    |r, rad, col| sink.paint_scrollbar_thumb(r, rad, col),
                );
            }
            if done.pushed_tf {
                sink.pop_transform();
            }
            if done.pushed_opacity {
                sink.pop_opacity();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn enter_chunk<S: ReplaySink>(
    scene: &CompositorScene,
    chunk_idx: ChunkIdx,
    parent_offset: (f32, f32),
    parent_clip: LayoutRect,
    in_transform: bool,
    allow_top_layer: bool,
    sink: &mut S,
    stack: &mut ReplayStack,
) {
    let Some(meta) = scene.chunks.get(chunk_idx.index()) else {
        return;
    };
    if !meta.is_live() {
        return;
    }
    let cidx = chunk_idx.index();
    let eff = scene.props.effects[cidx];
    if eff.opacity <= 0.001 {
        return;
    }
    let tf = scene.props.transforms[cidx];
    if tf.is_top_layer() && !allow_top_layer {
        return;
    }
    let clip = scene.props.clips[cidx];

    let node_offset = (parent_offset.0 + tf.sticky_x, parent_offset.1 + tf.sticky_y);
    let wx = tf.base_x + node_offset.0;
    let wy = tf.base_y + node_offset.1;
    let world_rect = LayoutRect {
        x: wx,
        y: wy,
        width: tf.width,
        height: tf.height,
    };

    let has_tf = tf.has_transform();
    let any_tf = in_transform || has_tf;
    let in_clip = any_tf || intersects_clip(parent_clip, world_rect, 1.0);

    if !any_tf && !in_clip && meta.shadow_len == 0 && (clip.clips_children || meta.children_len == 0)
    {
        return;
    }

    let has_content =
        meta.shadow_len > 0 || meta.op_len > 0 || meta.children_len > 0 || tf.has_scroll();
    if !has_content {
        return;
    }

    let pushed_opacity = eff.opacity < 0.999;
    if pushed_opacity {
        sink.push_opacity(eff.opacity);
    }
    if has_tf {
        sink.push_transform(
            tf.local_transform
                .to_surface_matrix_2d(wx, wy, tf.width, tf.height),
        );
    }

    if !eff.effective_hidden {
        for &sh_op in scene.arenas.shadow_slice(meta) {
            if let LocalPaintOp::DrawShadow {
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                ..
            } = sh_op
            {
                let shadow_bounds = LayoutRect {
                    x: wx + offset_x - spread_radius,
                    y: wy + offset_y - spread_radius,
                    width: tf.width + spread_radius * 2.0,
                    height: tf.height + spread_radius * 2.0,
                };
                if any_tf || intersects_clip(parent_clip, shadow_bounds, blur_radius * 1.5 + 1.0) {
                    sink.paint_local_op(scene, sh_op, wx, wy, tf.width, tf.height);
                }
            }
        }
        if in_clip {
            for &op in scene.arenas.op_slice(meta) {
                sink.paint_local_op(scene, op, wx, wy, tf.width, tf.height);
            }
        }
    }

    let (child_start, child_end, child_clip, pushed_clip) = if meta.children_len > 0 {
        let next_clip = if clip.clips_children && !any_tf {
            intersect_rects(parent_clip, world_rect)
        } else {
            Some(parent_clip)
        };
        if let Some(cc) = next_clip {
            if clip.clips_children {
                sink.push_clip(world_rect, clip.radius);
            }
            (
                meta.children_start,
                meta.children_start + meta.children_len,
                cc,
                clip.clips_children,
            )
        } else {
            (0, 0, parent_clip, false)
        }
    } else {
        (0, 0, parent_clip, false)
    };

    if child_start == child_end {
        if pushed_clip {
            sink.pop_clip();
        }
        if !eff.effective_hidden
            && in_clip
            && tf.scroll_idx.is_some()
            && let Some(sc) = scene.props.scrolls.get(tf.scroll_idx.index())
        {
            for_each_scrollbar_thumb(
                world_rect,
                sc.scroll_raw_x,
                sc.scroll_raw_y,
                sc.content_w,
                sc.content_h,
                |r, rad, col| sink.paint_scrollbar_thumb(r, rad, col),
            );
        }
        if has_tf {
            sink.pop_transform();
        }
        if pushed_opacity {
            sink.pop_opacity();
        }
        return;
    }

    let (scroll_shift_x, scroll_shift_y) = if tf.scroll_idx.is_some()
        && let Some(sc) = scene.props.scrolls.get(tf.scroll_idx.index())
    {
        (sc.scroll_shift_x, sc.scroll_shift_y)
    } else {
        (0.0, 0.0)
    };
    let child_offset = (node_offset.0 - scroll_shift_x, node_offset.1 - scroll_shift_y);
    stack.push(ReplayFrame {
        chunk_idx,
        child_cursor: child_start,
        child_end,
        world_rect,
        child_offset,
        child_clip,
        in_clip,
        any_tf,
        pushed_opacity,
        pushed_tf: has_tf,
        pushed_clip,
    });
}

struct DisplayListSink<'a> {
    ops: &'a mut Vec<PaintOp>,
    scale: f32,
    depth_markers: Vec<usize>,
}

impl ReplaySink for DisplayListSink<'_> {
    fn push_opacity(&mut self, alpha: f32) {
        self.depth_markers.push(self.ops.len());
        self.ops.push(PaintOp::SaveOpacity(alpha));
    }

    fn pop_opacity(&mut self) {
        if let Some(marker) = self.depth_markers.pop() {
            if self.ops.len() == marker + 1 {
                self.ops.pop();
            } else {
                self.ops.push(PaintOp::Restore);
            }
        }
    }

    fn push_transform(&mut self, matrix: [f32; 6]) {
        self.depth_markers.push(self.ops.len());
        self.ops.push(PaintOp::PushTransform { matrix });
    }

    fn pop_transform(&mut self) {
        if let Some(marker) = self.depth_markers.pop() {
            if self.ops.len() == marker + 1 {
                self.ops.pop();
            } else {
                self.ops.push(PaintOp::PopTransform);
            }
        }
    }

    fn push_clip(&mut self, rect: LayoutRect, radius: f32) {
        self.depth_markers.push(self.ops.len());
        self.ops.push(PaintOp::PushClip { rect, radius });
    }

    fn pop_clip(&mut self) {
        if let Some(marker) = self.depth_markers.pop() {
            if self.ops.len() == marker + 1 {
                self.ops.pop();
            } else {
                self.ops.push(PaintOp::PopClip);
            }
        }
    }

    fn paint_local_op(
        &mut self,
        scene: &CompositorScene,
        op: LocalPaintOp,
        wx: f32,
        wy: f32,
        width: f32,
        height: f32,
    ) {
        match op {
            LocalPaintOp::DrawShadow {
                radius,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                color,
                border_width,
                inset,
            } => self.ops.push(PaintOp::DrawShadow {
                rect: LayoutRect {
                    x: wx,
                    y: wy,
                    width,
                    height,
                },
                radius,
                offset_x,
                offset_y,
                blur_radius,
                spread_radius,
                color,
                border_width,
                inset,
            }),
            LocalPaintOp::FillRoundedRect {
                local_rect,
                radius,
                color,
            } => self.ops.push(PaintOp::FillRoundedRect {
                rect: translate_rect(local_rect, wx, wy),
                radius,
                color,
            }),
            LocalPaintOp::FillSnappedRect {
                local_rect,
                min_local_x,
                max_local_x,
                color,
            } => {
                self.ops.push(PaintOp::FillRoundedRect {
                    rect: snap_world_rect(
                        local_rect,
                        min_local_x,
                        max_local_x,
                        wx,
                        wy,
                        self.scale,
                    ),
                    radius: 0.0,
                    color,
                });
            }
            LocalPaintOp::FillRoundedRectGradient {
                local_rect,
                radius,
                gradient_idx,
            } => {
                if let Some(gradient) = scene.arenas.gradient(gradient_idx) {
                    self.ops.push(PaintOp::FillRoundedRectGradient {
                        rect: translate_rect(local_rect, wx, wy),
                        radius,
                        gradient: gradient.clone(),
                    });
                }
            }
            LocalPaintOp::StrokeRoundedRect {
                local_rect,
                radius,
                width: stroke_w,
                color,
            } => self.ops.push(PaintOp::StrokeRoundedRect {
                rect: translate_rect(local_rect, wx, wy),
                radius,
                width: stroke_w,
                color,
            }),
            LocalPaintOp::PushLocalClip { local_rect, radius } => {
                self.ops.push(PaintOp::PushClip {
                    rect: translate_rect(local_rect, wx, wy),
                    radius,
                });
            }
            LocalPaintOp::PopLocalClip => {
                self.ops.push(PaintOp::PopClip);
            }
            LocalPaintOp::DrawText {
                local_rect,
                str_off,
                str_len,
                font_size,
                font_weight,
                text_align,
                color,
            } => self.ops.push(PaintOp::DrawText {
                rect: translate_rect(local_rect, wx, wy),
                text: scene.arenas.str_slice(str_off, str_len).to_owned(),
                font_size,
                font_weight,
                text_align,
                color,
            }),
            LocalPaintOp::DrawImage {
                local_rect,
                radius,
                str_off,
                str_len,
                fit,
                handle_idx,
            } => self.ops.push(PaintOp::DrawImage {
                rect: translate_rect(local_rect, wx, wy),
                radius,
                src: scene.arenas.str_slice(str_off, str_len).to_owned(),
                fit,
                resolved: scene.arenas.image_handle(handle_idx),
            }),
            LocalPaintOp::Plugin {
                node_id,
                local_rect,
                inherited_text_color,
                plugin_epoch,
            } => self.ops.push(PaintOp::Plugin {
                node_id,
                rect: translate_rect(local_rect, wx, wy),
                inherited_text_color,
                plugin_epoch,
            }),
        }
    }

    fn paint_scrollbar_thumb(&mut self, rect: LayoutRect, radius: f32, color: ColorRgba) {
        self.ops.push(PaintOp::FillRoundedRect {
            rect,
            radius,
            color,
        });
    }
}

/// Flattens a retained [`CompositorScene`] into a [`DisplayList`] (used for inspection & tests).
pub fn materialize_scene_display_list(
    scene: &CompositorScene,
    scale_factor: f32,
    out: &mut DisplayList,
) {
    out.ops.clear();
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let mut sink = DisplayListSink {
        ops: &mut out.ops,
        scale,
        depth_markers: Vec::with_capacity(16),
    };
    replay_scene_into(scene, &mut sink);
}
