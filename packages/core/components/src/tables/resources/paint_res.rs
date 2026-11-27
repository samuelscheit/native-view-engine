//! Flat `FlatResource` implementations for [`BoxShadows`], [`BackgroundFillRes`], and
//! [`Transitions`], with zero-copy borrowed views over `ResourceStore`'s `u32` slab.

use std::sync::Arc;

use nve_abi::{ColorRgba, Key64};
use nve_store::{FlatResource, Rgba};

use crate::motion::{EasingCurve, TransitionProperty, TransitionSpec};
use crate::paint::{
    write_css_color, write_css_gradient_view, BackgroundFill, GradientStop, LinearGradient,
    RadialGradient,
};
use crate::tables::box_shadow::BoxShadow;

use super::MAX_ITEMS;

// ---------------------------------------------------------------------------------------------
// BoxShadows (6 words/shadow)
// ---------------------------------------------------------------------------------------------

const SHADOW_STRIDE: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct BoxShadows(pub Arc<[BoxShadow]>);

impl BoxShadows {
    pub fn new(shadows: impl Into<Arc<[BoxShadow]>>) -> Self {
        Self(shadows.into())
    }
}

/// Zero-copy borrowed view over flat [`BoxShadows`] words (`[count, (ox,oy,blur,spread,rgba,inset)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxShadowsView<'a>(&'a [u32]);

impl<'a> BoxShadowsView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            let max_fit = (self.0.len() - 1) / SHADOW_STRIDE;
            let count = self.0[0] as usize;
            if count < max_fit { count } else { max_fit }
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<BoxShadow> {
        if index >= self.len() {
            return None;
        }
        let b = 1 + index * SHADOW_STRIDE;
        Some(BoxShadow {
            offset_x: f32::from_bits(self.0[b]),
            offset_y: f32::from_bits(self.0[b + 1]),
            blur_radius: f32::from_bits(self.0[b + 2]),
            spread_radius: f32::from_bits(self.0[b + 3]),
            color: Rgba(self.0[b + 4]),
            inset: self.0[b + 5] != 0,
        })
    }

    #[inline]
    pub fn iter(self) -> BoxShadowsIter<'a> {
        BoxShadowsIter {
            view: self,
            idx: 0,
            end: self.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoxShadowsIter<'a> {
    view: BoxShadowsView<'a>,
    idx: usize,
    end: usize,
}

impl Iterator for BoxShadowsIter<'_> {
    type Item = BoxShadow;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        let item = self.view.get(self.idx);
        self.idx += 1;
        item
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.end - self.idx;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for BoxShadowsIter<'_> {}

impl FlatResource for BoxShadows {
    const KIND: Key64 = Key64::of("nve.res.box_shadows");
    type View<'a> = BoxShadowsView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ITEMS {
            return false;
        }
        out.reserve(1 + self.0.len() * SHADOW_STRIDE);
        out.push(self.0.len() as u32);
        for s in self.0.iter() {
            out.push(s.offset_x.to_bits());
            out.push(s.offset_y.to_bits());
            out.push(s.blur_radius.to_bits());
            out.push(s.spread_radius.to_bits());
            out.push(s.color.0);
            out.push(u32::from(s.inset));
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count_u32) = words.first() else {
            return false;
        };
        let count = count_u32 as usize;
        if count > MAX_ITEMS {
            return false;
        }
        let Some(expected) = count.checked_mul(SHADOW_STRIDE).and_then(|n| n.checked_add(1)) else {
            return false;
        };
        if words.len() != expected {
            return false;
        }
        for chunk in words[1..].as_chunks::<SHADOW_STRIDE>().0 {
            if chunk[5] > 1 {
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn view(words: &[u32]) -> BoxShadowsView<'_> {
        BoxShadowsView(words)
    }
}

// ---------------------------------------------------------------------------------------------
// BackgroundFillRes
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct BackgroundFillRes(pub BackgroundFill);

/// Zero-copy borrowed view over flat gradient stops (`[(offset_bits, color_u32)*]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientStopsView<'a>(&'a [u32]);

impl<'a> GradientStopsView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        self.0.len() / 2
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<GradientStop> {
        let b = index.checked_mul(2)?;
        let off = *self.0.get(b)?;
        let col = *self.0.get(b + 1)?;
        Some(GradientStop {
            offset: f32::from_bits(off),
            color: ColorRgba::from_u32(col),
        })
    }

    #[inline]
    pub fn iter(self) -> GradientStopsIter<'a> {
        GradientStopsIter {
            view: self,
            idx: 0,
            end: self.len(),
        }
    }

    pub fn to_vec(self) -> Vec<GradientStop> {
        self.iter().collect()
    }
}

#[derive(Debug, Clone)]
pub struct GradientStopsIter<'a> {
    view: GradientStopsView<'a>,
    idx: usize,
    end: usize,
}

impl Iterator for GradientStopsIter<'_> {
    type Item = GradientStop;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        let item = self.view.get(self.idx);
        self.idx += 1;
        item
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.end - self.idx;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for GradientStopsIter<'_> {}

/// Zero-copy borrowed view over flat [`BackgroundFillRes`] words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundFillView<'a> {
    Solid(ColorRgba),
    Linear {
        angle_deg: f32,
        stops: GradientStopsView<'a>,
    },
    Radial {
        center_x: f32,
        center_y: f32,
        radius: f32,
        stops: GradientStopsView<'a>,
    },
}

impl BackgroundFillView<'_> {
    /// Materializes an owned [`BackgroundFill`] when needed by retained paint lists.
    pub fn to_owned(self) -> BackgroundFill {
        match self {
            Self::Solid(c) => BackgroundFill::Solid(c),
            Self::Linear { angle_deg, stops } => {
                BackgroundFill::Linear(LinearGradient::new(angle_deg, stops.to_vec()))
            }
            Self::Radial {
                center_x,
                center_y,
                radius,
                stops,
            } => BackgroundFill::Radial(RadialGradient::new(
                center_x,
                center_y,
                radius,
                stops.to_vec(),
            )),
        }
    }

    /// Writes CSS declaration(s) directly from this zero-copy view.
    pub fn write_css_decl_into(self, clear_image_on_solid: bool, out: &mut String) {
        match self {
            Self::Solid(c) => {
                if clear_image_on_solid {
                    out.push_str("background-image:none;background-color:");
                    write_css_color(c, out);
                    out.push(';');
                } else if c.a > 0 {
                    out.push_str("background-color:");
                    write_css_color(c, out);
                    out.push(';');
                }
            }
            Self::Linear { .. } | Self::Radial { .. } => {
                out.push_str("background-image:");
                write_css_gradient_view(self, out);
                out.push(';');
            }
        }
    }
}

impl FlatResource for BackgroundFillRes {
    const KIND: Key64 = Key64::of("nve.res.background_fill");
    type View<'a> = BackgroundFillView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        match &self.0 {
            BackgroundFill::Solid(c) => {
                out.extend_from_slice(&[0, c.to_u32()]);
                true
            }
            BackgroundFill::Linear(lg) => {
                if lg.stops.len() > MAX_ITEMS {
                    return false;
                }
                out.reserve(3 + lg.stops.len() * 2);
                out.push(1);
                out.push(lg.angle_deg.to_bits());
                out.push(lg.stops.len() as u32);
                for s in lg.stops.iter() {
                    out.push(s.offset.to_bits());
                    out.push(s.color.to_u32());
                }
                true
            }
            BackgroundFill::Radial(rg) => {
                if rg.stops.len() > MAX_ITEMS {
                    return false;
                }
                out.reserve(5 + rg.stops.len() * 2);
                out.push(2);
                out.push(rg.center_x.to_bits());
                out.push(rg.center_y.to_bits());
                out.push(rg.radius.to_bits());
                out.push(rg.stops.len() as u32);
                for s in rg.stops.iter() {
                    out.push(s.offset.to_bits());
                    out.push(s.color.to_u32());
                }
                true
            }
        }
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&tag) = words.first() else {
            return false;
        };
        match tag {
            0 => words.len() == 2,
            1 => {
                let Some(&count_u32) = words.get(2) else {
                    return false;
                };
                let count = count_u32 as usize;
                count <= MAX_ITEMS
                    && count
                        .checked_mul(2)
                        .and_then(|n| n.checked_add(3))
                        .is_some_and(|exp| words.len() == exp)
            }
            2 => {
                let Some(&count_u32) = words.get(4) else {
                    return false;
                };
                let count = count_u32 as usize;
                count <= MAX_ITEMS
                    && count
                        .checked_mul(2)
                        .and_then(|n| n.checked_add(5))
                        .is_some_and(|exp| words.len() == exp)
            }
            _ => false,
        }
    }

    #[inline]
    fn view(words: &[u32]) -> BackgroundFillView<'_> {
        match words.first().copied() {
            Some(0) => BackgroundFillView::Solid(ColorRgba::from_u32(
                words.get(1).copied().unwrap_or(0),
            )),
            Some(1) => BackgroundFillView::Linear {
                angle_deg: f32::from_bits(words.get(1).copied().unwrap_or(180.0f32.to_bits())),
                stops: GradientStopsView(words.get(3..).unwrap_or(&[])),
            },
            Some(2) => BackgroundFillView::Radial {
                center_x: f32::from_bits(words.get(1).copied().unwrap_or(0.5f32.to_bits())),
                center_y: f32::from_bits(words.get(2).copied().unwrap_or(0.5f32.to_bits())),
                radius: f32::from_bits(words.get(3).copied().unwrap_or(1.0f32.to_bits())),
                stops: GradientStopsView(words.get(5..).unwrap_or(&[])),
            },
            _ => BackgroundFillView::Solid(ColorRgba::TRANSPARENT),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Transitions (7 words/spec)
// ---------------------------------------------------------------------------------------------

const TRANSITION_STRIDE: usize = 7;

#[derive(Debug, Clone, PartialEq)]
pub struct Transitions(pub Arc<[TransitionSpec]>);

impl Transitions {
    pub fn new(specs: impl Into<Arc<[TransitionSpec]>>) -> Self {
        Self(specs.into())
    }
}

/// Zero-copy borrowed view over flat [`Transitions`] words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionsView<'a>(&'a [u32]);

impl<'a> TransitionsView<'a> {
    #[inline(always)]
    pub const fn len(self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            let max_fit = (self.0.len() - 1) / TRANSITION_STRIDE;
            let count = self.0[0] as usize;
            if count < max_fit { count } else { max_fit }
        }
    }

    #[inline(always)]
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<TransitionSpec> {
        if index >= self.len() {
            return None;
        }
        let b = 1 + index * TRANSITION_STRIDE;
        let hdr = self.0[b];
        let property = match hdr & 0xFF {
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
            _ => TransitionProperty::All,
        };
        let easing = match (hdr >> 8) & 0xFF {
            0 => EasingCurve::Linear,
            1 => EasingCurve::Ease,
            2 => EasingCurve::EaseIn,
            3 => EasingCurve::EaseOut,
            4 => EasingCurve::EaseInOut,
            5 => EasingCurve::CubicBezier(
                f32::from_bits(self.0[b + 3]),
                f32::from_bits(self.0[b + 4]),
                f32::from_bits(self.0[b + 5]),
                f32::from_bits(self.0[b + 6]),
            ),
            _ => EasingCurve::Ease,
        };
        Some(TransitionSpec {
            property,
            duration_ms: self.0[b + 1],
            delay_ms: self.0[b + 2],
            easing,
            reduced_motion_instant: ((hdr >> 16) & 0xFF) != 0,
        })
    }

    #[inline]
    pub fn iter(self) -> TransitionsIter<'a> {
        TransitionsIter {
            view: self,
            idx: 0,
            end: self.len(),
        }
    }

    pub fn to_vec(self) -> Vec<TransitionSpec> {
        self.iter().collect()
    }
}

#[derive(Debug, Clone)]
pub struct TransitionsIter<'a> {
    view: TransitionsView<'a>,
    idx: usize,
    end: usize,
}

impl Iterator for TransitionsIter<'_> {
    type Item = TransitionSpec;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.end {
            return None;
        }
        let item = self.view.get(self.idx);
        self.idx += 1;
        item
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.end - self.idx;
        (rem, Some(rem))
    }
}

impl ExactSizeIterator for TransitionsIter<'_> {}

impl FlatResource for Transitions {
    const KIND: Key64 = Key64::of("nve.res.transitions");
    type View<'a> = TransitionsView<'a>;

    fn write_words(&self, out: &mut Vec<u32>) -> bool {
        if self.0.len() > MAX_ITEMS {
            return false;
        }
        out.reserve(1 + self.0.len() * TRANSITION_STRIDE);
        out.push(self.0.len() as u32);
        for t in self.0.iter() {
            let (easing_tag, c0, c1, c2, c3) = match t.easing {
                EasingCurve::Linear => (0u32, 0, 0, 0, 0),
                EasingCurve::Ease => (1, 0, 0, 0, 0),
                EasingCurve::EaseIn => (2, 0, 0, 0, 0),
                EasingCurve::EaseOut => (3, 0, 0, 0, 0),
                EasingCurve::EaseInOut => (4, 0, 0, 0, 0),
                EasingCurve::CubicBezier(x1, y1, x2, y2) => {
                    (5, x1.to_bits(), y1.to_bits(), x2.to_bits(), y2.to_bits())
                }
            };
            let hdr = u32::from(t.property as u8)
                | (easing_tag << 8)
                | (u32::from(t.reduced_motion_instant) << 16);
            out.extend_from_slice(&[hdr, t.duration_ms, t.delay_ms, c0, c1, c2, c3]);
        }
        true
    }

    fn validate(words: &[u32]) -> bool {
        let Some(&count_u32) = words.first() else {
            return false;
        };
        let count = count_u32 as usize;
        if count > MAX_ITEMS {
            return false;
        }
        let Some(expected) = count
            .checked_mul(TRANSITION_STRIDE)
            .and_then(|n| n.checked_add(1))
        else {
            return false;
        };
        if words.len() != expected {
            return false;
        }
        for chunk in words[1..].as_chunks::<TRANSITION_STRIDE>().0 {
            let hdr = chunk[0];
            let prop = hdr & 0xFF;
            let easing_tag = (hdr >> 8) & 0xFF;
            let instant = (hdr >> 16) & 0xFF;
            if prop > 10 || easing_tag > 5 || instant > 1 || (hdr >> 24) != 0 {
                return false;
            }
            if easing_tag != 5 && (chunk[3] | chunk[4] | chunk[5] | chunk[6]) != 0 {
                return false;
            }
        }
        true
    }

    #[inline(always)]
    fn view(words: &[u32]) -> TransitionsView<'_> {
        TransitionsView(words)
    }
}
