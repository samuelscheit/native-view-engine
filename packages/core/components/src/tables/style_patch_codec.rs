//! Compact sparse field-tagged binary codec for [`StylePatch`] inside `ResponsiveRules` and
//! `StateRules` `Resource` payloads.

use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};

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

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Option<&'a [u8; N]> {
        if self.0.len() < N {
            return None;
        }
        let (h, t) = self.0.split_at(N);
        self.0 = t;
        h.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> { Some(self.take::<1>()?[0]) }
    fn u16(&mut self) -> Option<u16> { Some(u16::from_le_bytes(*self.take::<2>()?)) }
    fn i16(&mut self) -> Option<i16> { Some(i16::from_le_bytes(*self.take::<2>()?)) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(*self.take::<4>()?)) }
    fn i32(&mut self) -> Option<i32> { Some(i32::from_le_bytes(*self.take::<4>()?)) }
    fn f32(&mut self) -> Option<f32> { Some(f32::from_le_bytes(*self.take::<4>()?)) }
    fn count(&mut self) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= MAX_ITEMS).then_some(n)
    }
}

#[inline]
fn canon_f32_bytes(v: f32) -> [u8; 4] {
    let v = if v.is_nan() {
        f32::NAN
    } else if v == 0.0 {
        0.0
    } else {
        v
    };
    v.to_le_bytes()
}

fn write_color(out: &mut Vec<u8>, c: ColorRgba) {
    out.extend_from_slice(&[c.r, c.g, c.b, c.a]);
}
fn read_color(r: &mut Reader<'_>) -> Option<ColorRgba> {
    let [red, g, b, a] = *r.take::<4>()?;
    Some(ColorRgba { r: red, g, b, a })
}

fn write_dim(out: &mut Vec<u8>, d: Dimension) {
    match d {
        Dimension::Auto => out.push(0),
        Dimension::Points(v) => {
            out.push(1);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        Dimension::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
    }
}
fn read_dim(r: &mut Reader<'_>) -> Option<Dimension> {
    match r.u8()? {
        0 => Some(Dimension::Auto),
        1 => Some(Dimension::Points(r.f32()?)),
        2 => Some(Dimension::Percent(r.f32()?)),
        _ => None,
    }
}

fn write_edges(out: &mut Vec<u8>, e: EdgeInsets) {
    out.extend_from_slice(&canon_f32_bytes(e.top));
    out.extend_from_slice(&canon_f32_bytes(e.right));
    out.extend_from_slice(&canon_f32_bytes(e.bottom));
    out.extend_from_slice(&canon_f32_bytes(e.left));
}
fn read_edges(r: &mut Reader<'_>) -> Option<EdgeInsets> {
    Some(EdgeInsets {
        top: r.f32()?,
        right: r.f32()?,
        bottom: r.f32()?,
        left: r.f32()?,
    })
}

fn write_track_unit(out: &mut Vec<u8>, u: GridTrackUnit) {
    match u {
        GridTrackUnit::Auto => out.push(0),
        GridTrackUnit::Px(v) => {
            out.push(1);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridTrackUnit::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridTrackUnit::Fr(v) => {
            out.push(3);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridTrackUnit::MinContent => out.push(4),
        GridTrackUnit::MaxContent => out.push(5),
    }
}
fn read_track_unit(r: &mut Reader<'_>) -> Option<GridTrackUnit> {
    Some(match r.u8()? {
        0 => GridTrackUnit::Auto,
        1 => GridTrackUnit::Px(r.f32()?),
        2 => GridTrackUnit::Percent(r.f32()?),
        3 => GridTrackUnit::Fr(r.f32()?),
        4 => GridTrackUnit::MinContent,
        5 => GridTrackUnit::MaxContent,
        _ => return None,
    })
}

fn write_single_track(out: &mut Vec<u8>, t: GridSingleTrack) {
    match t {
        GridSingleTrack::Auto => out.push(0),
        GridSingleTrack::Px(v) => {
            out.push(1);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridSingleTrack::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridSingleTrack::Fr(v) => {
            out.push(3);
            out.extend_from_slice(&canon_f32_bytes(v));
        }
        GridSingleTrack::MinMax(min, max) => {
            out.push(4);
            write_track_unit(out, min);
            write_track_unit(out, max);
        }
    }
}
fn read_single_track(r: &mut Reader<'_>) -> Option<GridSingleTrack> {
    Some(match r.u8()? {
        0 => GridSingleTrack::Auto,
        1 => GridSingleTrack::Px(r.f32()?),
        2 => GridSingleTrack::Percent(r.f32()?),
        3 => GridSingleTrack::Fr(r.f32()?),
        4 => GridSingleTrack::MinMax(read_track_unit(r)?, read_track_unit(r)?),
        _ => return None,
    })
}

fn write_grid_tracks(out: &mut Vec<u8>, tracks: &[GridTrack]) {
    out.extend_from_slice(&(tracks.len() as u32).to_le_bytes());
    for t in tracks {
        match *t {
            GridTrack::Auto => out.push(0),
            GridTrack::Px(v) => {
                out.push(1);
                out.extend_from_slice(&canon_f32_bytes(v));
            }
            GridTrack::Percent(v) => {
                out.push(2);
                out.extend_from_slice(&canon_f32_bytes(v));
            }
            GridTrack::Fr(v) => {
                out.push(3);
                out.extend_from_slice(&canon_f32_bytes(v));
            }
            GridTrack::MinMax(min, max) => {
                out.push(4);
                write_track_unit(out, min);
                write_track_unit(out, max);
            }
            GridTrack::Repeat(count, track) => {
                out.push(5);
                out.extend_from_slice(&count.to_le_bytes());
                write_single_track(out, track);
            }
        }
    }
}
fn read_grid_tracks(r: &mut Reader<'_>) -> Option<Vec<GridTrack>> {
    let n = r.count()?;
    let mut out = Vec::with_capacity(n.min(32));
    for _ in 0..n {
        let t = match r.u8()? {
            0 => GridTrack::Auto,
            1 => GridTrack::Px(r.f32()?),
            2 => GridTrack::Percent(r.f32()?),
            3 => GridTrack::Fr(r.f32()?),
            4 => GridTrack::MinMax(read_track_unit(r)?, read_track_unit(r)?),
            5 => GridTrack::Repeat(r.u16()?, read_single_track(r)?),
            _ => return None,
        };
        out.push(t);
    }
    Some(out)
}

fn write_line(out: &mut Vec<u8>, l: GridLine) {
    match l {
        GridLine::Auto => out.push(0),
        GridLine::Line(n) => {
            out.push(1);
            out.extend_from_slice(&n.to_le_bytes());
        }
        GridLine::Span(n) => {
            out.push(2);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}
fn read_line(r: &mut Reader<'_>) -> Option<GridLine> {
    match r.u8()? {
        0 => Some(GridLine::Auto),
        1 => Some(GridLine::Line(r.i16()?)),
        2 => Some(GridLine::Span(r.u16()?)),
        _ => None,
    }
}

fn write_placement(out: &mut Vec<u8>, p: GridTrackPlacement) {
    write_line(out, p.start);
    write_line(out, p.end);
}
fn read_placement(r: &mut Reader<'_>) -> Option<GridTrackPlacement> {
    Some(GridTrackPlacement {
        start: read_line(r)?,
        end: read_line(r)?,
    })
}

fn write_transform(out: &mut Vec<u8>, t: Transform2D) {
    for v in [
        t.translate_x,
        t.translate_y,
        t.scale_x,
        t.scale_y,
        t.rotate_deg,
        t.origin_x,
        t.origin_y,
    ] {
        out.extend_from_slice(&canon_f32_bytes(v));
    }
}
fn read_transform(r: &mut Reader<'_>) -> Option<Transform2D> {
    Some(Transform2D {
        translate_x: r.f32()?,
        translate_y: r.f32()?,
        scale_x: r.f32()?,
        scale_y: r.f32()?,
        rotate_deg: r.f32()?,
        origin_x: r.f32()?,
        origin_y: r.f32()?,
    })
}

fn write_bg_fill(out: &mut Vec<u8>, fill: &BackgroundFill) {
    match fill {
        BackgroundFill::Solid(c) => {
            out.push(0);
            write_color(out, *c);
        }
        BackgroundFill::Linear(lg) => {
            out.push(1);
            out.extend_from_slice(&canon_f32_bytes(lg.angle_deg));
            out.extend_from_slice(&(lg.stops.len() as u32).to_le_bytes());
            for s in lg.stops.iter() {
                out.extend_from_slice(&canon_f32_bytes(s.offset));
                write_color(out, s.color);
            }
        }
        BackgroundFill::Radial(rg) => {
            out.push(2);
            out.extend_from_slice(&canon_f32_bytes(rg.center_x));
            out.extend_from_slice(&canon_f32_bytes(rg.center_y));
            out.extend_from_slice(&canon_f32_bytes(rg.radius));
            out.extend_from_slice(&(rg.stops.len() as u32).to_le_bytes());
            for s in rg.stops.iter() {
                out.extend_from_slice(&canon_f32_bytes(s.offset));
                write_color(out, s.color);
            }
        }
    }
}
fn read_bg_fill(r: &mut Reader<'_>) -> Option<BackgroundFill> {
    match r.u8()? {
        0 => Some(BackgroundFill::Solid(read_color(r)?)),
        1 => {
            let angle = r.f32()?;
            let n = r.count()?;
            let mut stops = Vec::with_capacity(n.min(32));
            for _ in 0..n {
                stops.push(GradientStop::new(r.f32()?, read_color(r)?));
            }
            Some(BackgroundFill::Linear(LinearGradient::new(angle, stops)))
        }
        2 => {
            let cx = r.f32()?;
            let cy = r.f32()?;
            let rad = r.f32()?;
            let n = r.count()?;
            let mut stops = Vec::with_capacity(n.min(32));
            for _ in 0..n {
                stops.push(GradientStop::new(r.f32()?, read_color(r)?));
            }
            Some(BackgroundFill::Radial(RadialGradient::new(cx, cy, rad, stops)))
        }
        _ => None,
    }
}

pub(crate) fn encode_style_patch(p: &StylePatch, out: &mut Vec<u8>) {
    if let Some(v) = p.flex_direction { out.extend_from_slice(&[1, v as u8]); }
    if let Some(v) = p.justify_content { out.extend_from_slice(&[2, v as u8]); }
    if let Some(v) = p.align_items { out.extend_from_slice(&[3, v as u8]); }
    if let Some(v) = p.flex_grow { out.push(4); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.flex_shrink { out.push(5); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.flex_basis { out.push(6); write_dim(out, v); }
    if let Some(v) = p.aspect_ratio { out.push(7); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.position { out.extend_from_slice(&[8, v as u8]); }
    if let Some(v) = p.inset_top { out.push(9); write_dim(out, v); }
    if let Some(v) = p.inset_right { out.push(10); write_dim(out, v); }
    if let Some(v) = p.inset_bottom { out.push(11); write_dim(out, v); }
    if let Some(v) = p.inset_left { out.push(12); write_dim(out, v); }
    if let Some(v) = p.gap { out.push(13); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.column_gap { out.push(14); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.row_gap { out.push(15); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(ref v) = p.grid_template_columns { out.push(16); write_grid_tracks(out, v); }
    if let Some(ref v) = p.grid_template_rows { out.push(17); write_grid_tracks(out, v); }
    if let Some(v) = p.grid_column { out.push(18); write_placement(out, v); }
    if let Some(v) = p.grid_row { out.push(19); write_placement(out, v); }
    if let Some(v) = p.direction { out.extend_from_slice(&[20, v as u8]); }
    if let Some(v) = p.padding_inline_start { out.push(21); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.padding_inline_end { out.push(22); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.margin_inline_start { out.push(23); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.margin_inline_end { out.push(24); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.transform { out.push(25); write_transform(out, v); }
    if let Some(v) = p.width { out.push(26); write_dim(out, v); }
    if let Some(v) = p.height { out.push(27); write_dim(out, v); }
    if let Some(v) = p.min_width { out.push(28); write_dim(out, v); }
    if let Some(v) = p.max_width { out.push(29); write_dim(out, v); }
    if let Some(v) = p.min_height { out.push(30); write_dim(out, v); }
    if let Some(v) = p.max_height { out.push(31); write_dim(out, v); }
    if let Some(v) = p.padding { out.push(32); write_edges(out, v); }
    if let Some(v) = p.margin { out.push(33); write_edges(out, v); }
    if let Some(v) = p.background { out.push(34); write_color(out, v); }
    if let Some(ref v) = p.background_fill { out.push(35); write_bg_fill(out, v); }
    if let Some(v) = p.text_color { out.push(36); write_color(out, v); }
    if let Some(v) = p.border_width { out.push(37); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.border_color { out.push(38); write_color(out, v); }
    if let Some(v) = p.border_radius { out.push(39); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.opacity { out.push(40); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.font_size { out.push(41); out.extend_from_slice(&canon_f32_bytes(v)); }
    if let Some(v) = p.z_index { out.push(42); out.extend_from_slice(&v.to_le_bytes()); }
    if let Some(v) = p.flex_wrap { out.extend_from_slice(&[43, v as u8]); }
    if let Some(v) = p.overflow { out.extend_from_slice(&[44, v as u8]); }
    if let Some(v) = p.text_align { out.extend_from_slice(&[45, v as u8]); }
    if let Some(v) = p.font_weight { out.push(46); out.extend_from_slice(&v.to_le_bytes()); }
    if let Some(v) = p.display { out.extend_from_slice(&[47, v as u8]); }
    if let Some(v) = p.cursor { out.extend_from_slice(&[48, v as u8]); }
    out.push(TAG_END);
}

pub(crate) fn decode_style_patch_bytes(bytes: &[u8]) -> Option<StylePatch> {
    let mut r = Reader(bytes);
    let mut p = StylePatch::new();
    loop {
        match r.u8()? {
            TAG_END => return r.0.is_empty().then_some(p),
            1 => p.flex_direction = Some(match r.u8()? {
                0 => FlexDirection::Column, 1 => FlexDirection::Row,
                2 => FlexDirection::ColumnReverse, 3 => FlexDirection::RowReverse, _ => return None,
            }),
            2 => p.justify_content = Some(match r.u8()? {
                0 => FlexJustify::FlexStart, 1 => FlexJustify::Center, 2 => FlexJustify::FlexEnd,
                3 => FlexJustify::SpaceBetween, 4 => FlexJustify::SpaceAround, 5 => FlexJustify::SpaceEvenly,
                _ => return None,
            }),
            3 => p.align_items = Some(match r.u8()? {
                0 => FlexAlign::Stretch, 1 => FlexAlign::FlexStart, 2 => FlexAlign::Center,
                3 => FlexAlign::FlexEnd, 4 => FlexAlign::Baseline, _ => return None,
            }),
            4 => p.flex_grow = Some(r.f32()?),
            5 => p.flex_shrink = Some(r.f32()?),
            6 => p.flex_basis = Some(read_dim(&mut r)?),
            7 => p.aspect_ratio = Some(r.f32()?),
            8 => p.position = Some(match r.u8()? {
                0 => PositionMode::Relative, 1 => PositionMode::Absolute, 2 => PositionMode::Sticky,
                _ => return None,
            }),
            9 => p.inset_top = Some(read_dim(&mut r)?),
            10 => p.inset_right = Some(read_dim(&mut r)?),
            11 => p.inset_bottom = Some(read_dim(&mut r)?),
            12 => p.inset_left = Some(read_dim(&mut r)?),
            13 => p.gap = Some(r.f32()?),
            14 => p.column_gap = Some(r.f32()?),
            15 => p.row_gap = Some(r.f32()?),
            16 => p.grid_template_columns = Some(read_grid_tracks(&mut r)?.into_boxed_slice()),
            17 => p.grid_template_rows = Some(read_grid_tracks(&mut r)?.into_boxed_slice()),
            18 => p.grid_column = Some(read_placement(&mut r)?),
            19 => p.grid_row = Some(read_placement(&mut r)?),
            20 => p.direction = Some(match r.u8()? {
                0 => LayoutDirection::Ltr, 1 => LayoutDirection::Rtl, _ => return None,
            }),
            21 => p.padding_inline_start = Some(r.f32()?),
            22 => p.padding_inline_end = Some(r.f32()?),
            23 => p.margin_inline_start = Some(r.f32()?),
            24 => p.margin_inline_end = Some(r.f32()?),
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
            37 => p.border_width = Some(r.f32()?),
            38 => p.border_color = Some(read_color(&mut r)?),
            39 => p.border_radius = Some(r.f32()?),
            40 => p.opacity = Some(r.f32()?),
            41 => p.font_size = Some(r.f32()?),
            42 => p.z_index = Some(r.i32()?),
            43 => p.flex_wrap = Some(match r.u8()? {
                0 => FlexWrap::NoWrap, 1 => FlexWrap::Wrap, 2 => FlexWrap::WrapReverse, _ => return None,
            }),
            44 => p.overflow = Some(match r.u8()? {
                0 => Overflow::Visible, 1 => Overflow::Hidden, 2 => Overflow::Scroll, 3 => Overflow::Auto, _ => return None,
            }),
            45 => p.text_align = Some(match r.u8()? {
                0 => TextAlign::Left, 1 => TextAlign::Center, 2 => TextAlign::Right, 3 => TextAlign::Justify, _ => return None,
            }),
            46 => p.font_weight = Some(r.u16()?),
            47 => p.display = Some(match r.u8()? {
                0 => Display::Flex, 1 => Display::None, 2 => Display::Grid, _ => return None,
            }),
            48 => p.cursor = Some(CursorKind::from_u8(r.u8()?)?),
            _ => return None,
        }
    }
}
