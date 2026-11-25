//! Zero-copy `NVERELOC` Frame-0 scene export and direct pre-rendered painting for [`SkiaBackend`].
//!
//! Compiles a [`DisplayList`] straight from a mapped [`SceneRelocView`] with zero Taffy layout
//! recomputation so native shells can paint Frame 0 in microseconds before the live `VirtualDom`
//! mounts and hands off cleanly on the first live `submit_frame`.

use nve_abi::{ColorRgba, ComponentKind, FrameId};
use nve_components::canvas_ir::{DrawListDecoder, ImageResizeMode};
use nve_engine::{
    BackendError, LayoutEngine, LayoutRect, RelocBuf, RelocError, SceneRelocView, VerifyMode,
};

use crate::display_list::{intersect_rects, intersects_clip, InheritedTextStyle};
use crate::raster::parse_box_shadow;
use crate::text::TextAlign;
use crate::{PaintOp, SkiaBackend, TextFocusState};

#[inline]
fn decode_text_align(raw: u8) -> TextAlign {
    match raw {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

impl<L: LayoutEngine> SkiaBackend<L> {
    /// Exports the current laid-out tree as a validated `NVERELOC` Frame-0 scene (`RelocBuf`).
    pub fn export_scene_reloc(
        &self,
        schema_version: u32,
        frame_id: FrameId,
        app_tag: &str,
        clear_color: ColorRgba,
    ) -> Result<RelocBuf, RelocError> {
        self.layout_tree
            .export_scene_reloc(schema_version, frame_id, app_tag, clear_color)
    }

    /// Validates `bytes` according to `mode`, compiles the pre-rendered [`DisplayList`] directly
    /// from [`SceneRelocView`] (zero layout recomputation), and hydrates the retained tree for
    /// instant hit-testing and seamless live handoff.
    pub fn load_reloc_scene(
        &mut self,
        bytes: &[u8],
        mode: VerifyMode,
    ) -> Result<usize, BackendError> {
        let view = SceneRelocView::from_bytes(bytes, mode)
            .map_err(|e| BackendError::Internal(e.to_string()))?;
        self.render_reloc_scene(view)
    }

    /// Renders a pre-validated [`SceneRelocView`] into this backend's [`DisplayList`] and hydrates
    /// the underlying layout tree for zero-jump live handoff.
    pub fn render_reloc_scene(&mut self, view: SceneRelocView<'_>) -> Result<usize, BackendError> {
        self.viewport = view.viewport();
        self.scroll_offsets.clear();
        self.last_virtual_scroll_y.clear();
        self.focus = TextFocusState::default();

        let count = self.compile_reloc_display_list(view)?;
        self.layout_tree.load_reloc_scene(view)?;
        Ok(count)
    }

    /// Compiles `self.display_list` directly from a zero-copy [`SceneRelocView`] with zero
    /// Taffy layout work and zero retained node-tree allocations.
    pub fn compile_reloc_display_list(
        &mut self,
        view: SceneRelocView<'_>,
    ) -> Result<usize, BackendError> {
        self.viewport = view.viewport();
        self.display_list.ops.clear();
        self.sort_scratch.clear();
        let nodes = view.nodes();
        if nodes.is_empty() {
            return Ok(0);
        }
        let clip_bounds = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: self.viewport.width,
            height: self.viewport.height,
        };
        emit_reloc_node(
            view,
            view.root().root_node_index as usize,
            clip_bounds,
            InheritedTextStyle::default(),
            &mut self.sort_scratch,
            &mut self.display_list.ops,
            0,
        )?;
        Ok(nodes.len())
    }
}

fn emit_reloc_node(
    view: SceneRelocView<'_>,
    idx: usize,
    clip_bounds: LayoutRect,
    inherited_text: InheritedTextStyle,
    sort_scratch: &mut Vec<u32>,
    ops: &mut Vec<PaintOp>,
    depth: usize,
) -> Result<(), BackendError> {
    if depth > 512 {
        return Ok(());
    }
    let Some(node) = view.node(idx) else {
        return Ok(());
    };
    if node.is_hidden() {
        return Ok(());
    }

    let kind = node.component_kind();
    let mut style = node.style_props();
    if kind == ComponentKind::Text && !node.has_explicit_style() {
        if let Some(c) = inherited_text.text_color {
            style.text_color = c;
        }
        if let Some(fs) = inherited_text.font_size {
            style.font_size = fs;
        }
    }

    let opacity = style.opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        return Ok(());
    }
    let pushed_opacity = opacity < 0.999;
    let opacity_op_idx = ops.len();
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(opacity));
    }

    let rect = node.computed_rect();
    let in_clip = intersects_clip(clip_bounds, rect, 0.0);
    let radius = style.border_radius.max(0.0);

    let box_shadow = view
        .box_shadow_of(node)
        .map_err(|e| BackendError::Internal(e.to_string()))?;
    if !box_shadow.is_empty() {
        for sh in parse_box_shadow(box_shadow) {
            let shadow_bounds = LayoutRect {
                x: rect.x + sh.offset_x - sh.spread_radius,
                y: rect.y + sh.offset_y - sh.spread_radius,
                width: rect.width + sh.spread_radius * 2.0,
                height: rect.height + sh.spread_radius * 2.0,
            };
            if intersects_clip(clip_bounds, shadow_bounds, sh.blur_radius * 1.5 + 1.0) {
                ops.push(PaintOp::DrawShadow {
                    rect,
                    radius,
                    offset_x: sh.offset_x,
                    offset_y: sh.offset_y,
                    blur_radius: sh.blur_radius,
                    spread_radius: sh.spread_radius,
                    color: sh.color,
                });
            }
        }
    }

    if style.background.a > 0 && in_clip {
        ops.push(PaintOp::FillRoundedRect {
            rect,
            radius,
            color: style.background,
        });
    }
    if style.border_width > 0.0
        && style.border_color.a > 0
        && intersects_clip(clip_bounds, rect, style.border_width * 0.5)
    {
        ops.push(PaintOp::StrokeRoundedRect {
            rect,
            radius,
            width: style.border_width,
            color: style.border_color,
        });
    }

    let effective_weight = if node.font_weight != 400 {
        node.font_weight
    } else {
        inherited_text.font_weight.unwrap_or(400)
    };
    let effective_align = if node.text_align != 0 {
        decode_text_align(node.text_align)
    } else {
        inherited_text.text_align.unwrap_or_default()
    };

    if in_clip {
        if kind == ComponentKind::Image {
            let src = view
                .image_src_of(node)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            if !src.is_empty() {
                ops.push(PaintOp::DrawImage {
                    rect,
                    radius,
                    src: src.to_owned(),
                    resize_mode: ImageResizeMode::from_u8(node.image_resize_mode)
                        .unwrap_or_default(),
                });
            }
        }

        let text = view
            .text_of(node)
            .map_err(|e| BackendError::Internal(e.to_string()))?;
        if !text.is_empty() && style.font_size > 0.0 && style.text_color.a > 0 {
            ops.push(PaintOp::DrawText {
                rect,
                text: text.to_owned(),
                font_size: style.font_size,
                font_weight: effective_weight,
                text_align: effective_align,
                color: style.text_color,
            });
        }

        if kind == ComponentKind::SkiaCanvas {
            let raw_dl = view
                .draw_list_of(node)
                .map_err(|e| BackendError::Internal(e.to_string()))?;
            let canvas_ops = if raw_dl.is_empty() {
                Vec::new()
            } else {
                DrawListDecoder::decode(raw_dl).unwrap_or_default()
            };
            ops.push(PaintOp::CustomSkiaCanvas {
                node_id: node.node_id,
                rect,
                ops: canvas_ops,
            });
        }
    }

    let child_inherited = if kind == ComponentKind::Text {
        InheritedTextStyle {
            font_weight: Some(effective_weight),
            text_align: Some(effective_align),
            text_color: Some(style.text_color),
            font_size: Some(style.font_size),
        }
    } else {
        InheritedTextStyle::default()
    };

    let children = view
        .children_of(node)
        .map_err(|e| BackendError::Internal(e.to_string()))?;
    if !children.is_empty() {
        let clips = node.clips_children();
        let next_clip = if clips {
            intersect_rects(clip_bounds, rect)
        } else {
            Some(clip_bounds)
        };
        if let Some(child_clip) = next_clip {
            let clip_op_idx = ops.len();
            if clips {
                ops.push(PaintOp::PushClip { rect, radius });
            }

            let has_z_order = children.len() > 1
                && children
                    .iter()
                    .any(|&cidx| view.node(cidx as usize).is_some_and(|c| c.z_index != 0));
            if has_z_order {
                let start = sort_scratch.len();
                sort_scratch.extend_from_slice(children);
                sort_scratch[start..]
                    .sort_by_key(|&cidx| view.node(cidx as usize).map_or(0, |c| c.z_index));
                let end = sort_scratch.len();
                for i in start..end {
                    let cidx = sort_scratch[i] as usize;
                    emit_reloc_node(
                        view,
                        cidx,
                        child_clip,
                        child_inherited,
                        sort_scratch,
                        ops,
                        depth + 1,
                    )?;
                }
                sort_scratch.truncate(start);
            } else {
                for &cidx in children {
                    emit_reloc_node(
                        view,
                        cidx as usize,
                        child_clip,
                        child_inherited,
                        sort_scratch,
                        ops,
                        depth + 1,
                    )?;
                }
            }

            if clips {
                if ops.len() == clip_op_idx + 1 {
                    ops.pop();
                } else {
                    ops.push(PaintOp::PopClip);
                }
            }
        }
    }

    if pushed_opacity {
        if ops.len() == opacity_op_idx + 1 {
            ops.pop();
        } else {
            ops.push(PaintOp::Restore);
        }
    }

    Ok(())
}
