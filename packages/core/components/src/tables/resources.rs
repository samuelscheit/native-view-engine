//! Deduplicated `nve_store::Resource` blobs referenced by core word tables.
//!
//! In-process backends read the typed value directly (zero encode/decode); binary codecs run only
//! for XXH64 deduplication and cross-boundary `NVE2` frames.

use std::sync::Arc;

use nve_abi::Key64;
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

// ---------------------------------------------------------------------------------------------
// Compact binary reader/writer helpers
// ---------------------------------------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn is_empty(&self) -> bool {
        self.pos == self.bytes.len()
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn count(&mut self, max: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= max).then_some(n)
    }
    fn str_owned(&mut self) -> Option<String> {
        let len = self.count(MAX_STRING_BYTES)?;
        let raw = self.take(len)?;
        std::str::from_utf8(raw).ok().map(str::to_owned)
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

fn write_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_grid_unit(out: &mut Vec<u8>, u: GridTrackUnit) {
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

fn read_grid_unit(r: &mut Reader<'_>) -> Option<GridTrackUnit> {
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
            write_grid_unit(out, min);
            write_grid_unit(out, max);
        }
    }
}

fn read_single_track(r: &mut Reader<'_>) -> Option<GridSingleTrack> {
    Some(match r.u8()? {
        0 => GridSingleTrack::Auto,
        1 => GridSingleTrack::Px(r.f32()?),
        2 => GridSingleTrack::Percent(r.f32()?),
        3 => GridSingleTrack::Fr(r.f32()?),
        4 => GridSingleTrack::MinMax(read_grid_unit(r)?, read_grid_unit(r)?),
        _ => return None,
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
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        for &track in self.0.iter() {
            match track {
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
                    write_grid_unit(out, min);
                    write_grid_unit(out, max);
                }
                GridTrack::Repeat(count, single) => {
                    out.push(5);
                    out.extend_from_slice(&count.to_le_bytes());
                    write_single_track(out, single);
                }
            }
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let n = r.count(MAX_ITEMS)?;
        let mut out = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let t = match r.u8()? {
                0 => GridTrack::Auto,
                1 => GridTrack::Px(r.f32()?),
                2 => GridTrack::Percent(r.f32()?),
                3 => GridTrack::Fr(r.f32()?),
                4 => GridTrack::MinMax(read_grid_unit(&mut r)?, read_grid_unit(&mut r)?),
                5 => GridTrack::Repeat(r.u16()?, read_single_track(&mut r)?),
                _ => return None,
            };
            out.push(t);
        }
        r.is_empty().then(|| Self(Arc::from(out)))
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
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        for s in self.0.iter() {
            out.extend_from_slice(&canon_f32_bytes(s.offset_x));
            out.extend_from_slice(&canon_f32_bytes(s.offset_y));
            out.extend_from_slice(&canon_f32_bytes(s.blur_radius));
            out.extend_from_slice(&canon_f32_bytes(s.spread_radius));
            out.extend_from_slice(&s.color.0.to_le_bytes());
            out.push(u8::from(s.inset));
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let n = r.count(MAX_ITEMS)?;
        let mut out = Vec::with_capacity(n.min(16));
        for _ in 0..n {
            out.push(BoxShadow {
                offset_x: r.f32()?,
                offset_y: r.f32()?,
                blur_radius: r.f32()?,
                spread_radius: r.f32()?,
                color: nve_store::Rgba(r.u32()?),
                inset: match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return None,
                },
            });
        }
        r.is_empty().then(|| Self(Arc::from(out)))
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
        match &self.0 {
            BackgroundFill::Solid(c) => {
                out.push(0);
                out.extend_from_slice(&c.to_u32().to_le_bytes());
            }
            BackgroundFill::Linear(lg) => {
                out.push(1);
                out.extend_from_slice(&canon_f32_bytes(lg.angle_deg));
                out.extend_from_slice(&(lg.stops.len() as u32).to_le_bytes());
                for s in lg.stops.iter() {
                    out.extend_from_slice(&canon_f32_bytes(s.offset));
                    out.extend_from_slice(&s.color.to_u32().to_le_bytes());
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
                    out.extend_from_slice(&s.color.to_u32().to_le_bytes());
                }
            }
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let fill = match r.u8()? {
            0 => BackgroundFill::Solid(nve_abi::ColorRgba::from_u32(r.u32()?)),
            1 => {
                let angle = r.f32()?;
                let n = r.count(MAX_ITEMS)?;
                let mut stops = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    stops.push(GradientStop::new(
                        r.f32()?,
                        nve_abi::ColorRgba::from_u32(r.u32()?),
                    ));
                }
                BackgroundFill::Linear(LinearGradient::new(angle, stops))
            }
            2 => {
                let cx = r.f32()?;
                let cy = r.f32()?;
                let rad = r.f32()?;
                let n = r.count(MAX_ITEMS)?;
                let mut stops = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    stops.push(GradientStop::new(
                        r.f32()?,
                        nve_abi::ColorRgba::from_u32(r.u32()?),
                    ));
                }
                BackgroundFill::Radial(RadialGradient::new(cx, cy, rad, stops))
            }
            _ => return None,
        };
        r.is_empty().then_some(Self(fill))
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
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        for t in self.0.iter() {
            out.push(t.property as u8);
            out.extend_from_slice(&t.duration_ms.to_le_bytes());
            out.extend_from_slice(&t.delay_ms.to_le_bytes());
            match t.easing {
                EasingCurve::Linear => out.push(0),
                EasingCurve::Ease => out.push(1),
                EasingCurve::EaseIn => out.push(2),
                EasingCurve::EaseOut => out.push(3),
                EasingCurve::EaseInOut => out.push(4),
                EasingCurve::CubicBezier(x1, y1, x2, y2) => {
                    out.push(5);
                    out.extend_from_slice(&canon_f32_bytes(x1));
                    out.extend_from_slice(&canon_f32_bytes(y1));
                    out.extend_from_slice(&canon_f32_bytes(x2));
                    out.extend_from_slice(&canon_f32_bytes(y2));
                }
            }
            out.push(u8::from(t.reduced_motion_instant));
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let n = r.count(MAX_ITEMS)?;
        let mut out = Vec::with_capacity(n.min(32));
        for _ in 0..n {
            let prop = match r.u8()? {
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
                _ => return None,
            };
            let duration_ms = r.u32()?;
            let delay_ms = r.u32()?;
            let easing = match r.u8()? {
                0 => EasingCurve::Linear,
                1 => EasingCurve::Ease,
                2 => EasingCurve::EaseIn,
                3 => EasingCurve::EaseOut,
                4 => EasingCurve::EaseInOut,
                5 => EasingCurve::CubicBezier(r.f32()?, r.f32()?, r.f32()?, r.f32()?),
                _ => return None,
            };
            let reduced_motion_instant = match r.u8()? {
                0 => false,
                1 => true,
                _ => return None,
            };
            out.push(TransitionSpec {
                property: prop,
                duration_ms,
                delay_ms,
                easing,
                reduced_motion_instant,
            });
        }
        r.is_empty().then(|| Self(Arc::from(out)))
    }
}

// ---------------------------------------------------------------------------------------------
// ResponsiveRules & StateRules (delegate StylePatch encoding to `extended_wire::style_patch`
// until StylePatch itself migrates to word-patches)
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
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        let mut patch_buf = Vec::new();
        for rule in self.0.iter() {
            out.extend_from_slice(&rule.query.min_width_px.to_le_bytes());
            out.extend_from_slice(&rule.query.max_width_px.to_le_bytes());
            out.push(rule.query.flags);
            out.push(rule.query.color_scheme);
            out.push(rule.query.pointer);
            out.push(0);
            patch_buf.clear();
            encode_patch_bytes(&rule.patch, &mut patch_buf);
            out.extend_from_slice(&(patch_buf.len() as u32).to_le_bytes());
            out.extend_from_slice(&patch_buf);
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let n = r.count(MAX_ITEMS)?;
        let mut out = Vec::with_capacity(n.min(32));
        for _ in 0..n {
            let query = MediaQuery {
                min_width_px: r.u16()?,
                max_width_px: r.u16()?,
                flags: r.u8()?,
                color_scheme: r.u8()?,
                pointer: r.u8()?,
                _pad: r.u8()?,
            };
            let len = r.count(MAX_STRING_BYTES)?;
            let patch = decode_patch_bytes(r.take(len)?)?;
            out.push(ResponsiveRule { query, patch });
        }
        r.is_empty().then(|| Self(Arc::from(out)))
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
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        let mut patch_buf = Vec::new();
        for rule in self.0.iter() {
            out.push(rule.state.bits());
            patch_buf.clear();
            encode_patch_bytes(&rule.patch, &mut patch_buf);
            out.extend_from_slice(&(patch_buf.len() as u32).to_le_bytes());
            out.extend_from_slice(&patch_buf);
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let n = r.count(MAX_ITEMS)?;
        let mut out = Vec::with_capacity(n.min(32));
        for _ in 0..n {
            let state = InteractionState::from_bits_truncate(r.u8()?);
            let len = r.count(MAX_STRING_BYTES)?;
            let patch = decode_patch_bytes(r.take(len)?)?;
            out.push(StateStyleRule { state, patch });
        }
        r.is_empty().then(|| Self(Arc::from(out)))
    }
}

fn encode_patch_bytes(patch: &StylePatch, out: &mut Vec<u8>) {
    super::style_patch_codec::encode_style_patch(patch, out);
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
        write_str(out, &self.controls);
        write_str(out, &self.labelled_by);
        write_str(out, &self.described_by);
        write_str(out, &self.active_descendant);
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let rel = Self {
            controls: r.str_owned()?,
            labelled_by: r.str_owned()?,
            described_by: r.str_owned()?,
            active_descendant: r.str_owned()?,
        };
        r.is_empty().then_some(rel)
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
        match self {
            Self::Url(u) => {
                out.push(0);
                write_str(out, u);
            }
            Self::EncodedBytes(b) => {
                out.push(1);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b);
            }
        }
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut r = Reader::new(bytes);
        let src = match r.u8()? {
            0 => Self::Url(r.str_owned()?),
            1 => {
                let len = r.count(MAX_IMAGE_BYTES)?;
                Self::EncodedBytes(Arc::from(r.take(len)?))
            }
            _ => return None,
        };
        r.is_empty().then_some(src)
    }
}

