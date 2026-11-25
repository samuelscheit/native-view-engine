//! Retained `DisplayList` compilation, culling, `box-shadow` emission, `ScrollView` overlay
//! scrollbars, inherited text weight/alignment propagation, and zero-allocation z-index sorting
//! for [`crate::SkiaBackend`].

use std::collections::HashMap;

use nve_abi::{ColorRgba, ComponentKind, NodeId, RawPropValue};
use nve_backend_headless::{HeadlessBackend, HeadlessNode};
use nve_components::canvas_ir::{CanvasOp, DrawListDecoder, ImageResizeMode};
use nve_components::style_patch::{Display, Overflow, TextAlign as PatchTextAlign};
use nve_engine::{LayoutEngine, LayoutRect};

use crate::image_cache::ImageCache;
use crate::raster::parse_box_shadow;
use crate::scroll::compute_scroll_content_size;
use crate::text::{parse_font_weight, CosmicTextEngine, TextAlign, DEFAULT_FONT_WEIGHT};
use crate::text_input::emit_text_input_ops;
use crate::{PaintOp, TextFocusState, SCROLLBAR_THUMB_COLOR};

const SCROLLBAR_THICKNESS: f32 = 6.0;
const SCROLLBAR_MARGIN: f32 = 3.0;
const SCROLLBAR_MIN_THUMB: f32 = 20.0;

#[inline]
pub(crate) fn offset_rect(rect: LayoutRect, offset: (f32, f32)) -> LayoutRect {
    LayoutRect {
        x: rect.x + offset.0,
        y: rect.y + offset.1,
        width: rect.width,
        height: rect.height,
    }
}

#[inline]
pub(crate) fn point_in_rect(x: f32, y: f32, r: LayoutRect) -> bool {
    r.width > 0.0
        && r.height > 0.0
        && x >= r.x
        && x <= r.x + r.width
        && y >= r.y
        && y <= r.y + r.height
}

#[inline]
pub(crate) fn intersect_rects(a: LayoutRect, b: LayoutRect) -> Option<LayoutRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    if x1 > x0 && y1 > y0 {
        Some(LayoutRect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    } else {
        None
    }
}

#[inline]
pub(crate) fn intersects_clip(clip: LayoutRect, rect: LayoutRect, outset: f32) -> bool {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return false;
    }
    let x0 = rect.x - outset;
    let y0 = rect.y - outset;
    let x1 = rect.x + rect.width + outset;
    let y1 = rect.y + rect.height + outset;
    x1 > clip.x && y1 > clip.y && x0 < clip.x + clip.width && y0 < clip.y + clip.height
}

pub(crate) fn parse_px_raw_prop(val: Option<&RawPropValue>) -> Option<f32> {
    match val? {
        RawPropValue::F64(v) if v.is_finite() => Some(*v as f32),
        RawPropValue::I64(v) => Some(*v as f32),
        RawPropValue::String(s) => {
            let trimmed = s.trim().trim_end_matches("px").trim();
            let v = trimmed.parse::<f32>().ok()?;
            v.is_finite().then_some(v)
        }
        _ => None,
    }
}

pub(crate) fn extract_font_weight(node: &HeadlessNode) -> Option<u16> {
    let raw = node
        .raw_props
        .get("font-weight")
        .or_else(|| node.raw_props.get("fontWeight"))?;
    match raw {
        RawPropValue::I64(n) => Some((*n).clamp(100, 900) as u16),
        RawPropValue::F64(n) if n.is_finite() => Some((n.round() as i64).clamp(100, 900) as u16),
        RawPropValue::String(s) => parse_font_weight(s),
        _ => None,
    }
}

pub(crate) fn extract_text_align(node: &HeadlessNode) -> Option<TextAlign> {
    let raw = node
        .raw_props
        .get("text-align")
        .or_else(|| node.raw_props.get("textAlign"))?;
    match raw {
        RawPropValue::String(s) => TextAlign::parse(s),
        _ => None,
    }
}

pub(crate) fn should_clip_children(node: &HeadlessNode) -> bool {
    if node.kind == ComponentKind::ScrollView {
        return true;
    }
    if let Some(RawPropValue::String(ov)) = node.raw_props.get("overflow") {
        let lower = ov.trim().to_ascii_lowercase();
        if matches!(lower.as_str(), "hidden" | "clip" | "scroll" | "auto") {
            return true;
        }
    }
    if let Some(RawPropValue::Bool(true)) = node.raw_props.get("clip") {
        return true;
    }
    false
}

fn extract_image_src(node: &HeadlessNode) -> Option<String> {
    let raw = node.raw_props.get("src")?;
    match raw {
        RawPropValue::String(s) if !s.is_empty() => Some(s.clone()),
        RawPropValue::Bytes(bytes) if !bytes.is_empty() => {
            ImageCache::shared().register_png_bytes(bytes).ok()
        }
        _ => None,
    }
}

fn extract_image_resize_mode(node: &HeadlessNode) -> ImageResizeMode {
    let raw = node
        .raw_props
        .get("resizeMode")
        .or_else(|| node.raw_props.get("resize_mode"));
    match raw {
        Some(RawPropValue::String(s)) => ImageResizeMode::parse(s).unwrap_or_default(),
        Some(RawPropValue::I64(n)) => ImageResizeMode::from_u8(*n as u8).unwrap_or_default(),
        _ => ImageResizeMode::default(),
    }
}

fn extract_canvas_ops(node: &HeadlessNode) -> Vec<CanvasOp> {
    let Some(raw) = node
        .raw_props
        .get("drawList")
        .or_else(|| node.raw_props.get("draw_list"))
    else {
        return Vec::new();
    };
    DrawListDecoder::decode_raw_prop(raw).unwrap_or_default()
}

fn emit_box_shadows(
    node: &HeadlessNode,
    border_radius: f32,
    rect: LayoutRect,
    clip_bounds: LayoutRect,
    ops: &mut Vec<PaintOp>,
) {
    let Some(RawPropValue::String(raw_shadow)) = node
        .raw_props
        .get("box-shadow")
        .or_else(|| node.raw_props.get("boxShadow"))
    else {
        return;
    };
    let radius = border_radius.max(0.0);
    for sh in parse_box_shadow(raw_shadow) {
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

fn emit_scrollview_scrollbars<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_offsets: &HashMap<NodeId, (f32, f32)>,
    id: NodeId,
    rect: LayoutRect,
    ops: &mut Vec<PaintOp>,
) {
    let Some((cw, ch)) = compute_scroll_content_size(layout_tree, id) else {
        return;
    };
    let has_v = ch > rect.height + 0.5;
    let has_h = cw > rect.width + 0.5;
    if !has_v && !has_h {
        return;
    }
    let (sx, sy) = scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0));
    let corner_gap = if has_v && has_h {
        SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN
    } else {
        0.0
    };

    if has_v
        && rect.height > SCROLLBAR_MARGIN * 2.0 + corner_gap + 4.0
        && rect.width > SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN * 2.0
    {
        let track_h = (rect.height - SCROLLBAR_MARGIN * 2.0 - corner_gap).max(1.0);
        let thumb_h = ((rect.height / ch) * track_h).clamp(SCROLLBAR_MIN_THUMB.min(track_h), track_h);
        let max_sy = (ch - rect.height).max(1.0);
        let ratio_y = (sy / max_sy).clamp(0.0, 1.0);
        let thumb_y = rect.y + SCROLLBAR_MARGIN + ratio_y * (track_h - thumb_h);
        let thumb_x = rect.x + rect.width - SCROLLBAR_THICKNESS - SCROLLBAR_MARGIN;
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: SCROLLBAR_THICKNESS,
                height: thumb_h,
            },
            radius: SCROLLBAR_THICKNESS * 0.5,
            color: SCROLLBAR_THUMB_COLOR,
        });
    }

    if has_h
        && rect.width > SCROLLBAR_MARGIN * 2.0 + corner_gap + 4.0
        && rect.height > SCROLLBAR_THICKNESS + SCROLLBAR_MARGIN * 2.0
    {
        let track_w = (rect.width - SCROLLBAR_MARGIN * 2.0 - corner_gap).max(1.0);
        let thumb_w = ((rect.width / cw) * track_w).clamp(SCROLLBAR_MIN_THUMB.min(track_w), track_w);
        let max_sx = (cw - rect.width).max(1.0);
        let ratio_x = (sx / max_sx).clamp(0.0, 1.0);
        let thumb_x = rect.x + SCROLLBAR_MARGIN + ratio_x * (track_w - thumb_w);
        let thumb_y = rect.y + rect.height - SCROLLBAR_THICKNESS - SCROLLBAR_MARGIN;
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: thumb_x,
                y: thumb_y,
                width: thumb_w,
                height: SCROLLBAR_THICKNESS,
            },
            radius: SCROLLBAR_THICKNESS * 0.5,
            color: SCROLLBAR_THUMB_COLOR,
        });
    }
}

/// Inherited text formatting propagated from parent `ComponentKind::Text` containers to leaves.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InheritedTextStyle {
    pub(crate) font_weight: Option<u16>,
    pub(crate) text_align: Option<TextAlign>,
    pub(crate) text_color: Option<ColorRgba>,
    pub(crate) font_size: Option<f32>,
}

#[inline]
fn map_patch_text_align(align: PatchTextAlign) -> TextAlign {
    match align {
        PatchTextAlign::Left | PatchTextAlign::Justify => TextAlign::Left,
        PatchTextAlign::Center => TextAlign::Center,
        PatchTextAlign::Right => TextAlign::Right,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_node_into<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_offsets: &HashMap<NodeId, (f32, f32)>,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
    clip_bounds: LayoutRect,
    offset: (f32, f32),
    inherited_text: InheritedTextStyle,
    sort_scratch: &mut Vec<NodeId>,
    ops: &mut Vec<PaintOp>,
    id: NodeId,
) {
    let Some(node) = layout_tree.node(id) else {
        return;
    };

    let patch = layout_tree.effective_patch(id);
    if patch.display == Some(Display::None) {
        return;
    }

    let mut style = node.style;
    if !patch.is_empty() {
        patch.apply_style_to(&mut style);
    }
    if node.kind == ComponentKind::Text && !node.has_explicit_style {
        if patch.text_color.is_none()
            && let Some(c) = inherited_text.text_color
        {
            style.text_color = c;
        }
        if patch.font_size.is_none()
            && let Some(fs) = inherited_text.font_size
        {
            style.font_size = fs;
        }
    }

    let opacity = style.opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        return;
    }

    let pushed_opacity = opacity < 0.999;
    let opacity_op_idx = ops.len();
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(opacity));
    }

    let rect = offset_rect(node.computed_rect, offset);
    let in_clip = intersects_clip(clip_bounds, rect, 0.0);
    let radius = style.border_radius.max(0.0);

    // Emit outer box-shadows before background fill so shadows render behind the box.
    emit_box_shadows(node, radius, rect, clip_bounds, ops);

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

    let own_weight = patch.font_weight.or_else(|| extract_font_weight(node));
    let own_align = patch
        .text_align
        .map(map_patch_text_align)
        .or_else(|| extract_text_align(node));
    let effective_weight = own_weight
        .or(inherited_text.font_weight)
        .unwrap_or(DEFAULT_FONT_WEIGHT);
    let effective_align = own_align.or(inherited_text.text_align).unwrap_or_default();

    if in_clip {
        if node.kind == ComponentKind::Image
            && let Some(src) = extract_image_src(node)
        {
            ops.push(PaintOp::DrawImage {
                rect,
                radius,
                src,
                resize_mode: extract_image_resize_mode(node),
            });
        }
        if node.kind == ComponentKind::TextInput {
            emit_text_input_ops(
                node,
                &style,
                effective_weight,
                effective_align,
                rect,
                focus,
                text_engine,
                ops,
            );
        } else if let Some(text) = node.text.as_deref()
            && !text.is_empty()
            && style.font_size > 0.0
            && style.text_color.a > 0
        {
            ops.push(PaintOp::DrawText {
                rect,
                text: text.to_owned(),
                font_size: style.font_size,
                font_weight: effective_weight,
                text_align: effective_align,
                color: style.text_color,
            });
        }
        if node.kind == ComponentKind::SkiaCanvas {
            let canvas_ops = extract_canvas_ops(node);
            ops.push(PaintOp::CustomSkiaCanvas {
                node_id: id,
                rect,
                ops: canvas_ops,
            });
        }
    }

    let child_inherited = if node.kind == ComponentKind::Text {
        InheritedTextStyle {
            font_weight: own_weight.or(inherited_text.font_weight),
            text_align: own_align.or(inherited_text.text_align),
            text_color: patch.text_color.or(inherited_text.text_color),
            font_size: patch.font_size.or(inherited_text.font_size),
        }
    } else {
        InheritedTextStyle::default()
    };

    let children = &node.children;
    if !children.is_empty() {
        let clips = match patch.overflow {
            Some(Overflow::Visible) => node.kind == ComponentKind::ScrollView,
            Some(Overflow::Hidden | Overflow::Scroll | Overflow::Auto) => true,
            None => should_clip_children(node),
        };
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

            let child_offset = if node.kind == ComponentKind::ScrollView {
                let (sx, sy) = scroll_offsets.get(&id).copied().unwrap_or((0.0, 0.0));
                let visual_sy = if let Some(top_spacer) =
                    parse_px_raw_prop(node.raw_props.get("virtual-top-spacer"))
                {
                    (sy - top_spacer).max(0.0)
                } else {
                    sy
                };
                (offset.0 - sx, offset.1 - visual_sy)
            } else {
                offset
            };

            let has_z_order = children.len() > 1
                && children
                    .iter()
                    .any(|&cid| layout_tree.node(cid).is_some_and(|c| c.style.z_index != 0));
            if has_z_order {
                let start = sort_scratch.len();
                sort_scratch.extend_from_slice(children);
                sort_scratch[start..]
                    .sort_by_key(|&cid| layout_tree.node(cid).map_or(0, |c| c.style.z_index));
                let end = sort_scratch.len();
                for i in start..end {
                    let child = sort_scratch[i];
                    emit_node_into(
                        layout_tree,
                        scroll_offsets,
                        focus,
                        text_engine,
                        child_clip,
                        child_offset,
                        child_inherited,
                        sort_scratch,
                        ops,
                        child,
                    );
                }
                sort_scratch.truncate(start);
            } else {
                for &child in children {
                    emit_node_into(
                        layout_tree,
                        scroll_offsets,
                        focus,
                        text_engine,
                        child_clip,
                        child_offset,
                        child_inherited,
                        sort_scratch,
                        ops,
                        child,
                    );
                }
            }

            if node.kind == ComponentKind::ScrollView && in_clip {
                emit_scrollview_scrollbars(layout_tree, scroll_offsets, id, rect, ops);
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
}
