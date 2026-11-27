//! Compact sparse field-tagged binary codec for [`StylePatch`] inside `ResponsiveRules` and
//! `StateRules` `Resource` payloads.

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
use nve_codec::{ByteReader, ByteWriter, CodecError};

use crate::interaction::CursorKind;
use crate::layout_ext::{
    GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit, LayoutDirection,
    PositionMode,
};
use crate::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use crate::style_patch::{Display, FlexWrap, Overflow, StylePatch, TextAlign};
use crate::transform::Transform2D;

const TAG_END: u8 = 0;
const MAX_ITEMS: usize = 4096;

fn write_color(w: &mut ByteWriter, c: ColorRgba) {
    w.write_raw(&[c.r, c.g, c.b, c.a]);
}

fn read_color(r: &mut ByteReader<'_>) -> Result<ColorRgba, CodecError> {
    let [red, g, b, a] = r.read_array::<4>()?;
    Ok(ColorRgba { r: red, g, b, a })
}

fn write_dim(w: &mut ByteWriter, d: Dimension) {
    match d {
        Dimension::Auto => w.write_u8(0),
        Dimension::Points(v) => {
            w.write_u8(1);
            w.write_f32(v);
        }
        Dimension::Percent(v) => {
            w.write_u8(2);
            w.write_f32(v);
        }
    }
}

fn read_dim(r: &mut ByteReader<'_>) -> Result<Dimension, CodecError> {
    match r.read_u8()? {
        0 => Ok(Dimension::Auto),
        1 => Ok(Dimension::Points(r.read_f32()?)),
        2 => Ok(Dimension::Percent(r.read_f32()?)),
        other => Err(CodecError::BadEnum { ty: "Dimension", raw: i64::from(other) }),
    }
}

fn write_edges(w: &mut ByteWriter, e: EdgeInsets) {
    w.write_f32(e.top);
    w.write_f32(e.right);
    w.write_f32(e.bottom);
    w.write_f32(e.left);
}

fn read_edges(r: &mut ByteReader<'_>) -> Result<EdgeInsets, CodecError> {
    Ok(EdgeInsets {
        top: r.read_f32()?,
        right: r.read_f32()?,
        bottom: r.read_f32()?,
        left: r.read_f32()?,
    })
}

fn write_track_unit(w: &mut ByteWriter, u: GridTrackUnit) {
    match u {
        GridTrackUnit::Auto => w.write_u8(0),
        GridTrackUnit::Px(v) => {
            w.write_u8(1);
            w.write_f32(v);
        }
        GridTrackUnit::Percent(v) => {
            w.write_u8(2);
            w.write_f32(v);
        }
        GridTrackUnit::Fr(v) => {
            w.write_u8(3);
            w.write_f32(v);
        }
        GridTrackUnit::MinContent => w.write_u8(4),
        GridTrackUnit::MaxContent => w.write_u8(5),
    }
}

fn read_track_unit(r: &mut ByteReader<'_>) -> Result<GridTrackUnit, CodecError> {
    Ok(match r.read_u8()? {
        0 => GridTrackUnit::Auto,
        1 => GridTrackUnit::Px(r.read_f32()?),
        2 => GridTrackUnit::Percent(r.read_f32()?),
        3 => GridTrackUnit::Fr(r.read_f32()?),
        4 => GridTrackUnit::MinContent,
        5 => GridTrackUnit::MaxContent,
        other => return Err(CodecError::BadEnum { ty: "GridTrackUnit", raw: i64::from(other) }),
    })
}

fn write_single_track(w: &mut ByteWriter, t: GridSingleTrack) {
    match t {
        GridSingleTrack::Auto => w.write_u8(0),
        GridSingleTrack::Px(v) => {
            w.write_u8(1);
            w.write_f32(v);
        }
        GridSingleTrack::Percent(v) => {
            w.write_u8(2);
            w.write_f32(v);
        }
        GridSingleTrack::Fr(v) => {
            w.write_u8(3);
            w.write_f32(v);
        }
        GridSingleTrack::MinMax(min, max) => {
            w.write_u8(4);
            write_track_unit(w, min);
            write_track_unit(w, max);
        }
    }
}

fn read_single_track(r: &mut ByteReader<'_>) -> Result<GridSingleTrack, CodecError> {
    Ok(match r.read_u8()? {
        0 => GridSingleTrack::Auto,
        1 => GridSingleTrack::Px(r.read_f32()?),
        2 => GridSingleTrack::Percent(r.read_f32()?),
        3 => GridSingleTrack::Fr(r.read_f32()?),
        4 => GridSingleTrack::MinMax(read_track_unit(r)?, read_track_unit(r)?),
        other => return Err(CodecError::BadEnum { ty: "GridSingleTrack", raw: i64::from(other) }),
    })
}

fn write_grid_tracks(w: &mut ByteWriter, tracks: &[GridTrack]) -> Result<(), CodecError> {
    w.write_len_max(tracks.len(), MAX_ITEMS)?;
    for &t in tracks {
        match t {
            GridTrack::Auto => w.write_u8(0),
            GridTrack::Px(v) => {
                w.write_u8(1);
                w.write_f32(v);
            }
            GridTrack::Percent(v) => {
                w.write_u8(2);
                w.write_f32(v);
            }
            GridTrack::Fr(v) => {
                w.write_u8(3);
                w.write_f32(v);
            }
            GridTrack::MinMax(min, max) => {
                w.write_u8(4);
                write_track_unit(w, min);
                write_track_unit(w, max);
            }
            GridTrack::Repeat(count, track) => {
                w.write_u8(5);
                w.write_u16(count);
                write_single_track(w, track);
            }
        }
    }
    Ok(())
}

fn read_grid_tracks(r: &mut ByteReader<'_>) -> Result<Vec<GridTrack>, CodecError> {
    r.read_vec(MAX_ITEMS, 1, |r| match r.read_u8()? {
        0 => Ok(GridTrack::Auto),
        1 => Ok(GridTrack::Px(r.read_f32()?)),
        2 => Ok(GridTrack::Percent(r.read_f32()?)),
        3 => Ok(GridTrack::Fr(r.read_f32()?)),
        4 => Ok(GridTrack::MinMax(read_track_unit(r)?, read_track_unit(r)?)),
        5 => Ok(GridTrack::Repeat(r.read_u16()?, read_single_track(r)?)),
        other => Err(CodecError::BadEnum { ty: "GridTrack", raw: i64::from(other) }),
    })
}

fn write_line(w: &mut ByteWriter, l: GridLine) {
    match l {
        GridLine::Auto => w.write_u8(0),
        GridLine::Line(n) => {
            w.write_u8(1);
            w.write_i16(n);
        }
        GridLine::Span(n) => {
            w.write_u8(2);
            w.write_u16(n);
        }
    }
}

fn read_line(r: &mut ByteReader<'_>) -> Result<GridLine, CodecError> {
    match r.read_u8()? {
        0 => Ok(GridLine::Auto),
        1 => Ok(GridLine::Line(r.read_i16()?)),
        2 => Ok(GridLine::Span(r.read_u16()?)),
        other => Err(CodecError::BadEnum { ty: "GridLine", raw: i64::from(other) }),
    }
}

fn write_placement(w: &mut ByteWriter, p: GridTrackPlacement) {
    write_line(w, p.start);
    write_line(w, p.end);
}

fn read_placement(r: &mut ByteReader<'_>) -> Result<GridTrackPlacement, CodecError> {
    Ok(GridTrackPlacement {
        start: read_line(r)?,
        end: read_line(r)?,
    })
}

fn write_transform(w: &mut ByteWriter, t: Transform2D) {
    for v in [
        t.translate_x,
        t.translate_y,
        t.scale_x,
        t.scale_y,
        t.rotate_deg,
        t.origin_x,
        t.origin_y,
    ] {
        w.write_f32(v);
    }
}

fn read_transform(r: &mut ByteReader<'_>) -> Result<Transform2D, CodecError> {
    Ok(Transform2D {
        translate_x: r.read_f32()?,
        translate_y: r.read_f32()?,
        scale_x: r.read_f32()?,
        scale_y: r.read_f32()?,
        rotate_deg: r.read_f32()?,
        origin_x: r.read_f32()?,
        origin_y: r.read_f32()?,
    })
}

fn write_bg_fill(w: &mut ByteWriter, fill: &BackgroundFill) -> Result<(), CodecError> {
    match fill {
        BackgroundFill::Solid(c) => {
            w.write_u8(0);
            write_color(w, *c);
        }
        BackgroundFill::Linear(lg) => {
            w.write_u8(1);
            w.write_f32(lg.angle_deg);
            w.write_len_max(lg.stops.len(), MAX_ITEMS)?;
            for s in lg.stops.iter() {
                w.write_f32(s.offset);
                write_color(w, s.color);
            }
        }
        BackgroundFill::Radial(rg) => {
            w.write_u8(2);
            w.write_f32(rg.center_x);
            w.write_f32(rg.center_y);
            w.write_f32(rg.radius);
            w.write_len_max(rg.stops.len(), MAX_ITEMS)?;
            for s in rg.stops.iter() {
                w.write_f32(s.offset);
                write_color(w, s.color);
            }
        }
    }
    Ok(())
}

fn read_bg_fill(r: &mut ByteReader<'_>) -> Result<BackgroundFill, CodecError> {
    match r.read_u8()? {
        0 => Ok(BackgroundFill::Solid(read_color(r)?)),
        1 => {
            let angle = r.read_f32()?;
            let stops = r.read_vec(MAX_ITEMS, 8, |r| {
                Ok(GradientStop::new(r.read_f32()?, read_color(r)?))
            })?;
            Ok(BackgroundFill::Linear(LinearGradient::new(angle, stops)))
        }
        2 => {
            let cx = r.read_f32()?;
            let cy = r.read_f32()?;
            let rad = r.read_f32()?;
            let stops = r.read_vec(MAX_ITEMS, 8, |r| {
                Ok(GradientStop::new(r.read_f32()?, read_color(r)?))
            })?;
            Ok(BackgroundFill::Radial(RadialGradient::new(cx, cy, rad, stops)))
        }
        other => Err(CodecError::BadEnum { ty: "BackgroundFill", raw: i64::from(other) }),
    }
}

pub(crate) fn encode_style_patch(p: &StylePatch, w: &mut ByteWriter) -> Result<(), CodecError> {
    if let Some(v) = p.flex_direction { w.write_raw(&[1, v as u8]); }
    if let Some(v) = p.justify_content { w.write_raw(&[2, v as u8]); }
    if let Some(v) = p.align_items { w.write_raw(&[3, v as u8]); }
    if let Some(v) = p.flex_grow { w.write_u8(4); w.write_f32(v); }
    if let Some(v) = p.flex_shrink { w.write_u8(5); w.write_f32(v); }
    if let Some(v) = p.flex_basis { w.write_u8(6); write_dim(w, v); }
    if let Some(v) = p.aspect_ratio { w.write_u8(7); w.write_f32(v); }
    if let Some(v) = p.position { w.write_raw(&[8, v as u8]); }
    if let Some(v) = p.inset_top { w.write_u8(9); write_dim(w, v); }
    if let Some(v) = p.inset_right { w.write_u8(10); write_dim(w, v); }
    if let Some(v) = p.inset_bottom { w.write_u8(11); write_dim(w, v); }
    if let Some(v) = p.inset_left { w.write_u8(12); write_dim(w, v); }
    if let Some(v) = p.gap { w.write_u8(13); w.write_f32(v); }
    if let Some(v) = p.column_gap { w.write_u8(14); w.write_f32(v); }
    if let Some(v) = p.row_gap { w.write_u8(15); w.write_f32(v); }
    if let Some(ref v) = p.grid_template_columns { w.write_u8(16); write_grid_tracks(w, v)?; }
    if let Some(ref v) = p.grid_template_rows { w.write_u8(17); write_grid_tracks(w, v)?; }
    if let Some(v) = p.grid_column { w.write_u8(18); write_placement(w, v); }
    if let Some(v) = p.grid_row { w.write_u8(19); write_placement(w, v); }
    if let Some(v) = p.direction { w.write_raw(&[20, v as u8]); }
    if let Some(v) = p.padding_inline_start { w.write_u8(21); w.write_f32(v); }
    if let Some(v) = p.padding_inline_end { w.write_u8(22); w.write_f32(v); }
    if let Some(v) = p.margin_inline_start { w.write_u8(23); w.write_f32(v); }
    if let Some(v) = p.margin_inline_end { w.write_u8(24); w.write_f32(v); }
    if let Some(v) = p.transform { w.write_u8(25); write_transform(w, v); }
    if let Some(v) = p.width { w.write_u8(26); write_dim(w, v); }
    if let Some(v) = p.height { w.write_u8(27); write_dim(w, v); }
    if let Some(v) = p.min_width { w.write_u8(28); write_dim(w, v); }
    if let Some(v) = p.max_width { w.write_u8(29); write_dim(w, v); }
    if let Some(v) = p.min_height { w.write_u8(30); write_dim(w, v); }
    if let Some(v) = p.max_height { w.write_u8(31); write_dim(w, v); }
    if let Some(v) = p.padding { w.write_u8(32); write_edges(w, v); }
    if let Some(v) = p.margin { w.write_u8(33); write_edges(w, v); }
    if let Some(v) = p.background { w.write_u8(34); write_color(w, v); }
    if let Some(ref v) = p.background_fill { w.write_u8(35); write_bg_fill(w, v)?; }
    if let Some(v) = p.text_color { w.write_u8(36); write_color(w, v); }
    if let Some(v) = p.border_width { w.write_u8(37); w.write_f32(v); }
    if let Some(v) = p.border_color { w.write_u8(38); write_color(w, v); }
    if let Some(v) = p.border_radius { w.write_u8(39); w.write_f32(v); }
    if let Some(v) = p.opacity { w.write_u8(40); w.write_f32(v); }
    if let Some(v) = p.font_size { w.write_u8(41); w.write_f32(v); }
    if let Some(v) = p.z_index { w.write_u8(42); w.write_i32(v); }
    if let Some(v) = p.flex_wrap { w.write_raw(&[43, v as u8]); }
    if let Some(v) = p.overflow { w.write_raw(&[44, v as u8]); }
    if let Some(v) = p.text_align { w.write_raw(&[45, v as u8]); }
    if let Some(v) = p.font_weight { w.write_u8(46); w.write_u16(v); }
    if let Some(v) = p.display { w.write_raw(&[47, v as u8]); }
    if let Some(v) = p.cursor { w.write_raw(&[48, v as u8]); }
    w.write_u8(TAG_END);
    Ok(())
}

pub(crate) fn decode_style_patch_bytes(bytes: &[u8]) -> Option<StylePatch> {
    decode_style_patch(bytes).ok()
}

fn decode_style_patch(bytes: &[u8]) -> Result<StylePatch, CodecError> {
    let mut r = ByteReader::new(bytes);
    let mut p = StylePatch::new();
    loop {
        match r.read_u8()? {
            TAG_END => {
                r.finish()?;
                return Ok(p);
            }
            1 => p.flex_direction = Some(match r.read_u8()? {
                0 => FlexDirection::Column, 1 => FlexDirection::Row,
                2 => FlexDirection::ColumnReverse, 3 => FlexDirection::RowReverse,
                raw => return Err(CodecError::BadEnum { ty: "FlexDirection", raw: i64::from(raw) }),
            }),
            2 => p.justify_content = Some(match r.read_u8()? {
                0 => FlexJustify::FlexStart, 1 => FlexJustify::Center, 2 => FlexJustify::FlexEnd,
                3 => FlexJustify::SpaceBetween, 4 => FlexJustify::SpaceAround, 5 => FlexJustify::SpaceEvenly,
                raw => return Err(CodecError::BadEnum { ty: "FlexJustify", raw: i64::from(raw) }),
            }),
            3 => p.align_items = Some(match r.read_u8()? {
                0 => FlexAlign::Stretch, 1 => FlexAlign::FlexStart, 2 => FlexAlign::Center,
                3 => FlexAlign::FlexEnd, 4 => FlexAlign::Baseline,
                raw => return Err(CodecError::BadEnum { ty: "FlexAlign", raw: i64::from(raw) }),
            }),
            4 => p.flex_grow = Some(r.read_f32()?),
            5 => p.flex_shrink = Some(r.read_f32()?),
            6 => p.flex_basis = Some(read_dim(&mut r)?),
            7 => p.aspect_ratio = Some(r.read_f32()?),
            8 => p.position = Some(match r.read_u8()? {
                0 => PositionMode::Relative, 1 => PositionMode::Absolute, 2 => PositionMode::Sticky,
                raw => return Err(CodecError::BadEnum { ty: "PositionMode", raw: i64::from(raw) }),
            }),
            9 => p.inset_top = Some(read_dim(&mut r)?),
            10 => p.inset_right = Some(read_dim(&mut r)?),
            11 => p.inset_bottom = Some(read_dim(&mut r)?),
            12 => p.inset_left = Some(read_dim(&mut r)?),
            13 => p.gap = Some(r.read_f32()?),
            14 => p.column_gap = Some(r.read_f32()?),
            15 => p.row_gap = Some(r.read_f32()?),
            16 => p.grid_template_columns = Some(read_grid_tracks(&mut r)?.into_boxed_slice()),
            17 => p.grid_template_rows = Some(read_grid_tracks(&mut r)?.into_boxed_slice()),
            18 => p.grid_column = Some(read_placement(&mut r)?),
            19 => p.grid_row = Some(read_placement(&mut r)?),
            20 => p.direction = Some(match r.read_u8()? {
                0 => LayoutDirection::Ltr, 1 => LayoutDirection::Rtl,
                raw => return Err(CodecError::BadEnum { ty: "LayoutDirection", raw: i64::from(raw) }),
            }),
            21 => p.padding_inline_start = Some(r.read_f32()?),
            22 => p.padding_inline_end = Some(r.read_f32()?),
            23 => p.margin_inline_start = Some(r.read_f32()?),
            24 => p.margin_inline_end = Some(r.read_f32()?),
            25 => p.transform = Some(read_transform(&mut r)?),
            26 => p.width = Some(read_dim(&mut r)?),
            27 => p.height = Some(read_dim(&mut r)?),
            28 => p.min_width = Some(read_dim(&mut r)?),
            29 => p.max_width = Some(read_dim(&mut r)?),
            30 => p.min_height = Some(read_dim(&mut r)?),
            31 => p.max_height = Some(read_dim(&mut r)?),
            32 => p.padding = Some(read_edges(&mut r)?),
            33 => p.margin = Some(read_edges(&mut r)?),
            34 => p.background = Some(read_color(&mut r)?),
            35 => p.background_fill = Some(read_bg_fill(&mut r)?),
            36 => p.text_color = Some(read_color(&mut r)?),
            37 => p.border_width = Some(r.read_f32()?),
            38 => p.border_color = Some(read_color(&mut r)?),
            39 => p.border_radius = Some(r.read_f32()?),
            40 => p.opacity = Some(r.read_f32()?),
            41 => p.font_size = Some(r.read_f32()?),
            42 => p.z_index = Some(r.read_i32()?),
            43 => p.flex_wrap = Some(match r.read_u8()? {
                0 => FlexWrap::NoWrap, 1 => FlexWrap::Wrap, 2 => FlexWrap::WrapReverse,
                raw => return Err(CodecError::BadEnum { ty: "FlexWrap", raw: i64::from(raw) }),
            }),
            44 => p.overflow = Some(match r.read_u8()? {
                0 => Overflow::Visible, 1 => Overflow::Hidden, 2 => Overflow::Scroll, 3 => Overflow::Auto,
                raw => return Err(CodecError::BadEnum { ty: "Overflow", raw: i64::from(raw) }),
            }),
            45 => p.text_align = Some(match r.read_u8()? {
                0 => TextAlign::Left, 1 => TextAlign::Center, 2 => TextAlign::Right, 3 => TextAlign::Justify,
                raw => return Err(CodecError::BadEnum { ty: "TextAlign", raw: i64::from(raw) }),
            }),
            46 => p.font_weight = Some(r.read_u16()?),
            47 => p.display = Some(match r.read_u8()? {
                0 => Display::Flex, 1 => Display::None, 2 => Display::Grid,
                raw => return Err(CodecError::BadEnum { ty: "Display", raw: i64::from(raw) }),
            }),
            48 => {
                let raw = r.read_u8()?;
                p.cursor = Some(
                    CursorKind::try_from_word(u32::from(raw))
                        .ok_or(CodecError::BadEnum { ty: "CursorKind", raw: i64::from(raw) })?,
                );
            }
            raw => return Err(CodecError::BadEnum { ty: "StylePatchTag", raw: i64::from(raw) }),
        }
    }
}
