//! Zero-copy `SceneRelocView` (`NVERELOC` Frame-0) painting directly into [`SkiaBackend`]'s
//! retained [`DisplayList`] with zero layout computation.

use nve_components::tables::ImageFit;
use nve_engine::{BackendError, LayoutRect, SceneRelocView};
use nve_layout::StoreLayoutEngine;

use crate::display_list::{intersect_rects, intersects_clip, InheritedTextStyle};
use crate::raster::parse_box_shadow;
use crate::text::TextAlign;
use crate::{PaintOp, SkiaBackend, MAX_HIT_TEST_DEPTH};

#[inline]
fn decode_text_align(v: u8) -> TextAlign {
    match v {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

#[inline]
fn decode_image_fit(v: u8) -> ImageFit {
    match v {
        1 => ImageFit::Contain,
        2 => ImageFit::Fill,
        3 => ImageFit::None,
        4 => ImageFit::ScaleDown,
        _ => ImageFit::Cover,
    }
}

impl<L: StoreLayoutEngine> SkiaBackend<L> {
    /// Compiles a pre-laid-out [`SceneRelocView`] (`frame0.nvereloc`) directly into this
    /// backend's retained [`crate::DisplayList`] with zero layout work.
    pub fn load_scene_reloc(&mut self, view: &SceneRelocView<'_>) -> Result<(), BackendError> {
        let vp = view.viewport();
        self.viewport = vp;
        self.display_list.ops.clear();
        if view.nodes().is_empty() {
            return Ok(());
        }
        let clip_bounds = LayoutRect {
            x: 0.0,
            y: 0.0,
            width: vp.width.max(1.0),
            height: vp.height.max(1.0),
        };
        let mut sort_scratch = Vec::new();
        emit_reloc_node(
            view,
            view.root().root_node_index as usize,
            clip_bounds,
            InheritedTextStyle::default(),
            &mut sort_scratch,
            &mut self.display_list.ops,
            0,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_reloc_node(
    view: &SceneRelocView<'_>,
    idx: usize,
    clip_bounds: LayoutRect,
    inherited_text: InheritedTextStyle,
    sort_scratch: &mut Vec<u32>,
    ops: &mut Vec<PaintOp>,
    depth: usize,
) -> Result<(), BackendError> {
    if depth > MAX_HIT_TEST_DEPTH {
        return Ok(());
    }
    let Some(node) = view.node(idx) else {
        return Ok(());
    };
    if node.is_hidden() {
        return Ok(());
    }
    let rect = node.computed_rect();
    let style = node.style_props();
    if style.opacity <= 0.001 {
        return Ok(());
    }

    let opacity_op_idx = ops.len();
    let pushed_opacity = style.opacity < 0.999;
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(style.opacity));
    }

    let radius = style.border_radius.max(0.0);
    let in_clip = intersects_clip(clip_bounds, rect, 1.0);

    let box_shadow = view
        .box_shadow_of(node)
        .map_err(|e| BackendError::Internal(e.to_string()))?;
    if !box_shadow.is_empty() {
        for sh in parse_box_shadow(box_shadow) {
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

    if in_clip && style.background.a > 0 {
        ops.push(PaintOp::FillRoundedRect {
            rect,
            radius,
            color: style.background,
        });
    }
    if in_clip && style.border_width > 0.0 && style.border_color.a > 0 {
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
        let src = view
            .image_src_of(node)
            .map_err(|e| BackendError::Internal(e.to_string()))?;
        if !src.is_empty() {
            ops.push(PaintOp::DrawImage {
                rect,
                radius,
                src: src.to_owned(),
                fit: decode_image_fit(node.image_resize_mode),
            });
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
    }

    let child_inherited = InheritedTextStyle {
        font_weight: Some(effective_weight),
        text_align: Some(effective_align),
        text_color: Some(style.text_color),
        font_size: Some(style.font_size),
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
