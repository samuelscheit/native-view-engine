//! `TextInput` display-list emission (placeholder, secure bullet masking, range selection
//! highlight, vertical caret, IME preedit underline, focus ring) and logical-X cursor hit-testing.

use nve_abi::{ColorRgba, RawPropValue};
use nve_backend_headless::HeadlessNode;
use nve_engine::LayoutRect;

use crate::display_list::{extract_font_weight, extract_text_align};
use crate::text::{CosmicTextEngine, TextAlign};
use crate::{
    PaintOp, TextFocusState, FOCUS_RING_COLOR, FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR,
};

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

/// Returns `true` if the `TextInput` node has `secureTextEntry = true`.
#[inline]
pub(crate) fn is_secure_node(node: &HeadlessNode) -> bool {
    matches!(
        node.raw_props.get("secureTextEntry"),
        Some(RawPropValue::Bool(true))
    )
}

/// Measures single-line prefix advance width for caret/selection placement, preserving trailing spaces.
pub(crate) fn measure_caret_advance(
    text_engine: &CosmicTextEngine,
    prefix: &str,
    font_size: f32,
) -> f32 {
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
    let space_w = (text_engine.measure_text("a a", font_size, None).0
        - text_engine.measure_text("aa", font_size, None).0)
        .max(font_size * 0.28);
    base_w + (trailing_spaces as f32) * space_w
}

/// Computes the inner content rect of a `TextInput` accounting for padding and vertical centering.
pub(crate) fn compute_text_input_inner_rect(
    node: &HeadlessNode,
    rect: LayoutRect,
    font_size: f32,
) -> LayoutRect {
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

/// Resolves the closest UTF-8 byte boundary inside `committed` for a horizontal offset `local_x`
/// (relative to the start of the text content, already including any horizontal scroll offset).
pub(crate) fn cursor_byte_index_at_local_x(
    text_engine: &CosmicTextEngine,
    committed: &str,
    font_size: f32,
    is_secure: bool,
    local_x: f32,
) -> usize {
    if committed.is_empty() || !local_x.is_finite() || local_x <= 0.0 {
        return 0;
    }

    // Collect every valid UTF-8 char boundary in `committed` (including 0 and committed.len()).
    let mut boundaries: Vec<usize> = committed.char_indices().map(|(i, _)| i).collect();
    boundaries.push(committed.len());

    let advance_at = |boundary_slot: usize| -> f32 {
        let byte_idx = boundaries[boundary_slot];
        if byte_idx == 0 {
            return 0.0;
        }
        if is_secure {
            let bullets = "•".repeat(boundary_slot);
            measure_caret_advance(text_engine, &bullets, font_size)
        } else {
            measure_caret_advance(text_engine, &committed[..byte_idx], font_size)
        }
    };

    let last = boundaries.len() - 1;
    let total_w = advance_at(last);
    if local_x >= total_w {
        return committed.len();
    }

    // Binary search for the rightmost slot `lo` with `advance_at(lo) <= local_x`.
    let mut lo = 0usize;
    let mut hi = last;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if advance_at(mid) <= local_x {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    let left_w = advance_at(lo);
    let right_w = advance_at(hi);
    if (local_x - left_w) <= (right_w - local_x) {
        boundaries[lo]
    } else {
        boundaries[hi]
    }
}

/// Computes horizontal scroll offset for a focused `TextInput` so the caret stays visible.
pub(crate) fn compute_text_input_scroll_x(
    text_engine: &CosmicTextEngine,
    committed: &str,
    caret_byte: usize,
    font_size: f32,
    is_secure: bool,
    inner_width: f32,
) -> f32 {
    let caret_byte = clamp_char_boundary(committed, caret_byte);
    let caret_advance = if is_secure {
        let prefix = "•".repeat(committed[..caret_byte].chars().count());
        measure_caret_advance(text_engine, &prefix, font_size)
    } else {
        measure_caret_advance(text_engine, &committed[..caret_byte], font_size)
    };
    (caret_advance - (inner_width - 2.0).max(0.0)).max(0.0)
}

pub(crate) fn emit_text_input_ops(
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
    let font_weight = extract_font_weight(node).unwrap_or(400);
    let text_align = extract_text_align(node).unwrap_or(TextAlign::Left);
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
    let is_secure = is_secure_node(node);
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
                font_weight,
                text_align,
                color,
            });
        }
        return;
    }

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

    // Range selection highlight behind text.
    if preedit.is_none()
        && !committed.is_empty()
        && let Some((raw_s, raw_e)) = focus.selection_range()
    {
        let sel_s = clamp_char_boundary(committed, raw_s);
        let sel_e = clamp_char_boundary(committed, raw_e);
        if sel_s < sel_e {
            let (x0_adv, x1_adv) = if is_secure {
                let s_str = "•".repeat(committed[..sel_s].chars().count());
                let e_str = "•".repeat(committed[..sel_e].chars().count());
                (
                    measure_caret_advance(text_engine, &s_str, font_size),
                    measure_caret_advance(text_engine, &e_str, font_size),
                )
            } else {
                (
                    measure_caret_advance(text_engine, &committed[..sel_s], font_size),
                    measure_caret_advance(text_engine, &committed[..sel_e], font_size),
                )
            };
            let sel_w = (x1_adv - x0_adv).max(2.0);
            ops.push(PaintOp::FillRoundedRect {
                rect: LayoutRect {
                    x: text_rect.x + x0_adv - scroll_x,
                    y: caret_y,
                    width: sel_w,
                    height: caret_h,
                },
                radius: 2.0,
                color: FOCUS_SELECTION_COLOR,
            });
        }
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
            font_weight,
            text_align: TextAlign::Left,
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
            font_weight,
            text_align,
            color: ph_color,
        });
    }

    if let Some((before_str, pe_str)) = preedit_span {
        let pe_x =
            text_rect.x - scroll_x + measure_caret_advance(text_engine, &before_str, font_size);
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

    let caret_x = (text_rect.x + caret_advance - scroll_x).clamp(
        text_rect.x,
        (text_rect.x + text_rect.width - caret_w).max(text_rect.x),
    );
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

    ops.push(PaintOp::StrokeRoundedRect {
        rect,
        radius: node.style.border_radius.max(0.0),
        width: 2.0,
        color: FOCUS_RING_COLOR,
    });
}
