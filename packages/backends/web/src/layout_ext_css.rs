//! Zero-serialization CSS emission, structural hashing, and DOM `dir` synchronization for
//! [`LayoutExt`] (CSS Grid, `flex-basis`, `aspect-ratio`, `position` + insets, RTL logical
//! spacing) and [`Transform2D`] in [`crate::WebBackend`].

use crate::css::{Keyword, WriteCss};
use nve_abi::Dimension;
use nve_tables::layout_ext::{LayoutExt, PositionMode};
use nve_tables::tables::Display;
use nve_tables::transform::Transform2D;

use crate::css::{canonical_f32, write_compact_dim, write_compact_num, write_compact_px};

/// Emits compact native CSS declarations for [`LayoutExt`] into `out`.
pub fn write_compact_layout_ext_into(ext: Option<&LayoutExt>, out: &mut String) {
    let Some(ext) = ext else {
        return;
    };
    match ext.display {
        Display::Flex => {}
        Display::Grid => out.push_str("display:grid;"),
        Display::None => out.push_str("display:none;"),
        Display::Block => out.push_str("display:block;"),
        Display::Inline => out.push_str("display:inline;"),
        Display::InlineBlock => out.push_str("display:inline-block;"),
        Display::InlineFlex => out.push_str("display:inline-flex;"),
        Display::Contents => out.push_str("display:contents;"),
    }
    if ext.flex_basis != Dimension::Auto {
        out.push_str("flex-basis:");
        write_compact_dim(ext.flex_basis, out);
        out.push(';');
    }
    if let Some(ratio) = ext.aspect_ratio
        && ratio.is_finite()
        && ratio > 0.0
    {
        out.push_str("aspect-ratio:");
        write_compact_num(ratio, out);
        out.push(';');
    }
    if ext.position != PositionMode::Relative || !ext.insets.is_auto() {
        out.push_str("position:");
        out.push_str(ext.position.keyword());
        out.push(';');
    }
    for (prop, dim) in [
        ("top:", ext.insets.top),
        ("right:", ext.insets.right),
        ("bottom:", ext.insets.bottom),
        ("left:", ext.insets.left),
    ] {
        if dim != Dimension::Auto {
            out.push_str(prop);
            write_compact_dim(dim, out);
            out.push(';');
        }
    }
    if let Some(ref grid) = ext.grid {
        if !grid.template_columns.is_empty() {
            out.push_str("grid-template-columns:");
            crate::css::write_grid_tracks(grid.template_columns.iter().copied(), out);
            out.push(';');
        }
        if !grid.template_rows.is_empty() {
            out.push_str("grid-template-rows:");
            crate::css::write_grid_tracks(grid.template_rows.iter().copied(), out);
            out.push(';');
        }
        if let Some(cg) = grid.column_gap {
            out.push_str("column-gap:");
            write_compact_px(canonical_f32(cg).max(0.0), out);
            out.push(';');
        }
        if let Some(rg) = grid.row_gap {
            out.push_str("row-gap:");
            write_compact_px(canonical_f32(rg).max(0.0), out);
            out.push(';');
        }
    }
    if !ext.grid_item.column.is_auto() {
        out.push_str("grid-column:");
        ext.grid_item.column.write_css(out);
        out.push(';');
    }
    if !ext.grid_item.row.is_auto() {
        out.push_str("grid-row:");
        ext.grid_item.row.write_css(out);
        out.push(';');
    }
    if let Some(v) = ext.padding_inline.start {
        out.push_str("padding-inline-start:");
        write_compact_px(canonical_f32(v).max(0.0), out);
        out.push(';');
    }
    if let Some(v) = ext.padding_inline.end {
        out.push_str("padding-inline-end:");
        write_compact_px(canonical_f32(v).max(0.0), out);
        out.push(';');
    }
    if let Some(v) = ext.margin_inline.start {
        out.push_str("margin-inline-start:");
        write_compact_px(canonical_f32(v), out);
        out.push(';');
    }
    if let Some(v) = ext.margin_inline.end {
        out.push_str("margin-inline-end:");
        write_compact_px(canonical_f32(v), out);
        out.push(';');
    }
}

/// Emits compact native CSS `transform:` (and optional `transform-origin:`) declarations into `out`.
pub fn write_compact_transform_into(tf: Option<Transform2D>, out: &mut String) {
    if let Some(tf) = tf {
        crate::css::write_transform_decls(&tf, false, out);
    }
}



