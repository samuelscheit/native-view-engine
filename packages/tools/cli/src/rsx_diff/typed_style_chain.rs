//! Scanner for `nve_style: TypedStyle::...` literal builder chains on native RSX elements.
//!
//! Only scans chains rooted at `[path::]* TypedStyle::{new, row, col}()` followed purely by
//! `.method(args...)` calls. Uses a single caller-owned scratch buffer (zero per-element
//! allocations) and suppresses earlier overridden setters in-place.

use super::attrs::{AttrSpec, lookup_typed_style_arg, prop_bits};
use super::lexer::Lex;

/// Maximum arguments inspected on any patchable `TypedStyle` method (`padding_xy`, `margin_xy`, `border`).
const MAX_CHAIN_ARGS: usize = 2;

/// Candidate argument span extracted from one `TypedStyle` builder chain.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChainArgSpan {
    pub(super) start: u32,
    pub(super) end: u32,
    pub(super) spec: &'static AttrSpec,
}

/// Classification of a non-patchable method inside a `TypedStyle` chain.
///
/// Unrecognized methods (e.g. extension trait helpers like `StudioStyleExt::{outline, stretch_col}`)
/// return [`OpaqueEffect::Unknown`] so the scanner fails closed instead of diverging from fresh render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpaqueEffect {
    /// Known `TypedStyle` method writing only tables disjoint from every Tier-1 patchable slot.
    Disjoint,
    /// Known overlapping `TypedStyle` / `SurfaceStyleExt` method shadowing earlier setters in `bits`.
    Shadow(u64),
    /// Known `layout_ext` logical inline-axis method that overrides physical `bits` in BOTH
    /// directions (because `TypedStyle::apply_layout_layer` applies `layout_ext` after `layout`).
    Bidirectional(u64),
    /// `.merge` / `.merged` / `.merge_opt`: shadows every earlier setter AND may carry `layout_ext`
    /// logical inline-axis or grid-gap fields that override later physical `layout` setters.
    Merge,
    /// Unrecognized method (extension trait or unknown helper): fail closed across the whole chain.
    Unknown,
}

fn classify_opaque_method(method: &str) -> OpaqueEffect {
    use prop_bits as B;
    match method {
        // Explicit allowlist of known non-overlapping `TypedStyle` methods:
        "font_family"
        | "box_shadows"
        | "background_fill"
        | "linear_gradient"
        | "radial_gradient"
        | "transform"
        | "transform_origin"
        | "translate"
        | "scale"
        | "scale_xy"
        | "rotate"
        | "object_fit"
        | "show_on"
        | "at"
        | "at_with"
        | "on_compact"
        | "on_medium"
        | "on_expanded"
        | "on_state"
        | "on_state_with"
        | "on_hover"
        | "on_hover_with"
        | "on_active"
        | "on_active_with"
        | "on_focus"
        | "on_focus_with"
        | "on_focus_visible"
        | "on_focus_visible_with"
        | "on_disabled"
        | "on_disabled_with"
        | "grid_cols"
        | "grid_rows"
        | "grid_column"
        | "grid_row"
        | "grid_column_span"
        | "grid_row_span"
        | "direction"
        | "rtl"
        | "ltr" => OpaqueEffect::Disjoint,

        // Known overlapping `TypedStyle` / `SurfaceStyleExt` shorthands & merges:
        "merge" | "merged" | "merge_opt" => OpaqueEffect::Merge,
        "surface" => OpaqueEffect::Shadow(
            B::BG | B::BORDER_WIDTH | B::BORDER_COLOR | B::BORDER_RADIUS | B::OPACITY,
        ),
        "text_style" => OpaqueEffect::Shadow(
            B::TEXT_COLOR | B::FONT_SIZE | B::FONT_WEIGHT | B::TEXT_ALIGN | B::OPACITY,
        ),
        "width_auto" => OpaqueEffect::Shadow(B::WIDTH),
        "height_auto" => OpaqueEffect::Shadow(B::HEIGHT),
        "flex_row" | "flex_col" | "flex_row_reverse" | "flex_col_reverse" => {
            OpaqueEffect::Shadow(B::FLEX_DIR)
        }
        "flex_1" => OpaqueEffect::Shadow(B::FLEX_SHORTHAND),
        "position_relative" | "position_absolute" | "position_sticky" => {
            OpaqueEffect::Shadow(B::POSITION)
        }
        "display_grid" => OpaqueEffect::Shadow(B::DISPLAY),
        "padding_insets" => OpaqueEffect::Shadow(B::PAD_ALL),
        "margin_insets" => OpaqueEffect::Shadow(B::MAR_ALL),
        // `apply_layout_layer` sets `padding_inline_logical(true)` and writes `layout_ext` inline
        // edges AFTER physical `layout` padding/margin regardless of builder call order.
        "padding_inline_start" | "padding_inline_end" => OpaqueEffect::Bidirectional(B::PAD_X),
        "margin_inline_start" | "margin_inline_end" => OpaqueEffect::Bidirectional(B::MAR_X),

        // Any other method (e.g. extension trait helper) fails closed.
        _ => OpaqueEffect::Unknown,
    }
}

/// Scans `lex[start..end]` as a `TypedStyle::{new,row,col}().method(...)...` builder chain into
/// `steps` (cleared on entry), suppressing any setter shadowed by a later setter, shorthand,
/// `.merge*`, or `layout_ext` precedence rule, and failing closed on any unknown method.
///
/// Returns `true` iff `lex[start..end]` was a valid known `TypedStyle` builder chain (kept spans
/// live in `steps.iter().filter_map(|(s, _)| *s)` in source order). Returns `false` for non-chain
/// values or unknown extension methods so the caller falls back to scanning for nested `rsx!`.
pub(super) fn scan_typed_style_chain(
    lex: Lex<'_>,
    start: usize,
    end: usize,
    steps: &mut Vec<(Option<ChainArgSpan>, u64)>,
) -> bool {
    steps.clear();
    let Some(mut cur) = match_typed_style_ctor(lex, start, end) else {
        return false;
    };

    // Bits written by `layout_ext` in `apply_layout_layer` AFTER `layout` regardless of chain order.
    let mut always_shadowed = 0u64;
    let mut saw_ext_gap = false;

    while cur < end {
        if !lex.is_punct(cur, b'.') {
            steps.clear();
            return false;
        }
        cur += 1;
        let Some(method) = lex.ident(cur) else {
            steps.clear();
            return false;
        };
        cur += 1;
        if cur >= end || !lex.is_punct(cur, b'(') {
            steps.clear();
            return false;
        }
        let close = lex.close(cur);
        if close >= end {
            steps.clear();
            return false;
        }

        if lookup_typed_style_arg(method, 0).is_some() {
            let Some((args, argc)) = split_chain_args(lex, cur + 1, close) else {
                steps.clear();
                return false;
            };
            if argc == 0 {
                steps.clear();
                return false;
            }
            if matches!(method, "row_gap" | "column_gap") {
                saw_ext_gap = true;
            }
            for (idx, &(a_start, a_end)) in args[..argc].iter().enumerate() {
                let Some(spec) = lookup_typed_style_arg(method, idx) else {
                    steps.clear();
                    return false;
                };
                // `apply_layout_layer` applies `ext.grid.{row,column}_gap` after `layout.gap`, so
                // a `gap(...)` following `row_gap`/`column_gap` cannot safely Tier-1 patch both axes.
                let span = if method == "gap" && saw_ext_gap {
                    None
                } else {
                    Some(ChainArgSpan {
                        start: a_start as u32,
                        end: a_end as u32,
                        spec,
                    })
                };
                steps.push((span, spec.bits));
            }
        } else {
            match classify_opaque_method(method) {
                OpaqueEffect::Disjoint => {}
                OpaqueEffect::Shadow(bits) => steps.push((None, bits)),
                OpaqueEffect::Bidirectional(bits) => {
                    always_shadowed |= bits;
                    steps.push((None, bits));
                }
                OpaqueEffect::Merge => {
                    always_shadowed |= prop_bits::PAD_X | prop_bits::MAR_X | prop_bits::GAP;
                    steps.push((None, prop_bits::ALL));
                }
                OpaqueEffect::Unknown => {
                    steps.clear();
                    return false;
                }
            }
        }
        cur = close + 1;
    }

    // Reverse pass in-place: clear any setter shadowed by a later call, `.merge*`, or bidirectional
    // `layout_ext` inline-axis override.
    let mut later_written = 0u64;
    for (span, bits) in steps.iter_mut().rev() {
        if (*bits & (later_written | always_shadowed)) != 0 {
            *span = None;
        }
        later_written |= *bits;
    }
    true
}

/// Matches `[path::]* TypedStyle :: (new | row | col) ()`, returning the token index after `)`.
fn match_typed_style_ctor(lex: Lex<'_>, start: usize, end: usize) -> Option<usize> {
    let mut cur = start;
    if cur + 1 < end && lex.is_punct(cur, b':') && lex.is_punct(cur + 1, b':') {
        cur += 2;
    }
    let mut segs = ("", "");
    loop {
        let id = lex.ident(cur)?;
        segs = (segs.1, id);
        cur += 1;
        if cur + 1 < end && lex.is_punct(cur, b':') && lex.is_punct(cur + 1, b':') {
            cur += 2;
            continue;
        }
        break;
    }
    if segs.0 != "TypedStyle" || !matches!(segs.1, "new" | "row" | "col") {
        return None;
    }
    if cur >= end || !lex.is_punct(cur, b'(') {
        return None;
    }
    let close = lex.close(cur);
    if close >= end || close != cur + 1 {
        return None;
    }
    Some(close + 1)
}

fn split_chain_args(
    lex: Lex<'_>,
    start: usize,
    end: usize,
) -> Option<([(usize, usize); MAX_CHAIN_ARGS], usize)> {
    let mut out = [(0usize, 0usize); MAX_CHAIN_ARGS];
    let mut argc = 0usize;
    let mut arg_start = start;
    let mut cur = start;
    loop {
        let at_end = cur >= end;
        if at_end || lex.is_punct(cur, b',') {
            if cur > arg_start {
                if argc >= MAX_CHAIN_ARGS {
                    return None;
                }
                out[argc] = (arg_start, cur);
                argc += 1;
            } else if !at_end || (argc == 0 && start != end) {
                return None;
            }
            if at_end {
                return Some((out, argc));
            }
            cur += 1;
            arg_start = cur;
            continue;
        }
        let opens = lex.is_punct(cur, b'(') || lex.is_punct(cur, b'[') || lex.is_punct(cur, b'{');
        cur = if opens { lex.close(cur) + 1 } else { cur + 1 };
    }
}
