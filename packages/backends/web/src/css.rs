//! Base reset CSS, compact CSS formatting utilities, and direct `NodeStore` word-table CSS
//! emission (`write_store_node_css`).

use core::fmt::Write as _;

use nve_abi::num::{write_f32, write_i64, write_u64};
use nve_abi::{ColorRgba, Dimension, EdgeInsets};
use nve_tables::tables::{
    AlignContent, AlignItems, AlignSelf, BoxSizing, Direction, Display, FlexDirection, FlexWrap,
    GridItem, GridLayout, GridLine, GridTracks, JustifyContent, Layout, Position,
};
use nve_store::{Length, NodeId, NodeStore};

mod paint;
mod values;
#[cfg(test)]
mod values_tests;

pub use paint::{write_compositor_css, write_transform_css};
pub use values::{
    write_background_decl, write_box_shadows, write_grid_tracks, write_px, write_transform_decls,
    BackgroundValue, Keyword, WriteCss,
};

/// Compact CSS reset injected once into `<style data-nve-base>`.
///
/// Wraps structural & element-qualified defaults in `:where(...)` (zero specificity) so every
/// deduplicated `.nXXXXXXXX` class rule (`0,1,0`) cleanly overrides defaults such as
/// `display:inline`, `color`, `padding`, and `background-color`, while keeping closed `<dialog>`
/// and `[popover]` overlays hidden at higher specificity.
pub const NVE_BASE_RESET_CSS: &str = concat!(
    "@layer nve-base,nve-s,nve-m,nve-st0,nve-st1,nve-st2,nve-st3,nve-st4;",
    "@layer nve-base{",
    "*,*::before,*::after{box-sizing:border-box}",
    "html,body{margin:0;padding:0;min-height:100%}",
    ":where(.nve){display:flex;flex-direction:column;position:relative;min-width:0;min-height:0}",
    ":where(.nve-r){display:flex;flex-direction:column;width:100%;min-width:0}",
    ":where(span.nve){display:inline;color:inherit;font-size:inherit;font-weight:inherit;text-align:inherit}",
    ":where(.nve:is(strong,em,s,kbd)){display:inline}",
    ":where(br.nve){display:inline;width:auto;height:auto}",
    ":where(pre.nve>code){display:flex;flex-direction:column;width:100%;min-width:0}",
    ":where(a.nve){color:inherit;text-decoration:none}",
    ":where(button.nve){background:none;border:none;padding:0;font-family:inherit;color:inherit;text-align:inherit;cursor:pointer}",
    ":where(input.nve,textarea.nve){font-family:inherit;color:inherit;background:transparent;border:none;outline:none}",
    ":where(table.nve){display:table;border-collapse:collapse;border-style:hidden}",
    ":where(thead.nve){display:table-header-group}",
    ":where(tbody.nve){display:table-row-group}",
    ":where(tr.nve){display:table-row}",
    ":where(th.nve,td.nve){display:table-cell;vertical-align:top;width:auto}",
    ":where(th.nve){text-align:left}",
    "}",
    ".nve:is(dialog:not([open]),[popover]:not(:popover-open)){display:none}",
    "dialog.nve::backdrop{background:rgba(15,23,42,.55);backdrop-filter:blur(2px)}",
    "[dir=\"rtl\"] [data-nve-mirror=\"directional\"],[dir=\"rtl\"][data-nve-mirror=\"directional\"]{transform:scaleX(-1)}"
);

/// Canonicalizes an `f32` to deterministic IEEE-754 bits (folding `-0.0` and non-finite to `0`).
#[inline]
pub fn canonical_f32_bits(v: f32) -> u32 {
    if !v.is_finite() || v == 0.0 {
        0
    } else {
        v.to_bits()
    }
}

/// Formats a deterministic content-addressed class token `<prefix><8hex>` from the XXH64 digest.
pub fn format_content_class(prefix: &str, parts: &[&[u8]]) -> String {
    let mut h = nve_abi::Xxh64Hasher::new(0);
    for part in parts {
        h.update(part);
    }
    let h = h.digest();
    let mut out = String::with_capacity(prefix.len() + 8);
    let _ = write!(&mut out, "{prefix}{:08x}", (h ^ (h >> 32)) as u32);
    out
}

#[inline]
pub(crate) fn canonical_f32(v: f32) -> f32 {
    if !v.is_finite() || v == 0.0 {
        0.0
    } else {
        v
    }
}

#[inline]
pub(crate) fn write_compact_num(v: f32, out: &mut String) {
    write_f32(out, canonical_f32(v));
}

#[inline]
pub(crate) fn write_compact_px(v: f32, out: &mut String) {
    let v = canonical_f32(v);
    if v == 0.0 {
        out.push('0');
    } else {
        write_f32(out, v);
        out.push_str("px");
    }
}

#[inline]
pub(crate) fn write_compact_dim(d: Dimension, out: &mut String) {
    match d {
        Dimension::Auto => out.push_str("auto"),
        Dimension::Points(v) => write_compact_px(v, out),
        Dimension::Percent(v) => {
            let v = canonical_f32(v);
            if v == 0.0 {
                out.push('0');
            } else {
                write_f32(out, v);
                out.push('%');
            }
        }
    }
}

#[inline]
pub(crate) fn write_compact_length(l: Length, out: &mut String) {
    if l.is_points() {
        write_compact_px(l.value(), out);
    } else if l.is_percent() {
        let v = canonical_f32(l.value());
        if v == 0.0 {
            out.push('0');
        } else {
            write_f32(out, v);
            out.push('%');
        }
    } else {
        out.push_str("auto");
    }
}

#[inline]
pub(crate) fn write_compact_color(c: ColorRgba, out: &mut String) {
    if c.a == 0 {
        out.push_str("transparent");
    } else if c.a == 255 {
        let _ = write!(out, "#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
    } else {
        out.push_str("rgba(");
        write_u64(out, u64::from(c.r));
        out.push(',');
        write_u64(out, u64::from(c.g));
        out.push(',');
        write_u64(out, u64::from(c.b));
        out.push(',');
        write_f32(out, (f32::from(c.a) / 255.0 * 1000.0).round() / 1000.0);
        out.push(')');
    }
}

#[inline]
pub(crate) fn write_compact_edges(e: EdgeInsets, out: &mut String) {
    let t = canonical_f32(e.top);
    let r = canonical_f32(e.right);
    let b = canonical_f32(e.bottom);
    let l = canonical_f32(e.left);
    if t == r && r == b && b == l {
        write_compact_px(t, out);
    } else if t == b && r == l {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
    } else if r == l {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
        out.push(' ');
        write_compact_px(b, out);
    } else {
        write_compact_px(t, out);
        out.push(' ');
        write_compact_px(r, out);
        out.push(' ');
        write_compact_px(b, out);
        out.push(' ');
        write_compact_px(l, out);
    }
}

fn write_grid_line(l: GridLine, out: &mut String) {
    match l {
        GridLine::Auto => out.push_str("auto"),
        GridLine::Line(n) => write_i64(out, i64::from(n)),
        GridLine::Span(n) => {
            out.push_str("span ");
            write_u64(out, u64::from(n));
        }
    }
}

/// Emits structural + paint CSS declarations for `id` directly from `NodeStore` word tables
/// (excluding compositor-only `opacity`/`z-index`/`transform` when emitted inline for the 120 Hz
/// fast lane, unless `include_compositor` is true).
pub fn write_store_node_css(
    store: &NodeStore,
    id: NodeId,
    include_compositor: bool,
    out: &mut String,
) {
    if let Some(l) = store.try_row::<Layout>(id) {
        match l.display() {
            Display::Flex => {}
            Display::None => out.push_str("display:none;"),
            Display::Grid => out.push_str("display:grid;"),
            Display::Block => out.push_str("display:block;"),
            Display::Inline => out.push_str("display:inline;"),
            Display::InlineBlock => out.push_str("display:inline-block;"),
            Display::InlineFlex => out.push_str("display:inline-flex;"),
            Display::Contents => out.push_str("display:contents;"),
        }
        match l.position() {
            Position::Relative => {}
            Position::Absolute => out.push_str("position:absolute;"),
            Position::Sticky => out.push_str("position:sticky;"),
            Position::Fixed => out.push_str("position:fixed;"),
        }
        if l.box_sizing() == BoxSizing::ContentBox {
            out.push_str("box-sizing:content-box;");
        }
        match l.direction() {
            Direction::Inherit => {}
            Direction::Ltr => out.push_str("direction:ltr;"),
            Direction::Rtl => out.push_str("direction:rtl;"),
        }
        match l.flex_direction() {
            FlexDirection::Column => {}
            FlexDirection::Row => out.push_str("flex-direction:row;"),
            FlexDirection::ColumnReverse => out.push_str("flex-direction:column-reverse;"),
            FlexDirection::RowReverse => out.push_str("flex-direction:row-reverse;"),
        }
        match l.flex_wrap() {
            FlexWrap::NoWrap => {}
            FlexWrap::Wrap => out.push_str("flex-wrap:wrap;"),
            FlexWrap::WrapReverse => out.push_str("flex-wrap:wrap-reverse;"),
        }
        match l.justify_content() {
            JustifyContent::FlexStart => {}
            JustifyContent::Center => out.push_str("justify-content:center;"),
            JustifyContent::FlexEnd => out.push_str("justify-content:flex-end;"),
            JustifyContent::SpaceBetween => out.push_str("justify-content:space-between;"),
            JustifyContent::SpaceAround => out.push_str("justify-content:space-around;"),
            JustifyContent::SpaceEvenly => out.push_str("justify-content:space-evenly;"),
        }
        match l.align_items() {
            AlignItems::Stretch => {}
            AlignItems::FlexStart => out.push_str("align-items:flex-start;"),
            AlignItems::Center => out.push_str("align-items:center;"),
            AlignItems::FlexEnd => out.push_str("align-items:flex-end;"),
            AlignItems::Baseline => out.push_str("align-items:baseline;"),
        }
        match l.align_self() {
            AlignSelf::Auto => {}
            AlignSelf::Stretch => out.push_str("align-self:stretch;"),
            AlignSelf::FlexStart => out.push_str("align-self:flex-start;"),
            AlignSelf::Center => out.push_str("align-self:center;"),
            AlignSelf::FlexEnd => out.push_str("align-self:flex-end;"),
            AlignSelf::Baseline => out.push_str("align-self:baseline;"),
        }
        match l.align_content() {
            AlignContent::Stretch => {}
            AlignContent::FlexStart => out.push_str("align-content:flex-start;"),
            AlignContent::Center => out.push_str("align-content:center;"),
            AlignContent::FlexEnd => out.push_str("align-content:flex-end;"),
            AlignContent::SpaceBetween => out.push_str("align-content:space-between;"),
            AlignContent::SpaceAround => out.push_str("align-content:space-around;"),
            AlignContent::SpaceEvenly => out.push_str("align-content:space-evenly;"),
        }
        if l.flex_grow() != 0.0 {
            out.push_str("flex-grow:");
            write_compact_num(l.flex_grow(), out);
            out.push(';');
        }
        if (l.flex_shrink() - 1.0).abs() > 1e-4 {
            out.push_str("flex-shrink:");
            write_compact_num(l.flex_shrink(), out);
            out.push(';');
        }
        if !l.flex_basis().is_auto() {
            out.push_str("flex-basis:");
            write_compact_length(l.flex_basis(), out);
            out.push(';');
        }
        if !l.width().is_auto() {
            out.push_str("width:");
            write_compact_length(l.width(), out);
            out.push(';');
        }
        if !l.height().is_auto() {
            out.push_str("height:");
            write_compact_length(l.height(), out);
            out.push(';');
        }
        if !l.min_width().is_auto() {
            out.push_str("min-width:");
            write_compact_length(l.min_width(), out);
            out.push(';');
        }
        if !l.min_height().is_auto() {
            out.push_str("min-height:");
            write_compact_length(l.min_height(), out);
            out.push(';');
        }
        if !l.max_width().is_auto() {
            out.push_str("max-width:");
            write_compact_length(l.max_width(), out);
            out.push(';');
        }
        if !l.max_height().is_auto() {
            out.push_str("max-height:");
            write_compact_length(l.max_height(), out);
            out.push(';');
        }
        if let Some(ar) = l.aspect_ratio().filter(|&r| r.is_finite() && r > 0.0) {
            out.push_str("aspect-ratio:");
            write_compact_num(ar, out);
            out.push(';');
        }
        if !l.inset_top().is_auto() {
            out.push_str("top:");
            write_compact_length(l.inset_top(), out);
            out.push(';');
        }
        if !l.inset_right().is_auto() {
            out.push_str("right:");
            write_compact_length(l.inset_right(), out);
            out.push(';');
        }
        if !l.inset_bottom().is_auto() {
            out.push_str("bottom:");
            write_compact_length(l.inset_bottom(), out);
            out.push(';');
        }
        if !l.inset_left().is_auto() {
            out.push_str("left:");
            write_compact_length(l.inset_left(), out);
            out.push(';');
        }
        if !l.row_gap().is_auto() && l.row_gap().value() != 0.0 {
            out.push_str("row-gap:");
            write_compact_length(l.row_gap(), out);
            out.push(';');
        }
        if !l.column_gap().is_auto() && l.column_gap().value() != 0.0 {
            out.push_str("column-gap:");
            write_compact_length(l.column_gap(), out);
            out.push(';');
        }
        let px = |len: Length| if len.is_points() { len.value() } else { 0.0 };
        let pad = EdgeInsets {
            top: px(l.padding_top()),
            right: px(l.padding_right()),
            bottom: px(l.padding_bottom()),
            left: px(l.padding_left()),
        };
        if l.padding_inline_logical() {
            if pad.top != 0.0 {
                out.push_str("padding-top:");
                write_compact_px(pad.top, out);
                out.push(';');
            }
            if pad.bottom != 0.0 {
                out.push_str("padding-bottom:");
                write_compact_px(pad.bottom, out);
                out.push(';');
            }
            if pad.left != 0.0 {
                out.push_str("padding-inline-start:");
                write_compact_px(pad.left, out);
                out.push(';');
            }
            if pad.right != 0.0 {
                out.push_str("padding-inline-end:");
                write_compact_px(pad.right, out);
                out.push(';');
            }
        } else if pad.top != 0.0 || pad.right != 0.0 || pad.bottom != 0.0 || pad.left != 0.0 {
            out.push_str("padding:");
            write_compact_edges(pad, out);
            out.push(';');
        }
        let mar = EdgeInsets {
            top: px(l.margin_top()),
            right: px(l.margin_right()),
            bottom: px(l.margin_bottom()),
            left: px(l.margin_left()),
        };
        if l.margin_inline_logical() {
            if mar.top != 0.0 {
                out.push_str("margin-top:");
                write_compact_px(mar.top, out);
                out.push(';');
            }
            if mar.bottom != 0.0 {
                out.push_str("margin-bottom:");
                write_compact_px(mar.bottom, out);
                out.push(';');
            }
            if mar.left != 0.0 {
                out.push_str("margin-inline-start:");
                write_compact_px(mar.left, out);
                out.push(';');
            }
            if mar.right != 0.0 {
                out.push_str("margin-inline-end:");
                write_compact_px(mar.right, out);
                out.push(';');
            }
        } else if mar.top != 0.0 || mar.right != 0.0 || mar.bottom != 0.0 || mar.left != 0.0 {
            out.push_str("margin:");
            write_compact_edges(mar, out);
            out.push(';');
        }
    }

    if let Some(gl) = store.try_row::<GridLayout>(id) {
        if !gl.template_columns().is_none()
            && let Some(tracks) = store.resource::<GridTracks>(gl.template_columns())
            && !tracks.is_empty()
        {
            out.push_str("grid-template-columns:");
            tracks.write_css(out);
            out.push(';');
        }
        if !gl.template_rows().is_none()
            && let Some(tracks) = store.resource::<GridTracks>(gl.template_rows())
            && !tracks.is_empty()
        {
            out.push_str("grid-template-rows:");
            tracks.write_css(out);
            out.push(';');
        }
    }

    if let Some(gi) = store.try_row::<GridItem>(id) {
        if gi.column_start() != GridLine::Auto || gi.column_end() != GridLine::Auto {
            out.push_str("grid-column:");
            write_grid_line(gi.column_start(), out);
            if gi.column_end() != GridLine::Auto {
                out.push('/');
                write_grid_line(gi.column_end(), out);
            }
            out.push(';');
        }
        if gi.row_start() != GridLine::Auto || gi.row_end() != GridLine::Auto {
            out.push_str("grid-row:");
            write_grid_line(gi.row_start(), out);
            if gi.row_end() != GridLine::Auto {
                out.push('/');
                write_grid_line(gi.row_end(), out);
            }
            out.push(';');
        }
    }

    paint::write_visual_and_typography_css(store, id, include_compositor, out);
}
