//! Zero-serialization CSS emission, structural hashing, and DOM `dir` synchronization for
//! [`LayoutExt`] (CSS Grid, `flex-basis`, `aspect-ratio`, `position` + insets, RTL logical
//! spacing) and [`Transform2D`] in [`crate::WebBackend`].

use nve_abi::{Dimension, NodeId};
use nve_components::layout_ext::{
    GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit, LayoutDirection,
    LayoutExt, PositionMode,
};
use nve_components::style_patch::Display;
use nve_components::transform::Transform2D;

use crate::css::{canonical_f32, canonical_f32_bits, write_compact_dim, write_compact_num, write_compact_px};
use crate::WebBackend;

const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fnv_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV1A_PRIME);
    }
    h
}

#[inline]
fn fnv_u32(h: u64, v: u32) -> u64 {
    fnv_bytes(h, &v.to_le_bytes())
}

#[inline]
fn fnv_f32(h: u64, v: f32) -> u64 {
    fnv_u32(h, canonical_f32_bits(v))
}

#[inline]
fn fnv_dim(h: u64, d: Dimension) -> u64 {
    match d {
        Dimension::Auto => fnv_bytes(h, &[0]),
        Dimension::Points(v) => fnv_f32(fnv_bytes(h, &[1]), v),
        Dimension::Percent(v) => fnv_f32(fnv_bytes(h, &[2]), v),
    }
}

#[inline]
fn fnv_unit(h: u64, u: GridTrackUnit) -> u64 {
    match u {
        GridTrackUnit::Auto => fnv_bytes(h, &[0]),
        GridTrackUnit::Px(v) => fnv_f32(fnv_bytes(h, &[1]), v),
        GridTrackUnit::Percent(v) => fnv_f32(fnv_bytes(h, &[2]), v),
        GridTrackUnit::Fr(v) => fnv_f32(fnv_bytes(h, &[3]), v),
        GridTrackUnit::MinContent => fnv_bytes(h, &[4]),
        GridTrackUnit::MaxContent => fnv_bytes(h, &[5]),
    }
}

#[inline]
fn fnv_single_track(h: u64, t: GridSingleTrack) -> u64 {
    match t {
        GridSingleTrack::Auto => fnv_bytes(h, &[0]),
        GridSingleTrack::Px(v) => fnv_f32(fnv_bytes(h, &[1]), v),
        GridSingleTrack::Percent(v) => fnv_f32(fnv_bytes(h, &[2]), v),
        GridSingleTrack::Fr(v) => fnv_f32(fnv_bytes(h, &[3]), v),
        GridSingleTrack::MinMax(min, max) => fnv_unit(fnv_unit(fnv_bytes(h, &[4]), min), max),
    }
}

#[inline]
fn fnv_track(h: u64, t: GridTrack) -> u64 {
    match t {
        GridTrack::Auto => fnv_bytes(h, &[0]),
        GridTrack::Px(v) => fnv_f32(fnv_bytes(h, &[1]), v),
        GridTrack::Percent(v) => fnv_f32(fnv_bytes(h, &[2]), v),
        GridTrack::Fr(v) => fnv_f32(fnv_bytes(h, &[3]), v),
        GridTrack::MinMax(min, max) => fnv_unit(fnv_unit(fnv_bytes(h, &[4]), min), max),
        GridTrack::Repeat(c, s) => fnv_single_track(fnv_bytes(fnv_bytes(h, &[5]), &c.to_le_bytes()), s),
    }
}

#[inline]
fn fnv_grid_line(h: u64, line: GridLine) -> u64 {
    match line {
        GridLine::Auto => fnv_bytes(h, &[0]),
        GridLine::Line(n) => fnv_bytes(fnv_bytes(h, &[1]), &n.to_le_bytes()),
        GridLine::Span(n) => fnv_bytes(fnv_bytes(h, &[2]), &n.to_le_bytes()),
    }
}

#[inline]
fn fnv_placement(h: u64, p: GridTrackPlacement) -> u64 {
    fnv_grid_line(fnv_grid_line(h, p.start), p.end)
}

/// Emits compact native CSS declarations for [`LayoutExt`] into `out`.
pub fn write_compact_layout_ext_into(ext: Option<&LayoutExt>, out: &mut String) {
    let Some(ext) = ext else {
        return;
    };
    match ext.display {
        Display::Flex => {}
        Display::Grid => out.push_str("display:grid;"),
        Display::None => out.push_str("display:none;"),
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
        out.push_str(ext.position.as_css());
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
            GridTrack::write_css_list_into(&grid.template_columns, out);
            out.push(';');
        }
        if !grid.template_rows.is_empty() {
            out.push_str("grid-template-rows:");
            GridTrack::write_css_list_into(&grid.template_rows, out);
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
        ext.grid_item.column.write_css_into(out);
        out.push(';');
    }
    if !ext.grid_item.row.is_auto() {
        out.push_str("grid-row:");
        ext.grid_item.row.write_css_into(out);
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
        tf.write_css_decls_into(false, out);
    }
}

/// Mixes [`LayoutExt`] and [`Transform2D`] into the deterministic structural FNV-1a64 hash.
pub(crate) fn hash_layout_ext_and_transform(
    mut h: u64,
    ext: Option<&LayoutExt>,
    tf: Option<Transform2D>,
) -> u64 {
    if let Some(ext) = ext
        && !ext.is_default()
    {
        h = fnv_bytes(h, &[1, ext.display as u8, ext.position as u8]);
        h = fnv_dim(h, ext.flex_basis);
        h = match ext.aspect_ratio {
            Some(r) if r.is_finite() && r > 0.0 => fnv_f32(fnv_bytes(h, &[1]), r),
            _ => fnv_bytes(h, &[0]),
        };
        h = fnv_dim(h, ext.insets.top);
        h = fnv_dim(h, ext.insets.right);
        h = fnv_dim(h, ext.insets.bottom);
        h = fnv_dim(h, ext.insets.left);
        h = fnv_opt_f32(h, ext.padding_inline.start);
        h = fnv_opt_f32(h, ext.padding_inline.end);
        h = fnv_opt_f32(h, ext.margin_inline.start);
        h = fnv_opt_f32(h, ext.margin_inline.end);
        if let Some(ref grid) = ext.grid {
            h = fnv_u32(h, grid.template_columns.len() as u32);
            for &t in &grid.template_columns {
                h = fnv_track(h, t);
            }
            h = fnv_u32(h, grid.template_rows.len() as u32);
            for &t in &grid.template_rows {
                h = fnv_track(h, t);
            }
            h = fnv_opt_f32(h, grid.column_gap);
            h = fnv_opt_f32(h, grid.row_gap);
        } else {
            h = fnv_bytes(h, &[0]);
        }
        h = fnv_placement(h, ext.grid_item.column);
        h = fnv_placement(h, ext.grid_item.row);
    }
    if let Some(tf) = tf
        && (!tf.is_identity() || tf.has_custom_origin())
    {
        h = fnv_bytes(h, &[2]);
        h = fnv_f32(h, tf.translate_x);
        h = fnv_f32(h, tf.translate_y);
        h = fnv_f32(h, tf.scale_x);
        h = fnv_f32(h, tf.scale_y);
        h = fnv_f32(h, tf.rotate_deg);
        h = fnv_f32(h, tf.origin_x);
        h = fnv_f32(h, tf.origin_y);
    }
    h
}

#[inline]
fn fnv_opt_f32(h: u64, v: Option<f32>) -> u64 {
    match v {
        Some(x) => fnv_f32(fnv_bytes(h, &[1]), x),
        None => fnv_bytes(h, &[0]),
    }
}

/// Returns the explicit HTML `dir` attribute value (`"rtl"` / `"ltr"`) for a node, if set.
#[inline]
pub(crate) fn node_dir_attr(ext: Option<&LayoutExt>) -> Option<&'static str> {
    ext.and_then(|e| e.direction).map(LayoutDirection::as_html_dir)
}

impl WebBackend {
    /// Directly attaches or clears zero-serialization [`LayoutExt`] on `id`.
    pub fn set_node_layout_ext(&mut self, id: NodeId, ext: Option<LayoutExt>) {
        let next = ext.filter(|e| !e.is_default()).map(Box::new);
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.layout_ext != next {
            node.layout_ext = next;
            #[cfg(target_arch = "wasm32")]
            self.sync_dom_dir_attr(id);
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    /// Directly attaches or clears zero-serialization [`Transform2D`] on `id`.
    pub fn set_node_transform(&mut self, id: NodeId, transform: Option<Transform2D>) {
        let Some(node) = self.nodes.get_mut(&id) else {
            return;
        };
        if node.transform != transform {
            node.transform = transform;
            self.flush_or_queue_responsive_dirty(vec![id]);
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn sync_dom_dir_attr(&self, node_id: NodeId) {
        let Some(dom_el) = self.dom_nodes.get(&node_id) else {
            return;
        };
        let dir = self
            .nodes
            .get(&node_id)
            .and_then(|n| node_dir_attr(n.layout_ext.as_deref()));
        match dir {
            Some(val) => {
                if dom_el.get_attribute("dir").as_deref() != Some(val) {
                    let _ = dom_el.set_attribute("dir", val);
                }
            }
            None => {
                if dom_el.has_attribute("dir") {
                    let _ = dom_el.remove_attribute("dir");
                }
            }
        }
    }
}
