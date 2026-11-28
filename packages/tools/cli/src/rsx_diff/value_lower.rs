//! Typed attribute-value evaluators lowering [`TypedAttrValue`] directly to `nve-store` /
//! `nve-tables` binary scalars (`Length`, `f32`, `i32`, `u16`, `Rgba`, `TextDecoration`).

use nve_abi::num::{is_valid_style_scalar, MAX_STYLE_SCALAR};
use nve_store::{Length, Rgba};
use nve_tables::tables::TextDecoration;

use super::attrs::{AttrKind, AttrSpec, NumberRule};
use super::eval::TypedAttrValue;

fn scalar_floor(spec: &AttrSpec) -> f32 {
    match spec.kind {
        AttrKind::Dimension { float } | AttrKind::Pixels { float } => match float {
            Some(NumberRule::Px { min }) => min,
            _ => 0.0,
        },
        _ => f32::NEG_INFINITY,
    }
}

fn check_domain(spec: &AttrSpec, v: f32) -> Result<f32, String> {
    let min = scalar_floor(spec);
    let ok = is_valid_style_scalar(v) && if min > 0.0 { v > 0.0 } else { v >= min };
    if ok {
        Ok(v)
    } else {
        Err(format!("`{}` scalar {v} is out of domain", spec.name))
    }
}

fn clamp_numeric_float(v: f64) -> f32 {
    if v.is_finite() {
        (v as f32).clamp(-MAX_STYLE_SCALAR, MAX_STYLE_SCALAR)
    } else {
        0.0
    }
}

pub(super) fn eval_length(spec: &AttrSpec, value: &TypedAttrValue) -> Result<Length, String> {
    let allow_pct_auto = matches!(spec.kind, AttrKind::Dimension { .. });
    match (spec.kind, value) {
        (
            AttrKind::Dimension {
                float: Some(NumberRule::Px { min }),
            }
            | AttrKind::Pixels {
                float: Some(NumberRule::Px { min }),
            },
            TypedAttrValue::Float(v),
        ) => {
            let f = clamp_numeric_float(*v).max(min);
            Ok(Length::points(check_domain(spec, f)?))
        }
        (_, TypedAttrValue::LengthPx(v)) => Ok(Length::points(check_domain(spec, *v)?)),
        (_, TypedAttrValue::LengthPct(v)) if allow_pct_auto => {
            Ok(Length::percent(check_domain(spec, *v)?))
        }
        (_, TypedAttrValue::Auto) if allow_pct_auto => Ok(Length::AUTO),
        (_, TypedAttrValue::String(s)) => parse_length_str(spec, s, allow_pct_auto),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

fn parse_length_str(spec: &AttrSpec, raw: &str, allow_pct_auto: bool) -> Result<Length, String> {
    let s = raw.trim();
    if allow_pct_auto && s.eq_ignore_ascii_case("auto") {
        return Ok(Length::AUTO);
    }
    if allow_pct_auto && let Some(num) = s.strip_suffix('%') {
        let v: f32 = num
            .trim()
            .parse()
            .map_err(|_| format!("`{}` invalid length {raw:?}", spec.name))?;
        return Ok(Length::percent(check_domain(spec, v)?));
    }
    let num = s.strip_suffix("px").unwrap_or(s).trim();
    let v: f32 = num
        .parse()
        .map_err(|_| format!("`{}` invalid length {raw:?}", spec.name))?;
    Ok(Length::points(check_domain(spec, v)?))
}

pub(super) fn eval_px_scalar(spec: &AttrSpec, value: &TypedAttrValue) -> Result<f32, String> {
    match (spec.kind, value) {
        (
            AttrKind::Pixels {
                float: Some(NumberRule::Px { min }),
            },
            TypedAttrValue::Float(v),
        ) => {
            let f = clamp_numeric_float(*v).max(min);
            check_domain(spec, f)
        }
        (_, TypedAttrValue::LengthPx(v)) => check_domain(spec, *v),
        (_, TypedAttrValue::String(s)) => {
            let num = s.trim().strip_suffix("px").unwrap_or(s.trim()).trim();
            let v: f32 = num
                .parse()
                .map_err(|_| format!("`{}` invalid px scalar {s:?}", spec.name))?;
            check_domain(spec, v)
        }
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_unitless(spec: &AttrSpec, value: &TypedAttrValue) -> Result<f32, String> {
    let AttrKind::Number(NumberRule::Unitless { min, max }) = spec.kind else {
        return Err(format!("`{}` is not a unitless number", spec.name));
    };
    let raw = match value {
        TypedAttrValue::Float(v) => clamp_numeric_float(*v).clamp(min, max),
        TypedAttrValue::String(s) => s
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("`{}` invalid number {s:?}", spec.name))?
            .clamp(min, max),
        _ => return Err(format!("`{}` cannot carry {value}", spec.name)),
    };
    if is_valid_style_scalar(raw) {
        Ok(raw)
    } else {
        Err(format!("`{}` value {raw} is out of domain", spec.name))
    }
}

pub(super) fn eval_z_index(spec: &AttrSpec, value: &TypedAttrValue) -> Result<i32, String> {
    match value {
        TypedAttrValue::Int(v) => Ok((*v).clamp(i32::MIN.into(), i32::MAX.into()) as i32),
        TypedAttrValue::Float(v) => Ok(clamp_numeric_float(*v) as i32),
        TypedAttrValue::String(s) => s
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("`{}` invalid z-index {s:?}", spec.name)),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_color(spec: &AttrSpec, value: &TypedAttrValue) -> Result<Rgba, String> {
    match value {
        TypedAttrValue::Rgba { r, g, b, a } => Ok(Rgba::new(*r, *g, *b, *a)),
        TypedAttrValue::String(s) => parse_color_str(s)
            .ok_or_else(|| format!("`{}` cannot parse color {s:?}", spec.name)),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

fn parse_color_str(raw: &str) -> Option<Rgba> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("transparent") {
        return Some(Rgba::TRANSPARENT);
    }
    if s.eq_ignore_ascii_case("white") {
        return Some(Rgba::WHITE);
    }
    if s.eq_ignore_ascii_case("black") {
        return Some(Rgba::BLACK);
    }
    let hex = s.strip_prefix('#')?;
    let h = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    let h1 = |i: usize| {
        let d = u8::from_str_radix(hex.get(i..i + 1)?, 16).ok()?;
        Some(d * 17)
    };
    match hex.len() {
        3 => Some(Rgba::rgb(h1(0)?, h1(1)?, h1(2)?)),
        4 => Some(Rgba::new(h1(0)?, h1(1)?, h1(2)?, h1(3)?)),
        6 => Some(Rgba::rgb(h(0)?, h(2)?, h(4)?)),
        8 => Some(Rgba::new(h(0)?, h(2)?, h(4)?, h(6)?)),
        _ => None,
    }
}

pub(super) fn eval_font_weight(spec: &AttrSpec, value: &TypedAttrValue) -> Result<u16, String> {
    match value {
        TypedAttrValue::Int(v) if (1..=1000).contains(v) => Ok(*v as u16),
        TypedAttrValue::String(s) => parse_font_weight_str(s)
            .ok_or_else(|| format!("`{}` invalid font weight {s:?}", spec.name)),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

fn parse_font_weight_str(raw: &str) -> Option<u16> {
    let s = raw.trim();
    if let Ok(n) = s.parse::<u16>()
        && (1..=1000).contains(&n)
    {
        return Some(n);
    }
    const WEIGHTS: &[(&str, u16)] = &[
        ("thin", 100),
        ("extralight", 200),
        ("ultralight", 200),
        ("light", 300),
        ("normal", 400),
        ("regular", 400),
        ("medium", 500),
        ("semibold", 600),
        ("demibold", 600),
        ("bold", 700),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("black", 900),
        ("heavy", 900),
    ];
    WEIGHTS
        .iter()
        .find_map(|&(k, w)| s.eq_ignore_ascii_case(k).then_some(w))
}

pub(super) fn eval_line_height(spec: &AttrSpec, value: &TypedAttrValue) -> Result<Length, String> {
    match value {
        TypedAttrValue::Auto => Ok(Length::AUTO),
        TypedAttrValue::LengthPx(v) if is_valid_style_scalar(*v) && *v > 0.0 => {
            Ok(Length::points(*v))
        }
        TypedAttrValue::LengthPct(v) if is_valid_style_scalar(*v) && *v > 0.0 => {
            Ok(Length::percent(*v))
        }
        TypedAttrValue::Float(v) => {
            let f = clamp_numeric_float(*v);
            if is_valid_style_scalar(f) && f > 0.0 {
                Ok(Length::percent(f * 100.0))
            } else {
                Err(format!("`{}` ratio {f} must be positive", spec.name))
            }
        }
        TypedAttrValue::String(s) => {
            let t = s.trim();
            if t.eq_ignore_ascii_case("normal") || t.eq_ignore_ascii_case("auto") {
                return Ok(Length::AUTO);
            }
            if let Some(num) = t.strip_suffix("px") {
                let v: f32 = num
                    .trim()
                    .parse()
                    .map_err(|_| format!("`{}` invalid line-height {s:?}", spec.name))?;
                return (is_valid_style_scalar(v) && v > 0.0)
                    .then(|| Length::points(v))
                    .ok_or_else(|| format!("`{}` out of domain: {s:?}", spec.name));
            }
            if let Some(num) = t.strip_suffix('%') {
                let v: f32 = num
                    .trim()
                    .parse()
                    .map_err(|_| format!("`{}` invalid line-height {s:?}", spec.name))?;
                return (is_valid_style_scalar(v) && v > 0.0)
                    .then(|| Length::percent(v))
                    .ok_or_else(|| format!("`{}` out of domain: {s:?}", spec.name));
            }
            let ratio: f32 = t
                .parse()
                .map_err(|_| format!("`{}` invalid line-height {s:?}", spec.name))?;
            (is_valid_style_scalar(ratio) && ratio > 0.0)
                .then(|| Length::percent(ratio * 100.0))
                .ok_or_else(|| format!("`{}` ratio {ratio} must be positive", spec.name))
        }
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_text_decoration(
    spec: &AttrSpec,
    value: &TypedAttrValue,
) -> Result<TextDecoration, String> {
    let TypedAttrValue::String(s) = value else {
        return Err(format!("`{}` cannot carry {value}", spec.name));
    };
    let mut flags = TextDecoration::EMPTY;
    let mut seen = false;
    for tok in s.split_ascii_whitespace() {
        seen = true;
        if tok.eq_ignore_ascii_case("none") {
            return Ok(TextDecoration::EMPTY);
        } else if tok.eq_ignore_ascii_case("underline") {
            flags |= TextDecoration::UNDERLINE;
        } else if tok.eq_ignore_ascii_case("overline") {
            flags |= TextDecoration::OVERLINE;
        } else if tok.eq_ignore_ascii_case("line-through") {
            flags |= TextDecoration::LINE_THROUGH;
        } else {
            return Err(format!(
                "`{}` unknown text-decoration token {tok:?}",
                spec.name
            ));
        }
    }
    if seen {
        Ok(flags)
    } else {
        Err(format!("`{}` empty text-decoration string", spec.name))
    }
}
