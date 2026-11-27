//! `TextInput` display-list emission (placeholder, secure bullet masking, BiDi-aware multi-run
//! range selection highlights, vertical caret, IME preedit underline, focus ring), logical-X
//! grapheme-safe hit-testing, and exact live caret/preedit screen-rect computation over `&NodeStore`.

use nve_abi::ColorRgba;
use nve_components::tables::{Layout, TextContent, TextInput};
use nve_engine::LayoutRect;
use nve_store::{NodeId, NodeStore};

use crate::text::{CosmicTextEngine, TextAlign};
use crate::text_grapheme::{clamp_grapheme_boundary, grapheme_boundaries, grapheme_count};
use crate::{
    PaintOp, TextFocusState, FOCUS_RING_COLOR, FOCUS_SELECTION_COLOR, IME_PREEDIT_UNDERLINE_COLOR,
};

const CARET_WIDTH: f32 = 2.0;

#[inline]
pub(crate) fn raw_text_input_value(store: &NodeStore, id: NodeId) -> &str {
    if let Some(ti) = store.try_row::<TextInput>(id) {
        let v = ti.value();
        if !v.is_empty() {
            return v;
        }
    }
    store
        .try_row::<TextContent>(id)
        .map(|t| t.text())
        .unwrap_or("")
}

#[inline]
pub(crate) fn is_secure_node(store: &NodeStore, id: NodeId) -> bool {
    store.try_row::<TextInput>(id).is_some_and(|t| t.secure())
}

#[inline]
pub(crate) fn is_multiline_node(store: &NodeStore, id: NodeId) -> bool {
    store
        .try_row::<TextInput>(id)
        .is_some_and(|t| t.multiline())
}

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

pub(crate) fn compute_text_input_inner_rect(
    store: &NodeStore,
    id: NodeId,
    rect: LayoutRect,
    font_size: f32,
) -> LayoutRect {
    let (pad_l, pad_r, mut pad_t, pad_b) = store
        .try_row::<Layout>(id)
        .map(|l| {
            let px = |len: nve_store::Length| if len.is_points() { len.value().max(0.0) } else { 0.0 };
            (
                px(l.padding_left()),
                px(l.padding_right()),
                px(l.padding_top()),
                px(l.padding_bottom()),
            )
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
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
        measure_caret_advance(text_engine, &bullets, before_bullets.len(), font_size, font_weight)
    } else {
        measure_caret_advance(text_engine, committed, caret_byte, font_size, font_weight)
    };
    let usable = (inner_width - CARET_WIDTH - 2.0).max(1.0);
    if caret_advance > usable {
        caret_advance - usable
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn selection_rects_for_range(
    text_engine: &CosmicTextEngine,
    committed: &str,
    start: usize,
    end: usize,
    font_size: f32,
    font_weight: u16,
    is_secure: bool,
    text_rect: LayoutRect,
    scroll_x: f32,
) -> Vec<LayoutRect> {
    if committed.is_empty() || start >= end {
        return Vec::new();
    }
    let s = clamp_grapheme_boundary(committed, start);
    let e = clamp_grapheme_boundary(committed, end);
    if s >= e {
        return Vec::new();
    }
    let spans: Vec<(f32, f32)> = if is_secure {
        let bullets = "•".repeat(grapheme_count(committed));
        let bs = "•".repeat(grapheme_count(&committed[..s])).len();
        let be = "•".repeat(grapheme_count(&committed[..e])).len();
        let shaped = text_engine.shape_single_line(&bullets, font_size, font_weight);
        shaped.selection_spans_for_range(&bullets, bs, be)
    } else {
        let shaped = text_engine.shape_single_line(committed, font_size, font_weight);
        shaped.selection_spans_for_range(committed, s, e)
    };
    let line_h = (font_size * 1.35).min(text_rect.height);
    spans
        .into_iter()
        .filter_map(|(x0, x1)| {
            let sx0 = (text_rect.x + x0 - scroll_x).max(text_rect.x);
            let sx1 = (text_rect.x + x1 - scroll_x).min(text_rect.x + text_rect.width);
            (sx1 > sx0).then_some(LayoutRect {
                x: sx0,
                y: text_rect.y,
                width: sx1 - sx0,
                height: line_h,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_ime_cursor_rect(
    text_engine: &CosmicTextEngine,
    committed: &str,
    caret_byte: usize,
    preedit: Option<&str>,
    font_size: f32,
    font_weight: u16,
    is_secure: bool,
    text_rect: LayoutRect,
) -> LayoutRect {
    let caret_byte = clamp_grapheme_boundary(committed, caret_byte);
    let scroll_x = compute_text_input_scroll_x(
        text_engine,
        committed,
        caret_byte,
        font_size,
        font_weight,
        is_secure,
        text_rect.width,
    );
    let (caret_x, width) = if is_secure {
        let bullets = "•".repeat(grapheme_count(committed));
        let before = "•".repeat(grapheme_count(&committed[..caret_byte]));
        (
            measure_caret_advance(text_engine, &bullets, before.len(), font_size, font_weight),
            CARET_WIDTH,
        )
    } else if let Some(pre) = preedit
        && !pre.is_empty()
    {
        let mut combined = String::with_capacity(committed.len() + pre.len());
        combined.push_str(&committed[..caret_byte]);
        combined.push_str(pre);
        combined.push_str(&committed[caret_byte..]);
        let start_x =
            measure_caret_advance(text_engine, &combined, caret_byte, font_size, font_weight);
        let end_x = measure_caret_advance(
            text_engine,
            &combined,
            caret_byte + pre.len(),
            font_size,
            font_weight,
        );
        (start_x.min(end_x), (end_x - start_x).abs().max(CARET_WIDTH))
    } else {
        (
            measure_caret_advance(text_engine, committed, caret_byte, font_size, font_weight),
            CARET_WIDTH,
        )
    };
    LayoutRect {
        x: (text_rect.x + caret_x - scroll_x).clamp(text_rect.x, text_rect.x + text_rect.width),
        y: text_rect.y,
        width,
        height: (font_size * 1.35).min(text_rect.height),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_text_input_ops(
    store: &NodeStore,
    id: NodeId,
    rect: LayoutRect,
    radius: f32,
    text_color: ColorRgba,
    font_size: f32,
    font_weight: u16,
    focus: &TextFocusState,
    text_engine: &CosmicTextEngine,
    ops: &mut Vec<PaintOp>,
) {
    let is_focused = focus.focused_node == Some(id);
    let fs = if font_size > 0.0 { font_size } else { 14.0 };
    let text_rect = compute_text_input_inner_rect(store, id, rect, fs);
    let is_secure = is_secure_node(store, id);

    let committed = if is_focused {
        focus
            .uncontrolled_value
            .as_deref()
            .unwrap_or_else(|| raw_text_input_value(store, id))
    } else {
        raw_text_input_value(store, id)
    };
    let preedit = if is_focused {
        focus.ime_preedit.as_deref().filter(|p| !p.is_empty())
    } else {
        None
    };

    if is_focused {
        ops.push(PaintOp::StrokeRoundedRect {
            rect,
            radius,
            width: 2.0,
            color: FOCUS_RING_COLOR,
        });
    }

    let placeholder = store
        .try_row::<TextInput>(id)
        .map(|t| t.placeholder())
        .unwrap_or("");
    let placeholder_color = ColorRgba::rgba(148, 163, 184, 180);

    let caret_byte = clamp_grapheme_boundary(committed, focus.caret_byte_index);
    let scroll_x = if is_focused {
        compute_text_input_scroll_x(
            text_engine,
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

    ops.push(PaintOp::PushClip {
        rect: text_rect,
        radius: 0.0,
    });

    if is_focused
        && preedit.is_none()
        && let Some((sel_start, sel_end)) = focus.selection_range()
    {
        for sel_rect in selection_rects_for_range(
            text_engine,
            committed,
            sel_start,
            sel_end,
            fs,
            font_weight,
            is_secure,
            text_rect,
            scroll_x,
        ) {
            ops.push(PaintOp::FillRoundedRect {
                rect: sel_rect,
                radius: 2.0,
                color: FOCUS_SELECTION_COLOR,
            });
        }
    }

    if committed.is_empty() && preedit.is_none() {
        if !placeholder.is_empty() {
            ops.push(PaintOp::DrawText {
                rect: text_rect,
                text: placeholder.to_owned(),
                font_size: fs,
                font_weight,
                text_align: TextAlign::Left,
                color: placeholder_color,
            });
        }
    } else {
        let display_text = if is_secure {
            let mut s = "•".repeat(grapheme_count(committed));
            if let Some(p) = preedit {
                s.push_str(p);
            }
            s
        } else if let Some(p) = preedit {
            let mut s = String::with_capacity(committed.len() + p.len());
            s.push_str(&committed[..caret_byte]);
            s.push_str(p);
            s.push_str(&committed[caret_byte..]);
            s
        } else {
            committed.to_owned()
        };
        ops.push(PaintOp::DrawText {
            rect: LayoutRect {
                x: text_rect.x - scroll_x,
                y: text_rect.y,
                width: text_rect.width + scroll_x,
                height: text_rect.height,
            },
            text: display_text,
            font_size: fs,
            font_weight,
            text_align: TextAlign::Left,
            color: text_color,
        });

        if let Some(p) = preedit {
            let x0 = measure_caret_advance(text_engine, committed, caret_byte, fs, font_weight);
            let mut combined = String::with_capacity(committed.len() + p.len());
            combined.push_str(&committed[..caret_byte]);
            combined.push_str(p);
            let x1 = measure_caret_advance(text_engine, &combined, combined.len(), fs, font_weight);
            let underline_w = (x1 - x0).max(4.0);
            ops.push(PaintOp::FillRoundedRect {
                rect: LayoutRect {
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
        let caret = compute_ime_cursor_rect(
            text_engine,
            committed,
            caret_byte,
            preedit,
            fs,
            font_weight,
            is_secure,
            text_rect,
        );
        ops.push(PaintOp::FillRoundedRect {
            rect: caret,
            radius: 0.0,
            color: FOCUS_RING_COLOR,
        });
    }

    ops.push(PaintOp::PopClip);
}
