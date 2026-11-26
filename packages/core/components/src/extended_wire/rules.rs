//! Binary wire codecs for `MediaQuery`, `StateStyleRule`, `ResponsiveRule`, and `TransitionSpec`
//! (cross-boundary `BatchSink` only).

use crate::interaction::{CursorKind, InteractionState, StateStyleRule};
use crate::media::{MediaQuery, ResponsiveRule};
use crate::motion::{EasingCurve, TransitionProperty, TransitionSpec};

use super::style_patch::{decode_style_patch, encode_style_patch};
use super::WireReader;

/// Encodes [`CursorKind`] into 1 byte.
#[inline]
pub fn encode_cursor(cursor: CursorKind, out: &mut Vec<u8>) {
    out.push(cursor as u8);
}

/// Decodes [`CursorKind`] from 1 byte.
#[inline]
pub fn decode_cursor(bytes: &[u8]) -> Option<CursorKind> {
    if bytes.len() != 1 {
        return None;
    }
    CursorKind::from_u8(bytes[0])
}

/// Encodes [`MediaQuery`] into 8 LE bytes.
#[inline]
pub fn encode_media_query(q: MediaQuery, out: &mut Vec<u8>) {
    out.extend_from_slice(&q.min_width_px.to_le_bytes());
    out.extend_from_slice(&q.max_width_px.to_le_bytes());
    out.push(q.flags);
    out.push(q.color_scheme);
    out.push(q.pointer);
    out.push(0);
}

/// Decodes [`MediaQuery`] from 8 LE bytes.
#[inline]
pub fn decode_media_query(bytes: &[u8]) -> Option<MediaQuery> {
    let mut r = WireReader::new(bytes);
    let q = read_media_query(&mut r)?;
    r.is_empty().then_some(q)
}

#[inline]
fn read_media_query(r: &mut WireReader<'_>) -> Option<MediaQuery> {
    let min_width_px = r.read_u16()?;
    let max_width_px = r.read_u16()?;
    let flags = r.read_u8()?;
    let color_scheme = r.read_u8()?;
    let pointer = r.read_u8()?;
    let _pad = r.read_u8()?;
    Some(MediaQuery {
        min_width_px,
        max_width_px,
        flags,
        color_scheme,
        pointer,
        _pad: 0,
    })
}

/// Encodes a slice of [`StateStyleRule`]s into `out`.
pub fn encode_state_rules(rules: &[StateStyleRule], out: &mut Vec<u8>) {
    out.extend_from_slice(&(rules.len() as u32).to_le_bytes());
    for rule in rules {
        out.push(rule.state.bits());
        encode_style_patch(&rule.patch, out);
    }
}

/// Decodes [`StateStyleRule`]s from wire bytes.
pub fn decode_state_rules(bytes: &[u8]) -> Option<Vec<StateStyleRule>> {
    let mut r = WireReader::new(bytes);
    let n = r.read_count()?;
    let mut out = Vec::with_capacity(n.min(32));
    for _ in 0..n {
        let state = InteractionState::from_bits_truncate(r.read_u8()?);
        let patch = decode_style_patch(&mut r)?;
        out.push(StateStyleRule { state, patch });
    }
    r.is_empty().then_some(out)
}

/// Encodes a slice of [`ResponsiveRule`]s into `out`.
pub fn encode_responsive_rules(rules: &[ResponsiveRule], out: &mut Vec<u8>) {
    out.extend_from_slice(&(rules.len() as u32).to_le_bytes());
    for rule in rules {
        encode_media_query(rule.query, out);
        encode_style_patch(&rule.patch, out);
    }
}

/// Decodes [`ResponsiveRule`]s from wire bytes.
pub fn decode_responsive_rules(bytes: &[u8]) -> Option<Vec<ResponsiveRule>> {
    let mut r = WireReader::new(bytes);
    let n = r.read_count()?;
    let mut out = Vec::with_capacity(n.min(32));
    for _ in 0..n {
        let query = read_media_query(&mut r)?;
        let patch = decode_style_patch(&mut r)?;
        out.push(ResponsiveRule { query, patch });
    }
    r.is_empty().then_some(out)
}

fn decode_transition_property(raw: u8) -> Option<TransitionProperty> {
    match raw {
        0 => Some(TransitionProperty::All),
        1 => Some(TransitionProperty::BackgroundColor),
        2 => Some(TransitionProperty::Color),
        3 => Some(TransitionProperty::BorderColor),
        4 => Some(TransitionProperty::Opacity),
        5 => Some(TransitionProperty::BorderRadius),
        6 => Some(TransitionProperty::Transform),
        7 => Some(TransitionProperty::Width),
        8 => Some(TransitionProperty::Height),
        9 => Some(TransitionProperty::Padding),
        10 => Some(TransitionProperty::Gap),
        _ => None,
    }
}

/// Encodes a slice of [`TransitionSpec`]s into `out`.
pub fn encode_transitions(specs: &[TransitionSpec], out: &mut Vec<u8>) {
    out.extend_from_slice(&(specs.len() as u32).to_le_bytes());
    for s in specs {
        out.push(s.property as u8);
        out.extend_from_slice(&s.duration_ms.to_le_bytes());
        out.extend_from_slice(&s.delay_ms.to_le_bytes());
        match s.easing {
            EasingCurve::Linear => out.push(0),
            EasingCurve::Ease => out.push(1),
            EasingCurve::EaseIn => out.push(2),
            EasingCurve::EaseOut => out.push(3),
            EasingCurve::EaseInOut => out.push(4),
            EasingCurve::CubicBezier(x1, y1, x2, y2) => {
                out.push(5);
                out.extend_from_slice(&x1.to_le_bytes());
                out.extend_from_slice(&y1.to_le_bytes());
                out.extend_from_slice(&x2.to_le_bytes());
                out.extend_from_slice(&y2.to_le_bytes());
            }
        }
        out.push(u8::from(s.reduced_motion_instant));
    }
}

/// Decodes [`TransitionSpec`]s from wire bytes.
pub fn decode_transitions(bytes: &[u8]) -> Option<Vec<TransitionSpec>> {
    let mut r = WireReader::new(bytes);
    let n = r.read_count()?;
    let mut out = Vec::with_capacity(n.min(32));
    for _ in 0..n {
        let property = decode_transition_property(r.read_u8()?)?;
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
            _ => return None,
        };
        let reduced_motion_instant = r.read_bool()?;
        out.push(TransitionSpec {
            property,
            duration_ms,
            delay_ms,
            easing,
            reduced_motion_instant,
        });
    }
    r.is_empty().then_some(out)
}
