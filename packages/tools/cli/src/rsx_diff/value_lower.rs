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
        AttrKind::StandalonePx { min } => min,
        _ => f32::NEG_INFINITY,
    }
}

fn check_domain(spec: &AttrSpec, v: f32) -> Result<f32, String> {
    let min = scalar_floor(spec);
    if is_valid_style_scalar(v) && v >= min {
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
        (AttrKind::Percent, TypedAttrValue::Float(v)) => {
            let f = clamp_numeric_float(*v).clamp(0.0, 100.0);
            Ok(Length::percent(f))
        }
        (
            AttrKind::Dimension {
                float: Some(NumberRule::Px { min }),
            }
            | AttrKind::Pixels {
                float: Some(NumberRule::Px { min }),
            }
            | AttrKind::StandalonePx { min },
            TypedAttrValue::Float(v),
        ) => {
            let f = clamp_numeric_float(*v).max(min);
            Ok(Length::points(check_domain(spec, f)?))
        }
        (AttrKind::Dimension { .. } | AttrKind::Pixels { .. }, TypedAttrValue::LengthPx(v)) => {
            Ok(Length::points(check_domain(spec, *v)?))
        }
        (_, TypedAttrValue::LengthPct(v)) if allow_pct_auto => {
            Ok(Length::percent(check_domain(spec, *v)?))
        }
        (_, TypedAttrValue::Auto) if allow_pct_auto => Ok(Length::AUTO),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_px_scalar(spec: &AttrSpec, value: &TypedAttrValue) -> Result<f32, String> {
    match (spec.kind, value) {
        (
            AttrKind::Pixels {
                float: Some(NumberRule::Px { min }),
            }
            | AttrKind::StandalonePx { min },
            TypedAttrValue::Float(v),
        ) => {
            let f = clamp_numeric_float(*v).max(min);
            check_domain(spec, f)
        }
        (AttrKind::Pixels { .. }, TypedAttrValue::LengthPx(v)) => check_domain(spec, *v),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_unitless(spec: &AttrSpec, value: &TypedAttrValue) -> Result<f32, String> {
    let AttrKind::Number(NumberRule::Unitless { min, max }) = spec.kind else {
        return Err(format!("`{}` is not a unitless number", spec.name));
    };
    let raw = match value {
        TypedAttrValue::Float(v) => clamp_numeric_float(*v).clamp(min, max),
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
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_color(spec: &AttrSpec, value: &TypedAttrValue) -> Result<Rgba, String> {
    match value {
        TypedAttrValue::Rgba { r, g, b, a } => Ok(Rgba::new(*r, *g, *b, *a)),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_font_weight(spec: &AttrSpec, value: &TypedAttrValue) -> Result<u16, String> {
    match value {
        TypedAttrValue::Int(v) if (1..=1000).contains(v) => Ok(*v as u16),
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
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
            let pct = f * 100.0;
            if is_valid_style_scalar(pct) && pct > 0.0 {
                Ok(Length::percent(pct))
            } else {
                Err(format!("`{}` ratio {f} is out of domain", spec.name))
            }
        }
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}

pub(super) fn eval_text_decoration(
    spec: &AttrSpec,
    value: &TypedAttrValue,
) -> Result<TextDecoration, String> {
    match value {
        TypedAttrValue::Enum { ty, variant }
            if ty.as_deref().is_none_or(|t| t == "TextDecoration") =>
        {
            match variant.as_str() {
                "EMPTY" => Ok(TextDecoration::EMPTY),
                "UNDERLINE" => Ok(TextDecoration::UNDERLINE),
                "OVERLINE" => Ok(TextDecoration::OVERLINE),
                "LINE_THROUGH" => Ok(TextDecoration::LINE_THROUGH),
                _ => Err(format!("`{}` unknown TextDecoration::{variant}", spec.name)),
            }
        }
        TypedAttrValue::String(s) => {
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
        _ => Err(format!("`{}` cannot carry {value}", spec.name)),
    }
}
