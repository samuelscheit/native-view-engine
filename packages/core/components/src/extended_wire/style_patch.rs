//! Sparse field-tagged binary codec for [`StylePatch`] (used only by cross-boundary `BatchSink`
//! wire transport for `StateStyleRule` and `ResponsiveRule`).

use nve_abi::{FlexAlign, FlexDirection, FlexJustify};

use crate::interaction::CursorKind;
use crate::style_patch::{FlexWrap, Overflow, StylePatch, TextAlign};

use super::{
    decode_display, decode_layout_direction, decode_position_mode, read_background_fill,
    read_color, read_dim, read_edges, read_grid_tracks, read_placement, read_transform,
    write_background_fill, write_color, write_dim, write_edges, write_grid_tracks, write_placement,
    write_transform, WireReader,
};

const TAG_END: u8 = 0;

/// Encodes a sparse [`StylePatch`] into `out`, emitting only set fields followed by `TAG_END`.
pub fn encode_style_patch(p: &StylePatch, out: &mut Vec<u8>) {
    if let Some(v) = p.flex_direction {
        out.extend_from_slice(&[1, v as u8]);
    }
    if let Some(v) = p.justify_content {
        out.extend_from_slice(&[2, v as u8]);
    }
    if let Some(v) = p.align_items {
        out.extend_from_slice(&[3, v as u8]);
    }
    if let Some(v) = p.flex_grow {
        out.push(4);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.flex_shrink {
        out.push(5);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.flex_basis {
        out.push(6);
        write_dim(out, v);
    }
    if let Some(v) = p.aspect_ratio {
        out.push(7);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.position {
        out.extend_from_slice(&[8, v as u8]);
    }
    if let Some(v) = p.inset_top {
        out.push(9);
        write_dim(out, v);
    }
    if let Some(v) = p.inset_right {
        out.push(10);
        write_dim(out, v);
    }
    if let Some(v) = p.inset_bottom {
        out.push(11);
        write_dim(out, v);
    }
    if let Some(v) = p.inset_left {
        out.push(12);
        write_dim(out, v);
    }
    if let Some(v) = p.gap {
        out.push(13);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.column_gap {
        out.push(14);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.row_gap {
        out.push(15);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(ref v) = p.grid_template_columns {
        out.push(16);
        write_grid_tracks(out, v);
    }
    if let Some(ref v) = p.grid_template_rows {
        out.push(17);
        write_grid_tracks(out, v);
    }
    if let Some(v) = p.grid_column {
        out.push(18);
        write_placement(out, v);
    }
    if let Some(v) = p.grid_row {
        out.push(19);
        write_placement(out, v);
    }
    if let Some(v) = p.direction {
        out.extend_from_slice(&[20, v as u8]);
    }
    if let Some(v) = p.padding_inline_start {
        out.push(21);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.padding_inline_end {
        out.push(22);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.margin_inline_start {
        out.push(23);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.margin_inline_end {
        out.push(24);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.transform {
        out.push(25);
        write_transform(out, v);
    }
    if let Some(v) = p.width {
        out.push(26);
        write_dim(out, v);
    }
    if let Some(v) = p.height {
        out.push(27);
        write_dim(out, v);
    }
    if let Some(v) = p.min_width {
        out.push(28);
        write_dim(out, v);
    }
    if let Some(v) = p.max_width {
        out.push(29);
        write_dim(out, v);
    }
    if let Some(v) = p.min_height {
        out.push(30);
        write_dim(out, v);
    }
    if let Some(v) = p.max_height {
        out.push(31);
        write_dim(out, v);
    }
    if let Some(v) = p.padding {
        out.push(32);
        write_edges(out, v);
    }
    if let Some(v) = p.margin {
        out.push(33);
        write_edges(out, v);
    }
    if let Some(v) = p.background {
        out.push(34);
        write_color(out, v);
    }
    if let Some(ref v) = p.background_fill {
        out.push(35);
        write_background_fill(out, v);
    }
    if let Some(v) = p.text_color {
        out.push(36);
        write_color(out, v);
    }
    if let Some(v) = p.border_width {
        out.push(37);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.border_color {
        out.push(38);
        write_color(out, v);
    }
    if let Some(v) = p.border_radius {
        out.push(39);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.opacity {
        out.push(40);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.font_size {
        out.push(41);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.z_index {
        out.push(42);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.flex_wrap {
        out.extend_from_slice(&[43, v as u8]);
    }
    if let Some(v) = p.overflow {
        out.extend_from_slice(&[44, v as u8]);
    }
    if let Some(v) = p.text_align {
        out.extend_from_slice(&[45, v as u8]);
    }
    if let Some(v) = p.font_weight {
        out.push(46);
        out.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = p.display {
        out.extend_from_slice(&[47, v as u8]);
    }
    if let Some(v) = p.cursor {
        out.extend_from_slice(&[48, v as u8]);
    }
    out.push(TAG_END);
}

/// Decodes a sparse [`StylePatch`] from `r` up to `TAG_END`.
pub(crate) fn decode_style_patch(r: &mut WireReader<'_>) -> Option<StylePatch> {
    let mut p = StylePatch::new();
    loop {
        match r.read_u8()? {
            TAG_END => return Some(p),
            1 => {
                p.flex_direction = Some(match r.read_u8()? {
                    0 => FlexDirection::Column,
                    1 => FlexDirection::Row,
                    2 => FlexDirection::ColumnReverse,
                    3 => FlexDirection::RowReverse,
                    _ => return None,
                });
            }
            2 => {
                p.justify_content = Some(match r.read_u8()? {
                    0 => FlexJustify::FlexStart,
                    1 => FlexJustify::Center,
                    2 => FlexJustify::FlexEnd,
                    3 => FlexJustify::SpaceBetween,
                    4 => FlexJustify::SpaceAround,
                    5 => FlexJustify::SpaceEvenly,
                    _ => return None,
                });
            }
            3 => {
                p.align_items = Some(match r.read_u8()? {
                    0 => FlexAlign::Stretch,
                    1 => FlexAlign::FlexStart,
                    2 => FlexAlign::Center,
                    3 => FlexAlign::FlexEnd,
                    4 => FlexAlign::Baseline,
                    _ => return None,
                });
            }
            4 => p.flex_grow = Some(r.read_f32()?),
            5 => p.flex_shrink = Some(r.read_f32()?),
            6 => p.flex_basis = Some(read_dim(r)?),
            7 => p.aspect_ratio = Some(r.read_f32()?),
            8 => p.position = Some(decode_position_mode(r.read_u8()?)?),
            9 => p.inset_top = Some(read_dim(r)?),
            10 => p.inset_right = Some(read_dim(r)?),
            11 => p.inset_bottom = Some(read_dim(r)?),
            12 => p.inset_left = Some(read_dim(r)?),
            13 => p.gap = Some(r.read_f32()?),
            14 => p.column_gap = Some(r.read_f32()?),
            15 => p.row_gap = Some(r.read_f32()?),
            16 => p.grid_template_columns = Some(read_grid_tracks(r)?.into_boxed_slice()),
            17 => p.grid_template_rows = Some(read_grid_tracks(r)?.into_boxed_slice()),
            18 => p.grid_column = Some(read_placement(r)?),
            19 => p.grid_row = Some(read_placement(r)?),
            20 => p.direction = Some(decode_layout_direction(r.read_u8()?)?),
            21 => p.padding_inline_start = Some(r.read_f32()?),
            22 => p.padding_inline_end = Some(r.read_f32()?),
            23 => p.margin_inline_start = Some(r.read_f32()?),
            24 => p.margin_inline_end = Some(r.read_f32()?),
            25 => p.transform = Some(read_transform(r)?),
            26 => p.width = Some(read_dim(r)?),
            27 => p.height = Some(read_dim(r)?),
            28 => p.min_width = Some(read_dim(r)?),
            29 => p.max_width = Some(read_dim(r)?),
            30 => p.min_height = Some(read_dim(r)?),
            31 => p.max_height = Some(read_dim(r)?),
            32 => p.padding = Some(read_edges(r)?),
            33 => p.margin = Some(read_edges(r)?),
            34 => p.background = Some(read_color(r)?),
            35 => p.background_fill = Some(read_background_fill(r)?),
            36 => p.text_color = Some(read_color(r)?),
            37 => p.border_width = Some(r.read_f32()?),
            38 => p.border_color = Some(read_color(r)?),
            39 => p.border_radius = Some(r.read_f32()?),
            40 => p.opacity = Some(r.read_f32()?),
            41 => p.font_size = Some(r.read_f32()?),
            42 => p.z_index = Some(r.read_i32()?),
            43 => {
                p.flex_wrap = Some(match r.read_u8()? {
                    0 => FlexWrap::NoWrap,
                    1 => FlexWrap::Wrap,
                    2 => FlexWrap::WrapReverse,
                    _ => return None,
                });
            }
            44 => {
                p.overflow = Some(match r.read_u8()? {
                    0 => Overflow::Visible,
                    1 => Overflow::Hidden,
                    2 => Overflow::Scroll,
                    3 => Overflow::Auto,
                    _ => return None,
                });
            }
            45 => {
                p.text_align = Some(match r.read_u8()? {
                    0 => TextAlign::Left,
                    1 => TextAlign::Center,
                    2 => TextAlign::Right,
                    3 => TextAlign::Justify,
                    _ => return None,
                });
            }
            46 => p.font_weight = Some(r.read_u16()?),
            47 => p.display = Some(decode_display(r.read_u8()?)?),
            48 => p.cursor = Some(CursorKind::from_u8(r.read_u8()?)?),
            _ => return None,
        }
    }
}
