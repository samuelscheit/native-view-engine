//! Deduplicated `nve_store::Resource` blobs referenced by core word tables.
//!
//! In-process backends read the typed value directly (zero encode/decode); binary codecs run only
//! for XXH64 deduplication and cross-boundary `NVE2` frames.

use std::sync::Arc;

use nve_abi::Key64;
use nve_codec::{ByteReader, ByteWriter, CodecError};
use nve_store::Resource;

use crate::interaction::{InteractionState, StateStyleRule};
use crate::layout_ext::{GridSingleTrack, GridTrack, GridTrackUnit};
use crate::media::{MediaQuery, ResponsiveRule};
use crate::motion::{EasingCurve, TransitionProperty, TransitionSpec};
use crate::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
use crate::style_patch::StylePatch;

use super::box_shadow::BoxShadow;

const MAX_ITEMS: usize = 4096;
const MAX_STRING_BYTES: usize = 256 * 1024;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[inline]
fn encode_into_or_rollback(
    out: &mut Vec<u8>,
    f: impl FnOnce(&mut ByteWriter) -> Result<(), CodecError>,
) {
    let start = out.len();
    if ByteWriter::with_buf(out, f).is_err() {
        out.truncate(start);
    }
}

fn write_grid_unit(w: &mut ByteWriter, u: GridTrackUnit) {
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

fn read_grid_unit(r: &mut ByteReader<'_>) -> Result<GridTrackUnit, CodecError> {
    Ok(match r.read_u8()? {
        0 => GridTrackUnit::Auto,
        1 => GridTrackUnit::Px(r.read_f32()?),
        2 => GridTrackUnit::Percent(r.read_f32()?),
        3 => GridTrackUnit::Fr(r.read_f32()?),
        4 => GridTrackUnit::MinContent,
        5 => GridTrackUnit::MaxContent,
        raw => return Err(CodecError::BadEnum { ty: "GridTrackUnit", raw: i64::from(raw) }),
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
            write_grid_unit(w, min);
            write_grid_unit(w, max);
        }
    }
}

fn read_single_track(r: &mut ByteReader<'_>) -> Result<GridSingleTrack, CodecError> {
    Ok(match r.read_u8()? {
        0 => GridSingleTrack::Auto,
        1 => GridSingleTrack::Px(r.read_f32()?),
        2 => GridSingleTrack::Percent(r.read_f32()?),
        3 => GridSingleTrack::Fr(r.read_f32()?),
        4 => GridSingleTrack::MinMax(read_grid_unit(r)?, read_grid_unit(r)?),
        raw => return Err(CodecError::BadEnum { ty: "GridSingleTrack", raw: i64::from(raw) }),
    })
}

// ---------------------------------------------------------------------------------------------
// GridTracks
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct GridTracks(pub Arc<[GridTrack]>);

impl GridTracks {
    pub fn new(tracks: impl Into<Arc<[GridTrack]>>) -> Self {
        Self(tracks.into())
    }
}

impl Resource for GridTracks {
    const KIND: Key64 = Key64::of("nve.res.grid_tracks");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_len_max(self.0.len(), MAX_ITEMS)?;
            for &track in self.0.iter() {
                match track {
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
                        write_grid_unit(w, min);
                        write_grid_unit(w, max);
                    }
                    GridTrack::Repeat(count, single) => {
                        w.write_u8(5);
                        w.write_u16(count);
                        write_single_track(w, single);
                    }
                }
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ITEMS, 1, |r| match r.read_u8()? {
                0 => Ok(GridTrack::Auto),
                1 => Ok(GridTrack::Px(r.read_f32()?)),
                2 => Ok(GridTrack::Percent(r.read_f32()?)),
                3 => Ok(GridTrack::Fr(r.read_f32()?)),
                4 => Ok(GridTrack::MinMax(read_grid_unit(r)?, read_grid_unit(r)?)),
                5 => Ok(GridTrack::Repeat(r.read_u16()?, read_single_track(r)?)),
                raw => Err(CodecError::BadEnum { ty: "GridTrack", raw: i64::from(raw) }),
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
    }
}

// ---------------------------------------------------------------------------------------------
// BoxShadows
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct BoxShadows(pub Arc<[BoxShadow]>);

impl BoxShadows {
    pub fn new(shadows: impl Into<Arc<[BoxShadow]>>) -> Self {
        Self(shadows.into())
    }
}

impl Resource for BoxShadows {
    const KIND: Key64 = Key64::of("nve.res.box_shadows");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_len_max(self.0.len(), MAX_ITEMS)?;
            for s in self.0.iter() {
                w.write_f32(s.offset_x);
                w.write_f32(s.offset_y);
                w.write_f32(s.blur_radius);
                w.write_f32(s.spread_radius);
                w.write_u32(s.color.0);
                w.write_bool(s.inset);
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ITEMS, 21, |r| {
                Ok(BoxShadow {
                    offset_x: r.read_f32()?,
                    offset_y: r.read_f32()?,
                    blur_radius: r.read_f32()?,
                    spread_radius: r.read_f32()?,
                    color: nve_store::Rgba(r.read_u32()?),
                    inset: r.read_bool()?,
                })
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
    }
}

// ---------------------------------------------------------------------------------------------
// BackgroundFillRes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundFillRes(pub BackgroundFill);

impl Resource for BackgroundFillRes {
    const KIND: Key64 = Key64::of("nve.res.background_fill");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            match &self.0 {
                BackgroundFill::Solid(c) => {
                    w.write_u8(0);
                    w.write_u32(c.to_u32());
                }
                BackgroundFill::Linear(lg) => {
                    w.write_u8(1);
                    w.write_f32(lg.angle_deg);
                    w.write_len_max(lg.stops.len(), MAX_ITEMS)?;
                    for s in lg.stops.iter() {
                        w.write_f32(s.offset);
                        w.write_u32(s.color.to_u32());
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
                        w.write_u32(s.color.to_u32());
                    }
                }
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let fill = match r.read_u8().ok()? {
            0 => BackgroundFill::Solid(nve_abi::ColorRgba::from_u32(r.read_u32().ok()?)),
            1 => {
                let angle = r.read_f32().ok()?;
                let stops = r
                    .read_vec(MAX_ITEMS, 8, |r| {
                        Ok(GradientStop::new(
                            r.read_f32()?,
                            nve_abi::ColorRgba::from_u32(r.read_u32()?),
                        ))
                    })
                    .ok()?;
                BackgroundFill::Linear(LinearGradient::new(angle, stops))
            }
            2 => {
                let cx = r.read_f32().ok()?;
                let cy = r.read_f32().ok()?;
                let rad = r.read_f32().ok()?;
                let stops = r
                    .read_vec(MAX_ITEMS, 8, |r| {
                        Ok(GradientStop::new(
                            r.read_f32()?,
                            nve_abi::ColorRgba::from_u32(r.read_u32()?),
                        ))
                    })
                    .ok()?;
                BackgroundFill::Radial(RadialGradient::new(cx, cy, rad, stops))
            }
            _ => return None,
        };
        r.finish().ok()?;
        Some(Self(fill))
    }
}

// ---------------------------------------------------------------------------------------------
// Transitions
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Transitions(pub Arc<[TransitionSpec]>);

impl Transitions {
    pub fn new(specs: impl Into<Arc<[TransitionSpec]>>) -> Self {
        Self(specs.into())
    }
}

impl Resource for Transitions {
    const KIND: Key64 = Key64::of("nve.res.transitions");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_len_max(self.0.len(), MAX_ITEMS)?;
            for t in self.0.iter() {
                w.write_u8(t.property as u8);
                w.write_u32(t.duration_ms);
                w.write_u32(t.delay_ms);
                match t.easing {
                    EasingCurve::Linear => w.write_u8(0),
                    EasingCurve::Ease => w.write_u8(1),
                    EasingCurve::EaseIn => w.write_u8(2),
                    EasingCurve::EaseOut => w.write_u8(3),
                    EasingCurve::EaseInOut => w.write_u8(4),
                    EasingCurve::CubicBezier(x1, y1, x2, y2) => {
                        w.write_u8(5);
                        w.write_f32(x1);
                        w.write_f32(y1);
                        w.write_f32(x2);
                        w.write_f32(y2);
                    }
                }
                w.write_bool(t.reduced_motion_instant);
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ITEMS, 11, |r| {
                let prop = match r.read_u8()? {
                    0 => TransitionProperty::All,
                    1 => TransitionProperty::BackgroundColor,
                    2 => TransitionProperty::Color,
                    3 => TransitionProperty::BorderColor,
                    4 => TransitionProperty::Opacity,
                    5 => TransitionProperty::BorderRadius,
                    6 => TransitionProperty::Transform,
                    7 => TransitionProperty::Width,
                    8 => TransitionProperty::Height,
                    9 => TransitionProperty::Padding,
                    10 => TransitionProperty::Gap,
                    raw => {
                        return Err(CodecError::BadEnum {
                            ty: "TransitionProperty",
                            raw: i64::from(raw),
                        });
                    }
                };
                let duration_ms = r.read_u32()?;
                let delay_ms = r.read_u32()?;
                let easing = match r.read_u8()? {
                    0 => EasingCurve::Linear,
                    1 => EasingCurve::Ease,
                    2 => EasingCurve::EaseIn,
                    3 => EasingCurve::EaseOut,
                    4 => EasingCurve::EaseInOut,
                    5 => EasingCurve::CubicBezier(
                        r.read_f32()?,
                        r.read_f32()?,
                        r.read_f32()?,
                        r.read_f32()?,
                    ),
                    raw => {
                        return Err(CodecError::BadEnum {
                            ty: "EasingCurve",
                            raw: i64::from(raw),
                        });
                    }
                };
                let reduced_motion_instant = r.read_bool()?;
                Ok(TransitionSpec {
                    property: prop,
                    duration_ms,
                    delay_ms,
                    easing,
                    reduced_motion_instant,
                })
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
    }
}

// ---------------------------------------------------------------------------------------------
// ResponsiveRules & StateRules
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveRules(pub Arc<[ResponsiveRule]>);

impl ResponsiveRules {
    pub fn new(rules: impl Into<Arc<[ResponsiveRule]>>) -> Self {
        Self(rules.into())
    }
}

impl Resource for ResponsiveRules {
    const KIND: Key64 = Key64::of("nve.res.responsive_rules");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_len_max(self.0.len(), MAX_ITEMS)?;
            let mut patch_w = ByteWriter::new();
            for rule in self.0.iter() {
                w.write_u16(rule.query.min_width_px);
                w.write_u16(rule.query.max_width_px);
                w.write_u8(rule.query.flags);
                w.write_u8(rule.query.color_scheme);
                w.write_u8(rule.query.pointer);
                w.write_u8(0);
                patch_w.clear();
                encode_patch_bytes(&rule.patch, &mut patch_w)?;
                w.write_bytes_max(patch_w.as_slice(), MAX_STRING_BYTES)?;
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ITEMS, 12, |r| {
                let query = MediaQuery {
                    min_width_px: r.read_u16()?,
                    max_width_px: r.read_u16()?,
                    flags: r.read_u8()?,
                    color_scheme: r.read_u8()?,
                    pointer: r.read_u8()?,
                    _pad: r.read_u8()?,
                };
                let patch_raw = r.read_bytes(MAX_STRING_BYTES)?;
                let patch = decode_patch_bytes(patch_raw).ok_or(CodecError::BadMagic)?;
                Ok(ResponsiveRule { query, patch })
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateRules(pub Arc<[StateStyleRule]>);

impl StateRules {
    pub fn new(rules: impl Into<Arc<[StateStyleRule]>>) -> Self {
        Self(rules.into())
    }
}

impl Resource for StateRules {
    const KIND: Key64 = Key64::of("nve.res.state_rules");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_len_max(self.0.len(), MAX_ITEMS)?;
            let mut patch_w = ByteWriter::new();
            for rule in self.0.iter() {
                w.write_u8(rule.state.bits());
                patch_w.clear();
                encode_patch_bytes(&rule.patch, &mut patch_w)?;
                w.write_bytes_max(patch_w.as_slice(), MAX_STRING_BYTES)?;
            }
            Ok(())
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let out = r
            .read_vec(MAX_ITEMS, 5, |r| {
                let state = InteractionState::from_bits_truncate(r.read_u8()?);
                let patch_raw = r.read_bytes(MAX_STRING_BYTES)?;
                let patch = decode_patch_bytes(patch_raw).ok_or(CodecError::BadMagic)?;
                Ok(StateStyleRule { state, patch })
            })
            .ok()?;
        r.finish().ok()?;
        Some(Self(Arc::from(out)))
    }
}

fn encode_patch_bytes(patch: &StylePatch, w: &mut ByteWriter) -> Result<(), CodecError> {
    super::style_patch_codec::encode_style_patch(patch, w)
}

fn decode_patch_bytes(bytes: &[u8]) -> Option<StylePatch> {
    super::style_patch_codec::decode_style_patch_bytes(bytes)
}

// ---------------------------------------------------------------------------------------------
// AriaRelations
// ---------------------------------------------------------------------------------------------

/// ARIA ID-reference relations (`aria-controls`, `aria-labelledby`, `aria-describedby`,
/// `aria-activedescendant`). Empty strings are omitted on emission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AriaRelations {
    pub controls: String,
    pub labelled_by: String,
    pub described_by: String,
    pub active_descendant: String,
}

impl Resource for AriaRelations {
    const KIND: Key64 = Key64::of("nve.res.aria_relations");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| {
            w.write_str_max(&self.controls, MAX_STRING_BYTES)?;
            w.write_str_max(&self.labelled_by, MAX_STRING_BYTES)?;
            w.write_str_max(&self.described_by, MAX_STRING_BYTES)?;
            w.write_str_max(&self.active_descendant, MAX_STRING_BYTES)
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let rel = Self {
            controls: r.read_string(MAX_STRING_BYTES).ok()?,
            labelled_by: r.read_string(MAX_STRING_BYTES).ok()?,
            described_by: r.read_string(MAX_STRING_BYTES).ok()?,
            active_descendant: r.read_string(MAX_STRING_BYTES).ok()?,
        };
        r.finish().ok()?;
        Some(rel)
    }
}

// ---------------------------------------------------------------------------------------------
// ImageSrc
// ---------------------------------------------------------------------------------------------

/// Image source payload: either a URL/URI string or inline encoded image bytes (PNG/WebP/JPEG).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSrc {
    Url(String),
    EncodedBytes(Arc<[u8]>),
}

impl Resource for ImageSrc {
    const KIND: Key64 = Key64::of("nve.res.image_src");

    fn encode(&self, out: &mut Vec<u8>) {
        encode_into_or_rollback(out, |w| match self {
            Self::Url(u) => {
                w.write_u8(0);
                w.write_str_max(u, MAX_STRING_BYTES)
            }
            Self::EncodedBytes(b) => {
                w.write_u8(1);
                w.write_bytes_max(b, MAX_IMAGE_BYTES)
            }
        });
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = ByteReader::new(bytes);
        let src = match r.read_u8().ok()? {
            0 => Self::Url(r.read_string(MAX_STRING_BYTES).ok()?),
            1 => Self::EncodedBytes(Arc::from(r.read_bytes(MAX_IMAGE_BYTES).ok()?)),
            _ => return None,
        };
        r.finish().ok()?;
        Some(src)
    }
}
