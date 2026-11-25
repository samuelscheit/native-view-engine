//! `TextInput` display-list emission (placeholder, secure bullet masking, BiDi-aware multi-run
//! range selection highlights, vertical caret, IME preedit underline, focus ring), logical-X
//! grapheme-safe hit-testing, and exact live caret/preedit screen-rect computation.

use nve_abi::{ColorRgba, RawPropValue, StyleProps};
use nve_backend_headless::HeadlessNode;
use nve_engine::LayoutRect;

use crate::text::{CosmicTextEngine, TextAlign, DEFAULT_FONT_WEIGHT};
use crate::text_grapheme::{clamp_grapheme_boundary, grapheme_boundaries, grapheme_count};
use crate::{
    PaintOp, TextFocusState, FOCUS_RING_COLOR, FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR,
};

const CARET_WIDTH: f32 = 2.0;

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

/// Returns `true` if the `TextInput` node has `multiline = true`.
#[inline]
pub(crate) fn is_multiline_node(node: &HeadlessNode) -> bool {
    matches!(node.raw_props.get("multiline"), Some(RawPropValue::Bool(true)))
}

/// Measures single-line caret X advance using shaped HarfBuzz + BiDi layout, preserving trailing
/// spaces and grapheme cluster widths.
pub(crate) fn measure_caret_advance(
    text_engine: &CosmicTextEngine,
    text: &str,
    byte_index: usize,
    font_size: f32,
    font_weight: u16,
) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let idx = clamp_grapheme_boundary(text, byte_index);
    let shaped = text_engine.shape_single_line(text, font_size, font_weight);
    shaped.caret_x_at(text, idx)
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

/// Resolves the closest UAX #29 grapheme byte boundary inside `committed` for a visual horizontal
/// offset `local_x` (relative to the start of the text content, already including horizontal
/// scroll offset). Supports BiDi LTR/RTL runs and ligatures cleanly.
pub(crate) fn cursor_byte_index_at_local_x(
    text_engine: &CosmicTextEngine,
    committed: &str,
    font_size: f32,
    font_weight: u16,
    is_secure: bool,
    local_x: f32,
) -> usize {
    if committed.is_empty() || !local_x.is_finite() {
        return 0;
    }

    if is_secure {
        let boundaries = grapheme_boundaries(committed);
        let bullet_count = boundaries.len().saturating_sub(1);
        if bullet_count == 0 {
            return 0;
        }
        let bullets = "•".repeat(bullet_count);
        let shaped = text_engine.shape_single_line(&bullets, font_size, font_weight);
        let bullet_byte = shaped.hit_test_x(&bullets, local_x);
        let slot = bullets[..bullet_byte].chars().count().min(bullet_count);
        return boundaries[slot];
    }

    let shaped = text_engine.shape_single_line(committed, font_size, font_weight);
    shaped.hit_test_x(committed, local_x)
}

/// Computes horizontal scroll offset for a focused `TextInput` so the caret stays visible.
pub(crate) fn compute_text_input_scroll_x(
    text_engine: &CosmicTextEngine,
    committed: &str,
    caret_byte: usize,
    font_size: f32,
    font_weight: u16,
    is_secure: bool,
    inner_width: f32,
) -> f32 {
    let caret_byte = clamp_grapheme_boundary(committed, caret_byte);
    let caret_advance = if is_secure {
        let bullets = "•".repeat(grapheme_count(committed));
        let before_bullets = "•".repeat(grapheme_count(&committed[..caret_byte]));
        measure_caret_advance(
            text_engine,
            &bullets,
            before_bullets.len(),
            font_size,
            font_weight,
        )
    } else {
        measure_caret_advance(text_engine, committed, caret_byte, font_size, font_weight)
    };
    (caret_advance - (inner_width - CARET_WIDTH).max(0.0)).max(0.0)
}

/// Computes visual selection highlight rectangles (potentially multiple disjoint rects for mixed
/// BiDi selections crossing an LTR/RTL run boundary) in logical viewport coordinates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn selection_rects_for_range(
    text_engine: &CosmicTextEngine,
    committed: &str,
    range_start: usize,
    range_end: usize,
    font_size: f32,
    font_weight: u16,
    is_secure: bool,
    text_rect: LayoutRect,
    caret_y: f32,
    caret_h: f32,
    scroll_x: f32,
) -> Vec<LayoutRect> {
    let sel_s = clamp_grapheme_boundary(committed, range_start.min(range_end));
    let sel_e = clamp_grapheme_boundary(committed, range_start.max(range_end));
    if committed.is_empty() || sel_s >= sel_e {
        return Vec::new();
    }

    let spans = if is_secure {
        let total = "•".repeat(grapheme_count(committed));
        let s_bytes = grapheme_count(&committed[..sel_s]) * "•".len();
        let e_bytes = grapheme_count(&committed[..sel_e]) * "•".len();
        text_engine
            .shape_single_line(&total, font_size, font_weight)
            .selection_spans_for_range(&total, s_bytes, e_bytes)
    } else {
        text_engine
            .shape_single_line(committed, font_size, font_weight)
            .selection_spans_for_range(committed, sel_s, sel_e)
    };

    spans
        .into_iter()
        .map(|(x_left, width)| LayoutRect {
            x: text_rect.x + x_left - scroll_x,
            y: caret_y,
            width: width.max(2.0),
            height: caret_h,
        })
        .collect()
}

/// Shared computed visual geometry for a focused `TextInput` (used by both display-list painting
/// and live OS IME candidate-window cursor-area feedback).
pub(crate) struct FocusedTextInputGeometry {
    pub rendered_value: String,
    pub text_rect: LayoutRect,
    pub scroll_x: f32,
    pub caret_x: f32,
    pub caret_y: f32,
    pub caret_h: f32,
    pub preedit_underline: Option<(f32, f32)>,
    pub ime_cursor_rect: LayoutRect,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_focused_text_input_geometry(
    node: &HeadlessNode,
    rect: LayoutRect,
    font_size: f32,
    font_weight: u16,
    committed: &str,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
) -> FocusedTextInputGeometry {
    let is_secure = is_secure_node(node);
    let preedit = focus.ime_preedit.as_deref().filter(|s| !s.is_empty());
    let text_rect = compute_text_input_inner_rect(node, rect, font_size);
    let caret_byte = clamp_grapheme_boundary(committed, focus.caret_byte_index);

    let (rendered_value, caret_byte_in_rendered, preedit_byte_span) = if is_secure {
        let before_g = grapheme_count(&committed[..caret_byte]);
        let after_g = grapheme_count(&committed[caret_byte..]);
        let bullet_len = "•".len();
        if let Some(pe) = preedit {
            let pe_g = grapheme_count(pe);
            let pe_cursor_g = focus
                .ime_preedit_cursor
                .map(|(_, end)| grapheme_count(&pe[..clamp_grapheme_boundary(pe, end)]))
                .unwrap_or(pe_g);
            let total = "•".repeat(before_g + pe_g + after_g);
            let pe_start = before_g * bullet_len;
            let pe_end = (before_g + pe_g) * bullet_len;
            let caret_pos = (before_g + pe_cursor_g) * bullet_len;
            (total, caret_pos, Some((pe_start, pe_end)))
        } else {
            let total = "•".repeat(before_g + after_g);
            (total, before_g * bullet_len, None)
        }
    } else if let Some(pe) = preedit {
        let before = &committed[..caret_byte];
        let after = &committed[caret_byte..];
        let pe_cursor_end = focus
            .ime_preedit_cursor
            .map(|(_, end)| clamp_grapheme_boundary(pe, end))
            .unwrap_or(pe.len());
        let mut total = String::with_capacity(committed.len() + pe.len());
        total.push_str(before);
        total.push_str(pe);
        total.push_str(after);
        let pe_start = before.len();
        let pe_end = pe_start + pe.len();
        (total, pe_start + pe_cursor_end, Some((pe_start, pe_end)))
    } else {
        (committed.to_owned(), caret_byte, None)
    };

    let caret_h = (font_size * 1.25).clamp(8.0, (rect.height - 4.0).max(4.0));
    let line_h = (font_size * 1.35).min(text_rect.height);
    let caret_y = text_rect.y + ((line_h - caret_h) * 0.5).max(0.0);

    let shaped = text_engine.shape_single_line(&rendered_value, font_size, font_weight);
    let caret_advance = shaped.caret_x_at(&rendered_value, caret_byte_in_rendered);
    let scroll_x = (caret_advance - (text_rect.width - CARET_WIDTH).max(0.0)).max(0.0);

    let max_caret_x = (text_rect.x + text_rect.width - CARET_WIDTH).max(text_rect.x);
    let caret_x = (text_rect.x + caret_advance - scroll_x).clamp(text_rect.x, max_caret_x);

    let preedit_underline = preedit_byte_span.map(|(pe_start, pe_end)| {
        let spans = shaped.selection_spans_for_range(&rendered_value, pe_start, pe_end);
        if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
            let x0 = text_rect.x - scroll_x + first.0;
            let x1 = text_rect.x - scroll_x + last.0 + last.1;
            (x0, (x1 - x0).max(4.0))
        } else {
            let pe_x = text_rect.x - scroll_x + shaped.caret_x_at(&rendered_value, pe_start);
            (pe_x, 4.0)
        }
    });

    let ime_cursor_rect = if let Some((pe_x, pe_w)) = preedit_underline {
        let visible_left = pe_x.clamp(text_rect.x, max_caret_x);
        let visible_right = (pe_x + pe_w).clamp(visible_left + CARET_WIDTH, text_rect.x + text_rect.width);
        LayoutRect {
            x: visible_left,
            y: caret_y,
            width: (visible_right - visible_left).max(CARET_WIDTH),
            height: caret_h,
        }
    } else {
        LayoutRect {
            x: caret_x,
            y: caret_y,
            width: CARET_WIDTH,
            height: caret_h,
        }
    };

    FocusedTextInputGeometry {
        rendered_value,
        text_rect,
        scroll_x,
        caret_x,
        caret_y,
        caret_h,
        preedit_underline,
        ime_cursor_rect,
    }
}

/// Computes the live logical IME cursor/preedit screen rectangle for `node`, accounting for
/// `visual_rect` (scroll offsets), inner padding, horizontal text scroll, BiDi layout, and
/// active IME preedit span.
pub(crate) fn compute_ime_cursor_rect(
    node: &HeadlessNode,
    visual_rect: LayoutRect,
    font_size: f32,
    committed: &str,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
) -> LayoutRect {
    compute_focused_text_input_geometry(
        node,
        visual_rect,
        font_size,
        DEFAULT_FONT_WEIGHT,
        committed,
        focus,
        text_engine,
    )
    .ime_cursor_rect
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_text_input_ops(
    node: &HeadlessNode,
    style: &StyleProps,
    font_weight: u16,
    text_align: TextAlign,
    rect: LayoutRect,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
    ops: &mut Vec<PaintOp>,
) {
    let font_size = if style.font_size > 0.0 {
        style.font_size
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
    let is_secure = is_secure_node(node);
    let preedit = if is_focused {
        focus.ime_preedit.as_deref().filter(|s| !s.is_empty())
    } else {
        None
    };

    let text_color = if style.text_color.a > 0 {
        style.text_color
    } else {
        ColorRgba::rgb(241, 245, 249)
    };
    let text_rect = compute_text_input_inner_rect(node, rect, font_size);

    if !is_focused {
        let display_text = if !committed.is_empty() {
            let rendered = if is_secure {
                "•".repeat(grapheme_count(committed))
            } else {
                committed.to_owned()
            };
            Some((rendered, text_color))
        } else {
            placeholder.map(|ph| {
                let base = if style.text_color.a > 0 {
                    style.text_color
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

    let geom = compute_focused_text_input_geometry(
        node,
        rect,
        font_size,
        font_weight,
        committed,
        focus,
        text_engine,
    );

    ops.push(PaintOp::PushClip {
        rect,
        radius: style.border_radius.max(0.0),
    });

    // BiDi-aware range selection highlights (may emit multiple disjoint rects across LTR/RTL runs).
    if preedit.is_none()
        && !committed.is_empty()
        && let Some((raw_s, raw_e)) = focus.selection_range()
    {
        for sel_rect in selection_rects_for_range(
            text_engine,
            committed,
            raw_s,
            raw_e,
            font_size,
            font_weight,
            is_secure,
            geom.text_rect,
            geom.caret_y,
            geom.caret_h,
            geom.scroll_x,
        ) {
            ops.push(PaintOp::FillRoundedRect {
                rect: sel_rect,
                radius: 2.0,
                color: FOCUS_SELECTION_COLOR,
            });
        }
    }

    let draw_text_rect = LayoutRect {
        x: geom.text_rect.x - geom.scroll_x,
        y: geom.text_rect.y,
        width: geom.text_rect.width + geom.scroll_x,
        height: geom.text_rect.height,
    };

    if !geom.rendered_value.is_empty() {
        ops.push(PaintOp::DrawText {
            rect: draw_text_rect,
            text: geom.rendered_value,
            font_size,
            font_weight,
            text_align: TextAlign::Left,
            color: text_color,
        });
    } else if let Some(ph) = placeholder {
        let base = if style.text_color.a > 0 {
            style.text_color
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
            rect: geom.text_rect,
            text: ph.to_owned(),
            font_size,
            font_weight,
            text_align,
            color: ph_color,
        });
    }

    if let Some((pe_x, pe_w)) = geom.preedit_underline {
        ops.push(PaintOp::FillRoundedRect {
            rect: LayoutRect {
                x: pe_x,
                y: geom.caret_y + geom.caret_h,
                width: pe_w,
                height: 1.5,
            },
            radius: 0.0,
            color: IME_PREEDIT_UNDERLINE_COLOR,
        });
    }

    let caret_color = if text_color == style.background {
        FOCUS_RING_COLOR
    } else {
        text_color
    };
    ops.push(PaintOp::FillRoundedRect {
        rect: LayoutRect {
            x: geom.caret_x,
            y: geom.caret_y,
            width: CARET_WIDTH,
            height: geom.caret_h,
        },
        radius: 0.0,
        color: caret_color,
    });

    ops.push(PaintOp::PopClip);

    ops.push(PaintOp::StrokeRoundedRect {
        rect,
        radius: style.border_radius.max(0.0),
        width: 2.0,
        color: FOCUS_RING_COLOR,
    });
}
