//! Retained `DisplayList` compilation, culling, `TextInput` focus/caret/IME painting, and
//! zero-allocation z-index sorting for [`crate::SkiaBackend`].

use std::collections::HashMap;

use nve_abi::{ColorRgba, ComponentKind, NodeId, RawPropValue};
use nve_backend_headless::{HeadlessBackend, HeadlessNode};
use nve_components::canvas_ir::{CanvasOp, DrawListDecoder, ImageResizeMode};
use nve_engine::{LayoutEngine, LayoutRect};

use crate::image_cache::ImageCache;
use crate::text::CosmicTextEngine;
use crate::{
    PaintOp, TextFocusState, FOCUS_RING_COLOR, FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR,
};

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

/// Clamps `byte_index` to the nearest valid UTF-8 character boundary at or before `byte_index`.
#[inline]
pub(crate) fn clamp_char_boundary(text: &str, byte_index: usize) -> usize {
    let mut idx = byte_index.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Returns the committed text value stored in a `TextInput` node's props or text payload.
#[inline]
pub(crate) fn raw_text_input_value(node: &HeadlessNode) -> &str {
    match node.raw_props.get("value") {
        Some(RawPropValue::String(s)) => s.as_str(),
        _ => node.text.as_deref().unwrap_or(""),
    }
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

/// Measures single-line prefix advance width for caret placement, preserving trailing spaces.
fn measure_caret_advance(text_engine: &CosmicTextEngine, prefix: &str, font_size: f32) -> f32 {
    if prefix.is_empty() {
        return 0.0;
    }
    let trimmed = prefix.trim_end_matches(' ');
    let trailing_spaces = prefix.len() - trimmed.len();
    let base_w = if trimmed.is_empty() {
        0.0
    } else {
        text_engine.measure_text(trimmed, font_size, None).0
    };
    if trailing_spaces == 0 {
        return base_w;
    }
    // Cosmic-text trims trailing whitespace from `run.line_w`; derive exact space advance via
    // `"a a" - "aa"` so typing spaces visibly advances the caret cleanly.
    let space_w = (text_engine.measure_text("a a", font_size, None).0
        - text_engine.measure_text("aa", font_size, None).0)
        .max(font_size * 0.28);
    base_w + (trailing_spaces as f32) * space_w
}

fn compute_text_input_inner_rect(node: &HeadlessNode, rect: LayoutRect, font_size: f32) -> LayoutRect {
    let pad_l = node.layout.padding.left.max(0.0);
    let pad_r = node.layout.padding.right.max(0.0);
    let mut pad_t = node.layout.padding.top.max(0.0);
    let pad_b = node.layout.padding.bottom.max(0.0);
    if pad_t == 0.0 && pad_b == 0.0 && rect.height > font_size * 1.4 {
        pad_t = ((rect.height - font_size * 1.35) * 0.5).max(0.0);
    }
    LayoutRect {
        x: rect.x + pad_l,
        y: rect.y + pad_t,
        width: (rect.width - pad_l - pad_r).max(1.0),
        height: (rect.height - pad_t - pad_b).max(1.0),
    }
}

fn emit_text_input_ops(
    node: &HeadlessNode,
    rect: LayoutRect,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
    ops: &mut Vec<PaintOp>,
) {
    let font_size = if node.style.font_size > 0.0 {
        node.style.font_size
    } else {
        13.0
    };
    let is_focused = focus.focused_node == Some(node.id);
    let committed = if is_focused {
        focus
            .uncontrolled_value
            .as_deref()
            .unwrap_or_else(|| raw_text_input_value(node))
    } else {
        raw_text_input_value(node)
    };
    let placeholder = match node.raw_props.get("placeholder") {
        Some(RawPropValue::String(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    };
    let is_secure = matches!(
        node.raw_props.get("secureTextEntry"),
        Some(RawPropValue::Bool(true))
    );
    let preedit = if is_focused {
        focus.ime_preedit.as_deref().filter(|s| !s.is_empty())
    } else {
        None
    };

    let text_color = if node.style.text_color.a > 0 {
        node.style.text_color
    } else {
        ColorRgba::rgb(241, 245, 249)
    };
    let text_rect = compute_text_input_inner_rect(node, rect, font_size);

    if !is_focused {
        let display_text = if !committed.is_empty() {
            let rendered = if is_secure {
                "•".repeat(committed.chars().count())
            } else {
                committed.to_owned()
            };
            Some((rendered, text_color))
        } else {
            placeholder.map(|ph| {
                let base = if node.style.text_color.a > 0 {
                    node.style.text_color
                } else {
                    ColorRgba::rgb(148, 163, 184)
                };
                let color = ColorRgba::rgba(
                    base.r,
                    base.g,
                    base.b,
                    ((base.a as f32) * 0.55).round().clamp(96.0, 170.0) as u8,
                );
                (ph.to_owned(), color)
            })
        };
        if let Some((text, color)) = display_text {
            ops.push(PaintOp::DrawText {
                rect: text_rect,
                text,
                font_size,
                color,
            });
        }
        return;
    }

    // Focused TextInput: build composed visual string, caret prefix, and optional IME underline.
    let caret_byte = clamp_char_boundary(committed, focus.caret_byte_index);
    let (rendered_value, caret_prefix, preedit_span) = if is_secure {
        let before_chars = committed[..caret_byte].chars().count();
        let after_chars = committed[caret_byte..].chars().count();
        if let Some(pe) = preedit {
            let pe_chars = pe.chars().count();
            let pe_cursor_chars = focus
                .ime_preedit_cursor
                .map(|(_, end)| pe[..clamp_char_boundary(pe, end)].chars().count())
                .unwrap_or(pe_chars);
            let total = "•".repeat(before_chars + pe_chars + after_chars);
            let prefix = "•".repeat(before_chars + pe_cursor_chars);
            let pe_start_str = "•".repeat(before_chars);
            let pe_str = "•".repeat(pe_chars);
            (total, prefix, Some((pe_start_str, pe_str)))
        } else {
            let total = "•".repeat(before_chars + after_chars);
            let prefix = "•".repeat(before_chars);
            (total, prefix, None)
        }
    } else if let Some(pe) = preedit {
        let before = &committed[..caret_byte];
        let after = &committed[caret_byte..];
        let pe_cursor_end = focus
            .ime_preedit_cursor
            .map(|(_, end)| clamp_char_boundary(pe, end))
            .unwrap_or(pe.len());
        let mut total = String::with_capacity(committed.len() + pe.len());
        total.push_str(before);
        total.push_str(pe);
        total.push_str(after);
        let mut prefix = String::with_capacity(before.len() + pe_cursor_end);
        prefix.push_str(before);
        prefix.push_str(&pe[..pe_cursor_end]);
        (total, prefix, Some((before.to_owned(), pe.to_owned())))
    } else {
        (
            committed.to_owned(),
            committed[..caret_byte].to_owned(),
            None,
        )
    };

    let caret_w = 2.0f32;
    let caret_h = (font_size * 1.25).clamp(8.0, (rect.height - 4.0).max(4.0));
    let line_h = (font_size * 1.35).min(text_rect.height);
    let caret_y = text_rect.y + ((line_h - caret_h) * 0.5).max(0.0);
    let caret_advance = measure_caret_advance(text_engine, &caret_prefix, font_size);
    let scroll_x = (caret_advance - (text_rect.width - caret_w).max(0.0)).max(0.0);

    ops.push(PaintOp::PushClip {
        rect,
        radius: node.style.border_radius.max(0.0),
    });

    // Selection-all highlight behind text.
    if focus.select_all && !rendered_value.is_empty() {
        let sel_w = measure_caret_advance(text_engine, &rendered_value, font_size)
            .min(text_rect.width)
            .max(2.0);
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: text_rect.x - scroll_x,
                y: caret_y,
                width: sel_w,
                height: caret_h,
            },
            radius: 2.0,
            color: FOCUS_SELECTION_COLOR,
        });
    }

    let draw_text_rect = LayoutRect {
        x: text_rect.x - scroll_x,
        y: text_rect.y,
        width: text_rect.width + scroll_x,
        height: text_rect.height,
    };

    if !rendered_value.is_empty() {
        ops.push(PaintOp::DrawText {
            rect: draw_text_rect,
            text: rendered_value,
            font_size,
            color: text_color,
        });
    } else if let Some(ph) = placeholder {
        let base = if node.style.text_color.a > 0 {
            node.style.text_color
        } else {
            ColorRgba::rgb(148, 163, 184)
        };
        let ph_color = ColorRgba::rgba(
            base.r,
            base.g,
            base.b,
            ((base.a as f32) * 0.55).round().clamp(96.0, 170.0) as u8,
        );
        ops.push(PaintOp::DrawText {
            rect: text_rect,
            text: ph.to_owned(),
            font_size,
            color: ph_color,
        });
    }

    // Subtle underline beneath active IME preedit span.
    if let Some((before_str, pe_str)) = preedit_span {
        let pe_x = text_rect.x - scroll_x + measure_caret_advance(text_engine, &before_str, font_size);
        let pe_w = measure_caret_advance(text_engine, &pe_str, font_size).max(4.0);
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: pe_x,
                y: caret_y + caret_h,
                width: pe_w,
                height: 1.5,
            },
            radius: 0.0,
            color: IME_PREEDIT_UNDERLINE_COLOR,
        });
    }

    // Crisp vertical cursor caret (works identically in software rasterizer & Skia GPU).
    let caret_x = (text_rect.x + caret_advance - scroll_x)
        .clamp(text_rect.x, (text_rect.x + text_rect.width - caret_w).max(text_rect.x));
    let caret_color = if text_color == node.style.background {
        FOCUS_RING_COLOR
    } else {
        text_color
    };
    ops.push(PaintOp::FillRoundedRect {
        rect: LayoutRect {
            x: caret_x,
            y: caret_y,
            width: caret_w,
            height: caret_h,
        },
        radius: 0.0,
        color: caret_color,
    });

    ops.push(PaintOp::PopClip);

    // Crisp focus ring painted on top of default border.
    ops.push(PaintOp::StrokeRoundedRect {
        rect,
        radius: node.style.border_radius.max(0.0),
        width: 2.0,
        color: FOCUS_RING_COLOR,
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_node_into<L: LayoutEngine>(
    layout_tree: &HeadlessBackend<L>,
    scroll_offsets: &HashMap<NodeId, (f32, f32)>,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
    clip_bounds: LayoutRect,
    offset: (f32, f32),
    sort_scratch: &mut Vec<NodeId>,
    ops: &mut Vec<PaintOp>,
    id: NodeId,
) {
    let Some(node) = layout_tree.node(id) else {
        return;
    };

    let opacity = node.style.opacity.clamp(0.0, 1.0);
    if opacity <= 0.001 {
        // Fully transparent subtree produces no visible pixels — cull completely.
        return;
    }

    let pushed_opacity = opacity < 0.999;
    let opacity_op_idx = ops.len();
    if pushed_opacity {
        ops.push(PaintOp::SaveOpacity(opacity));
    }

    let rect = offset_rect(node.computed_rect, offset);
    let in_clip = intersects_clip(clip_bounds, rect, 0.0);

    if node.style.background.a > 0 && in_clip {
        ops.push(PaintOp::FillRoundedRect {
            rect,
            radius: node.style.border_radius.max(0.0),
            color: node.style.background,
        });
    }
    if node.style.border_width > 0.0
        && node.style.border_color.a > 0
        && intersects_clip(clip_bounds, rect, node.style.border_width * 0.5)
    {
        ops.push(PaintOp::StrokeRoundedRect {
            rect,
            radius: node.style.border_radius.max(0.0),
            width: node.style.border_width,
            color: node.style.border_color,
        });
    }
    if in_clip {
        if node.kind == ComponentKind::Image {
            if let Some(src) = extract_image_src(node) {
                ops.push(PaintOp::DrawImage {
                    rect,
                    radius: node.style.border_radius.max(0.0),
                    src,
                    resize_mode: extract_image_resize_mode(node),
                });
            }
        }
        if node.kind == ComponentKind::TextInput {
            emit_text_input_ops(node, rect, focus, text_engine, ops);
        } else if let Some(text) = node.text.as_deref() {
            if !text.is_empty() && node.style.font_size > 0.0 && node.style.text_color.a > 0 {
                ops.push(PaintOp::DrawText {
                    rect,
                    text: text.to_owned(),
                    font_size: node.style.font_size,
                    color: node.style.text_color,
                });
            }
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

    // Paint children (optionally clipped for ScrollView / overflow:hidden, and scroll-translated).
    let children = &node.children;
    if !children.is_empty() {
        let clips = should_clip_children(node);
        let next_clip = if clips {
            intersect_rects(clip_bounds, rect)
        } else {
            Some(clip_bounds)
        };

        if let Some(child_clip) = next_clip {
            let clip_op_idx = ops.len();
            if clips {
                ops.push(PaintOp::PushClip {
                    rect,
                    radius: node.style.border_radius.max(0.0),
                });
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
                // Reuse the backend's scratch Vec as a stack frame to avoid per-node heap clones.
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
                        sort_scratch,
                        ops,
                        child,
                    );
                }
            }

            if clips {
                if ops.len() == clip_op_idx + 1 {
                    // Elide redundant PushClip/PopClip around non-drawing subtrees.
                    ops.pop();
                } else {
                    ops.push(PaintOp::PopClip);
                }
            }
        }
    }

    if pushed_opacity {
        if ops.len() == opacity_op_idx + 1 {
            // Elide redundant SaveOpacity/Restore around non-drawing subtrees.
            ops.pop();
        } else {
            ops.push(PaintOp::Restore);
        }
    }
}
