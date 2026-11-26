//! Compact, zero-dependency binary wire codecs for extended typed node properties.
//!
//! Used **exclusively** by cross-boundary transport (`BatchSink` -> `CommandBuffer` ->
//! `replay_command_buffer_into`, `dlopen` `cdylib` hot-reload, and `nve-wire` streaming).
//! In-process Rust hosts pass typed references directly through `RenderSink` with zero encoding.

mod paint_svg;
mod rules;
mod style_patch;

use nve_abi::{ColorRgba, Dimension, EdgeInsets};

use crate::layout_ext::{
    GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack, GridTrackPlacement,
    GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets, PositionMode,
};
use crate::style_patch::Display;
use crate::transform::Transform2D;

pub use paint_svg::{
    decode_background_fill, decode_svg_picture, encode_background_fill, encode_svg_picture,
};
pub use rules::{
    decode_cursor, decode_media_query, decode_responsive_rules, decode_state_rules,
    decode_transitions, encode_cursor, encode_media_query, encode_responsive_rules,
    encode_state_rules, encode_transitions,
};
pub use style_patch::encode_style_patch;

pub(crate) use paint_svg::{read_background_fill, write_background_fill};

/// Reserved `SetRawProp` key prefix identifying binary extended-property payloads emitted by
/// `BatchSink` for cross-boundary transport.
pub const EXT_PROP_PREFIX: &str = "nve.ext.";
pub const EXT_PROP_LAYOUT_EXT: &str = "nve.ext.layout_ext";
pub const EXT_PROP_TRANSFORM: &str = "nve.ext.transform";
pub const EXT_PROP_CURSOR: &str = "nve.ext.cursor";
pub const EXT_PROP_STATE_RULES: &str = "nve.ext.state_rules";
pub const EXT_PROP_TRANSITIONS: &str = "nve.ext.transitions";
pub const EXT_PROP_SHOW_ON: &str = "nve.ext.show_on";
pub const EXT_PROP_RESPONSIVE_RULES: &str = "nve.ext.responsive_rules";
pub const EXT_PROP_BACKGROUND_FILL: &str = "nve.ext.background_fill";
pub const EXT_PROP_SVG: &str = "nve.ext.svg";

/// Hard upper bound on decoded collection counts to reject corrupt/malicious wire payloads.
const MAX_WIRE_ITEMS: usize = 65_536;

pub(crate) struct WireReader<'a> {
    bytes: &'a [u8],
}

impl<'a> WireReader<'a> {
    #[inline]
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[inline]
    pub(crate) fn read_array<const N: usize>(&mut self) -> Option<&'a [u8; N]> {
        if self.bytes.len() < N {
            return None;
        }
        let (head, rest) = self.bytes.split_at(N);
        self.bytes = rest;
        head.try_into().ok()
    }

    #[inline]
    pub(crate) fn read_u8(&mut self) -> Option<u8> {
        Some(self.read_array::<1>()?[0])
    }

    #[inline]
    pub(crate) fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(*self.read_array::<2>()?))
    }

    #[inline]
    pub(crate) fn read_i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(*self.read_array::<2>()?))
    }

    #[inline]
    pub(crate) fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(*self.read_array::<4>()?))
    }

    #[inline]
    pub(crate) fn read_i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(*self.read_array::<4>()?))
    }

    #[inline]
    pub(crate) fn read_f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(*self.read_array::<4>()?))
    }

    #[inline]
    pub(crate) fn read_bool(&mut self) -> Option<bool> {
        match self.read_u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn read_count(&mut self) -> Option<usize> {
        let n = self.read_u32()? as usize;
        (n <= MAX_WIRE_ITEMS).then_some(n)
    }
}

#[inline]
pub(crate) fn write_color(out: &mut Vec<u8>, c: ColorRgba) {
    out.extend_from_slice(&[c.r, c.g, c.b, c.a]);
}

#[inline]
pub(crate) fn read_color(r: &mut WireReader<'_>) -> Option<ColorRgba> {
    let [red, g, b, a] = *r.read_array::<4>()?;
    Some(ColorRgba { r: red, g, b, a })
}

#[inline]
pub(crate) fn write_dim(out: &mut Vec<u8>, d: Dimension) {
    match d {
        Dimension::Auto => out.push(0),
        Dimension::Points(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Dimension::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

#[inline]
pub(crate) fn read_dim(r: &mut WireReader<'_>) -> Option<Dimension> {
    match r.read_u8()? {
        0 => Some(Dimension::Auto),
        1 => Some(Dimension::Points(r.read_f32()?)),
        2 => Some(Dimension::Percent(r.read_f32()?)),
        _ => None,
    }
}

#[inline]
pub(crate) fn write_edges(out: &mut Vec<u8>, e: EdgeInsets) {
    out.extend_from_slice(&e.top.to_le_bytes());
    out.extend_from_slice(&e.right.to_le_bytes());
    out.extend_from_slice(&e.bottom.to_le_bytes());
    out.extend_from_slice(&e.left.to_le_bytes());
}

#[inline]
pub(crate) fn read_edges(r: &mut WireReader<'_>) -> Option<EdgeInsets> {
    Some(EdgeInsets {
        top: r.read_f32()?,
        right: r.read_f32()?,
        bottom: r.read_f32()?,
        left: r.read_f32()?,
    })
}

#[inline]
pub(crate) fn write_opt_f32(out: &mut Vec<u8>, v: Option<f32>) {
    match v {
        None => out.push(0),
        Some(x) => {
            out.push(1);
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
}

#[inline]
pub(crate) fn read_opt_f32(r: &mut WireReader<'_>) -> Option<Option<f32>> {
    match r.read_u8()? {
        0 => Some(None),
        1 => Some(Some(r.read_f32()?)),
        _ => None,
    }
}

#[inline]
pub(crate) fn write_track_unit(out: &mut Vec<u8>, u: GridTrackUnit) {
    match u {
        GridTrackUnit::Auto => out.push(0),
        GridTrackUnit::Px(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrackUnit::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrackUnit::Fr(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrackUnit::MinContent => out.push(4),
        GridTrackUnit::MaxContent => out.push(5),
    }
}

#[inline]
pub(crate) fn read_track_unit(r: &mut WireReader<'_>) -> Option<GridTrackUnit> {
    match r.read_u8()? {
        0 => Some(GridTrackUnit::Auto),
        1 => Some(GridTrackUnit::Px(r.read_f32()?)),
        2 => Some(GridTrackUnit::Percent(r.read_f32()?)),
        3 => Some(GridTrackUnit::Fr(r.read_f32()?)),
        4 => Some(GridTrackUnit::MinContent),
        5 => Some(GridTrackUnit::MaxContent),
        _ => None,
    }
}

#[inline]
pub(crate) fn write_single_track(out: &mut Vec<u8>, t: GridSingleTrack) {
    match t {
        GridSingleTrack::Auto => out.push(0),
        GridSingleTrack::Px(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridSingleTrack::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridSingleTrack::Fr(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridSingleTrack::MinMax(a, b) => {
            out.push(4);
            write_track_unit(out, a);
            write_track_unit(out, b);
        }
    }
}

#[inline]
pub(crate) fn read_single_track(r: &mut WireReader<'_>) -> Option<GridSingleTrack> {
    match r.read_u8()? {
        0 => Some(GridSingleTrack::Auto),
        1 => Some(GridSingleTrack::Px(r.read_f32()?)),
        2 => Some(GridSingleTrack::Percent(r.read_f32()?)),
        3 => Some(GridSingleTrack::Fr(r.read_f32()?)),
        4 => Some(GridSingleTrack::MinMax(
            read_track_unit(r)?,
            read_track_unit(r)?,
        )),
        _ => None,
    }
}

pub(crate) fn write_grid_track(out: &mut Vec<u8>, t: GridTrack) {
    match t {
        GridTrack::Auto => out.push(0),
        GridTrack::Px(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrack::Percent(v) => {
            out.push(2);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrack::Fr(v) => {
            out.push(3);
            out.extend_from_slice(&v.to_le_bytes());
        }
        GridTrack::MinMax(a, b) => {
            out.push(4);
            write_track_unit(out, a);
            write_track_unit(out, b);
        }
        GridTrack::Repeat(n, single) => {
            out.push(5);
            out.extend_from_slice(&n.to_le_bytes());
            write_single_track(out, single);
        }
    }
}

pub(crate) fn read_grid_track(r: &mut WireReader<'_>) -> Option<GridTrack> {
    match r.read_u8()? {
        0 => Some(GridTrack::Auto),
        1 => Some(GridTrack::Px(r.read_f32()?)),
        2 => Some(GridTrack::Percent(r.read_f32()?)),
        3 => Some(GridTrack::Fr(r.read_f32()?)),
        4 => Some(GridTrack::MinMax(read_track_unit(r)?, read_track_unit(r)?)),
        5 => Some(GridTrack::Repeat(r.read_u16()?, read_single_track(r)?)),
        _ => None,
    }
}

pub(crate) fn write_grid_tracks(out: &mut Vec<u8>, tracks: &[GridTrack]) {
    out.extend_from_slice(&(tracks.len() as u32).to_le_bytes());
    for &t in tracks {
        write_grid_track(out, t);
    }
}

pub(crate) fn read_grid_tracks(r: &mut WireReader<'_>) -> Option<Vec<GridTrack>> {
    let n = r.read_count()?;
    let mut out = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        out.push(read_grid_track(r)?);
    }
    Some(out)
}

#[inline]
pub(crate) fn write_grid_line(out: &mut Vec<u8>, line: GridLine) {
    match line {
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

#[inline]
pub(crate) fn read_grid_line(r: &mut WireReader<'_>) -> Option<GridLine> {
    match r.read_u8()? {
        0 => Some(GridLine::Auto),
        1 => Some(GridLine::Line(r.read_i16()?)),
        2 => Some(GridLine::Span(r.read_u16()?)),
        _ => None,
    }
}

#[inline]
pub(crate) fn write_placement(out: &mut Vec<u8>, p: GridTrackPlacement) {
    write_grid_line(out, p.start);
    write_grid_line(out, p.end);
}

#[inline]
pub(crate) fn read_placement(r: &mut WireReader<'_>) -> Option<GridTrackPlacement> {
    Some(GridTrackPlacement {
        start: read_grid_line(r)?,
        end: read_grid_line(r)?,
    })
}

pub(crate) fn decode_display(raw: u8) -> Option<Display> {
    match raw {
        0 => Some(Display::Flex),
        1 => Some(Display::None),
        2 => Some(Display::Grid),
        _ => None,
    }
}

pub(crate) fn decode_position_mode(raw: u8) -> Option<PositionMode> {
    match raw {
        0 => Some(PositionMode::Relative),
        1 => Some(PositionMode::Absolute),
        2 => Some(PositionMode::Sticky),
        _ => None,
    }
}

pub(crate) fn decode_layout_direction(raw: u8) -> Option<LayoutDirection> {
    match raw {
        0 => Some(LayoutDirection::Ltr),
        1 => Some(LayoutDirection::Rtl),
        _ => None,
    }
}

/// Encodes [`LayoutExt`] into `out`.
pub fn encode_layout_ext(ext: &LayoutExt, out: &mut Vec<u8>) {
    out.push(ext.display as u8);
    write_dim(out, ext.flex_basis);
    write_opt_f32(out, ext.aspect_ratio);
    out.push(ext.position as u8);
    write_dim(out, ext.insets.top);
    write_dim(out, ext.insets.right);
    write_dim(out, ext.insets.bottom);
    write_dim(out, ext.insets.left);
    match ext.direction {
        None => out.push(0),
        Some(dir) => {
            out.push(1);
            out.push(dir as u8);
        }
    }
    write_opt_f32(out, ext.padding_inline.start);
    write_opt_f32(out, ext.padding_inline.end);
    write_opt_f32(out, ext.margin_inline.start);
    write_opt_f32(out, ext.margin_inline.end);
    match ext.grid.as_deref() {
        None => out.push(0),
        Some(grid) => {
            out.push(1);
            write_grid_tracks(out, &grid.template_columns);
            write_grid_tracks(out, &grid.template_rows);
            write_opt_f32(out, grid.column_gap);
            write_opt_f32(out, grid.row_gap);
        }
    }
    write_placement(out, ext.grid_item.column);
    write_placement(out, ext.grid_item.row);
}

/// Decodes [`LayoutExt`] from binary wire bytes.
pub fn decode_layout_ext(bytes: &[u8]) -> Option<LayoutExt> {
    let mut r = WireReader::new(bytes);
    let display = decode_display(r.read_u8()?)?;
    let flex_basis = read_dim(&mut r)?;
    let aspect_ratio = read_opt_f32(&mut r)?;
    let position = decode_position_mode(r.read_u8()?)?;
    let insets = PositionInsets {
        top: read_dim(&mut r)?,
        right: read_dim(&mut r)?,
        bottom: read_dim(&mut r)?,
        left: read_dim(&mut r)?,
    };
    let direction = match r.read_u8()? {
        0 => None,
        1 => Some(decode_layout_direction(r.read_u8()?)?),
        _ => return None,
    };
    let padding_inline = LogicalInlineEdges {
        start: read_opt_f32(&mut r)?,
        end: read_opt_f32(&mut r)?,
    };
    let margin_inline = LogicalInlineEdges {
        start: read_opt_f32(&mut r)?,
        end: read_opt_f32(&mut r)?,
    };
    let grid = match r.read_u8()? {
        0 => None,
        1 => Some(Box::new(GridLayout {
            template_columns: read_grid_tracks(&mut r)?,
            template_rows: read_grid_tracks(&mut r)?,
            column_gap: read_opt_f32(&mut r)?,
            row_gap: read_opt_f32(&mut r)?,
        })),
        _ => return None,
    };
    let grid_item = GridItemPlacement {
        column: read_placement(&mut r)?,
        row: read_placement(&mut r)?,
    };
    r.is_empty().then_some(LayoutExt {
        display,
        flex_basis,
        aspect_ratio,
        position,
        insets,
        direction,
        padding_inline,
        margin_inline,
        grid,
        grid_item,
    })
}

/// Encodes [`Transform2D`] into 28 LE bytes.
#[inline]
pub fn encode_transform(tf: Transform2D, out: &mut Vec<u8>) {
    out.extend_from_slice(&tf.translate_x.to_le_bytes());
    out.extend_from_slice(&tf.translate_y.to_le_bytes());
    out.extend_from_slice(&tf.scale_x.to_le_bytes());
    out.extend_from_slice(&tf.scale_y.to_le_bytes());
    out.extend_from_slice(&tf.rotate_deg.to_le_bytes());
    out.extend_from_slice(&tf.origin_x.to_le_bytes());
    out.extend_from_slice(&tf.origin_y.to_le_bytes());
}

/// Decodes [`Transform2D`] from 28 LE bytes.
#[inline]
pub fn decode_transform(bytes: &[u8]) -> Option<Transform2D> {
    let mut r = WireReader::new(bytes);
    let tf = read_transform(&mut r)?;
    r.is_empty().then_some(tf)
}

#[inline]
pub(crate) fn write_transform(out: &mut Vec<u8>, tf: Transform2D) {
    encode_transform(tf, out);
}

#[inline]
pub(crate) fn read_transform(r: &mut WireReader<'_>) -> Option<Transform2D> {
    Some(Transform2D {
        translate_x: r.read_f32()?,
        translate_y: r.read_f32()?,
        scale_x: r.read_f32()?,
        scale_y: r.read_f32()?,
        rotate_deg: r.read_f32()?,
        origin_x: r.read_f32()?,
        origin_y: r.read_f32()?,
    })
}
