//! Shared CSS/RN inline style parsers and bitmask-based style-layer diffing (`Layout`, `Visual`,
//! `Typography`) backed by a single-source property table.

use nve_abi::num::parse_f32;
use nve_abi::{ColorRgba, Dimension};
use nve_store::{Length, NodeId, Transaction};

#[path = "css_decl.rs"]
mod decl;
use decl::{parse_css_decl, write_parsed_decl};

/// Style-layer bit for [`Layout`].
pub const STYLE_LAYER_LAYOUT: u8 = 1 << 0;
/// Style-layer bit for [`Visual`].
pub const STYLE_LAYER_VISUAL: u8 = 1 << 1;
/// Style-layer bit for [`Typography`].
pub const STYLE_LAYER_TYPOGRAPHY: u8 = 1 << 2;
/// Style-layer mask covering all three core CSS style tables (`Layout`, `Visual`, `Typography`).
pub const STYLE_LAYER_ALL: u8 = STYLE_LAYER_LAYOUT | STYLE_LAYER_VISUAL | STYLE_LAYER_TYPOGRAPHY;

// Canonical property bits partitioned by style table so dropped-layer detection is branch-minimal.
const BIT_WIDTH: u64 = 1 << 0;
const BIT_HEIGHT: u64 = 1 << 1;
const BIT_MIN_WIDTH: u64 = 1 << 2;
const BIT_MIN_HEIGHT: u64 = 1 << 3;
const BIT_MAX_WIDTH: u64 = 1 << 4;
const BIT_MAX_HEIGHT: u64 = 1 << 5;
const BIT_DISPLAY: u64 = 1 << 6;
const BIT_FLEX_DIRECTION: u64 = 1 << 7;
const BIT_FLEX_WRAP: u64 = 1 << 8;
const BIT_JUSTIFY_CONTENT: u64 = 1 << 9;
const BIT_ALIGN_ITEMS: u64 = 1 << 10;
const BIT_FLEX_GROW: u64 = 1 << 11;
const BIT_FLEX_SHRINK: u64 = 1 << 12;
const BIT_ROW_GAP: u64 = 1 << 13;
const BIT_COLUMN_GAP: u64 = 1 << 14;
const BIT_PADDING_H: u64 = 1 << 15;
const BIT_PADDING_V: u64 = 1 << 16;
const BIT_MARGIN: u64 = 1 << 17;
const BIT_FLEX_BASIS: u64 = 1 << 18;
const LAYOUT_PROPS_MASK: u64 = (1 << 19) - 1;

const BIT_BACKGROUND: u64 = 1 << 19;
const BIT_BORDER_COLOR: u64 = 1 << 20;
const BIT_BORDER_WIDTH: u64 = 1 << 21;
const BIT_BORDER_RADIUS: u64 = 1 << 22;
const BIT_OPACITY: u64 = 1 << 23;
const BIT_VISIBILITY: u64 = 1 << 24;
const BIT_OVERFLOW_X: u64 = 1 << 25;
const BIT_OVERFLOW_Y: u64 = 1 << 26;
const BIT_BOX_SHADOW: u64 = 1 << 27;
const BIT_Z_INDEX: u64 = 1 << 28;
const VISUAL_PROPS_MASK: u64 = ((1 << 29) - 1) & !LAYOUT_PROPS_MASK;

const BIT_COLOR: u64 = 1 << 29;
const BIT_OVERFLOW_WRAP: u64 = 1 << 30;
const BIT_WORD_BREAK: u64 = 1 << 31;
const BIT_WHITE_SPACE: u64 = 1 << 32;
const BIT_LINE_HEIGHT: u64 = 1 << 33;
const BIT_FONT_SIZE: u64 = 1 << 34;
const BIT_FONT_WEIGHT: u64 = 1 << 35;
const BIT_TEXT_ALIGN: u64 = 1 << 36;
const BIT_FONT_FAMILY: u64 = 1 << 37;
const BIT_FONT_STYLE: u64 = 1 << 38;
const BIT_TEXT_DECORATION: u64 = 1 << 39;
const TYPOGRAPHY_PROPS_MASK: u64 = ((1 << 40) - 1) & !(LAYOUT_PROPS_MASK | VISUAL_PROPS_MASK);

/// Parses a pixel-suffixed or unitless CSS numeric scalar.
pub fn parse_px(raw: &str) -> Option<f32> {
    let s = raw.trim().strip_suffix("px").unwrap_or(raw.trim());
    parse_f32(s)
}

/// Strictly parses a CSS dimension (`auto`, `50%`, `120px`, `120`), returning `None` on invalid text.
pub fn try_parse_dim(raw: &str) -> Option<Dimension> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("auto") {
        Some(Dimension::Auto)
    } else if let Some(pct) = s.strip_suffix('%') {
        parse_f32(pct.trim()).map(Dimension::Percent)
    } else {
        parse_px(s).map(Dimension::Points)
    }
}

/// Parses a CSS dimension (`auto`, `50%`, `120px`, `120`) into [`Dimension`], defaulting to `Auto`.
pub fn parse_dim(raw: &str) -> Dimension {
    try_parse_dim(raw).unwrap_or(Dimension::Auto)
}

/// Converts a [`Dimension`] into a word-table [`Length`].
#[inline]
pub const fn dim_to_length(d: Dimension) -> Length {
    match d {
        Dimension::Auto => Length::AUTO,
        Dimension::Points(v) => Length::points(v),
        Dimension::Percent(v) => Length::percent(v),
    }
}

/// Parses `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(...)`, `rgba(...)`, or basic named colors.
pub fn parse_color(raw: &str) -> Option<ColorRgba> {
    let s = raw.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.is_ascii() {
            return None;
        }
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(ColorRgba::rgb(r, g, b))
            }
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
                Some(ColorRgba::rgba(r, g, b, a))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(ColorRgba::rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(ColorRgba::rgba(r, g, b, a))
            }
            _ => None,
        };
    }

    let rgb_inner = if s.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("rgba(")) {
        s[5..].strip_suffix(')')
    } else if s.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("rgb(")) {
        s[4..].strip_suffix(')')
    } else {
        None
    };
    if let Some(inner) = rgb_inner {
        let normalized = inner.replace([',', '/'], " ");
        let comps: Vec<&str> = normalized.split_whitespace().collect();
        if comps.len() != 3 && comps.len() != 4 {
            return None;
        }
        let parse_chan = |tok: &str, alpha: bool| -> Option<u8> {
            let v = if let Some(pct) = tok.strip_suffix('%') {
                parse_f32(pct)? * 2.55
            } else {
                let raw = parse_f32(tok)?;
                if alpha && raw <= 1.0 { raw * 255.0 } else { raw }
            };
            v.is_finite().then(|| v.round().clamp(0.0, 255.0) as u8)
        };
        let r = parse_chan(comps[0], false)?;
        let g = parse_chan(comps[1], false)?;
        let b = parse_chan(comps[2], false)?;
        let a = if comps.len() == 4 { parse_chan(comps[3], true)? } else { 255 };
        return Some(ColorRgba::rgba(r, g, b, a));
    }

    const NAMED: &[(&str, ColorRgba)] = &[
        ("white", ColorRgba::WHITE),
        ("black", ColorRgba::BLACK),
        ("transparent", ColorRgba::TRANSPARENT),
        ("red", ColorRgba::rgb(239, 68, 68)),
        ("green", ColorRgba::rgb(34, 197, 94)),
        ("blue", ColorRgba::rgb(59, 130, 246)),
    ];
    NAMED.iter().find_map(|&(n, c)| s.eq_ignore_ascii_case(n).then_some(c))
}

/// Parses a CSS font-weight keyword or numeric value (`100..=900`), returning `None` if invalid.
pub fn try_parse_font_weight(raw: &str) -> Option<u16> {
    const WEIGHTS: &[(&str, u16)] = &[
        ("thin", 100), ("light", 300), ("normal", 400), ("regular", 400),
        ("medium", 500), ("semibold", 600), ("bold", 700), ("extrabold", 800), ("black", 900),
    ];
    let s = raw.trim();
    WEIGHTS.iter().find_map(|&(n, w)| s.eq_ignore_ascii_case(n).then_some(w))
        .or_else(|| s.parse::<u16>().ok().map(|w| w.clamp(100, 900)))
}

/// Parses a CSS font-weight keyword or numeric value (`100..=900`), defaulting to `400`.
pub fn parse_font_weight(raw: &str) -> u16 {
    try_parse_font_weight(raw).unwrap_or(400)
}

/// Maps a canonical CSS property bitmask to its [`STYLE_LAYER_*`] bitmask.
#[inline]
pub const fn prop_bits_to_layers(bits: u64) -> u8 {
    let mut layers = 0u8;
    if bits & LAYOUT_PROPS_MASK != 0 {
        layers |= STYLE_LAYER_LAYOUT;
    }
    if bits & VISUAL_PROPS_MASK != 0 {
        layers |= STYLE_LAYER_VISUAL;
    }
    if bits & TYPOGRAPHY_PROPS_MASK != 0 {
        layers |= STYLE_LAYER_TYPOGRAPHY;
    }
    layers
}

/// Three-state outcome of applying a CSS/style attribute to a [`Transaction`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssAttrResult {
    /// Attribute name is not a recognized CSS style property.
    Unknown,
    /// Recognized CSS property whose value failed validation (carries affected [`STYLE_LAYER_*`]).
    Invalid(u8),
    /// Recognized CSS property whose value parsed validly (carries affected [`STYLE_LAYER_*`]).
    Applied(u8),
}

/// Single-source mapping from CSS/RN property name to its canonical property bitmask.
#[inline]
pub fn css_key_bits(key: &str) -> Option<u64> {
    Some(match key {
        "width" => BIT_WIDTH,
        "height" => BIT_HEIGHT,
        "min-width" | "minWidth" => BIT_MIN_WIDTH,
        "min-height" | "minHeight" => BIT_MIN_HEIGHT,
        "max-width" | "maxWidth" => BIT_MAX_WIDTH,
        "max-height" | "maxHeight" => BIT_MAX_HEIGHT,
        "display" => BIT_DISPLAY,
        "flex-direction" | "flexDirection" => BIT_FLEX_DIRECTION,
        "flex-wrap" | "flexWrap" => BIT_FLEX_WRAP,
        "justify-content" | "justifyContent" => BIT_JUSTIFY_CONTENT,
        "align-items" | "alignItems" => BIT_ALIGN_ITEMS,
        "flex" | "flex-grow" | "flexGrow" => BIT_FLEX_GROW,
        "flex-shrink" | "flexShrink" => BIT_FLEX_SHRINK,
        "flex-basis" | "flexBasis" => BIT_FLEX_BASIS,
        "gap" => BIT_ROW_GAP | BIT_COLUMN_GAP,
        "row-gap" | "rowGap" => BIT_ROW_GAP,
        "column-gap" | "columnGap" => BIT_COLUMN_GAP,
        "padding" => BIT_PADDING_H | BIT_PADDING_V,
        "padding-horizontal" | "paddingHorizontal" => BIT_PADDING_H,
        "padding-vertical" | "paddingVertical" => BIT_PADDING_V,
        "padding-top" | "paddingTop" | "padding-bottom" | "paddingBottom" => BIT_PADDING_V,
        "padding-left" | "paddingLeft" | "padding-right" | "paddingRight" => BIT_PADDING_H,
        "margin" => BIT_MARGIN,
        "background" | "background-color" | "backgroundColor" => BIT_BACKGROUND,
        "border-color" | "borderColor" => BIT_BORDER_COLOR,
        "border-width" | "borderWidth" => BIT_BORDER_WIDTH,
        "border-radius" | "borderRadius" => BIT_BORDER_RADIUS,
        "opacity" => BIT_OPACITY,
        "visibility" => BIT_VISIBILITY,
        "overflow" => BIT_OVERFLOW_X | BIT_OVERFLOW_Y,
        "overflow-x" | "overflowX" => BIT_OVERFLOW_X,
        "overflow-y" | "overflowY" => BIT_OVERFLOW_Y,
        "box-shadow" | "boxShadow" => BIT_BOX_SHADOW,
        "z-index" | "zIndex" => BIT_Z_INDEX,
        "color" => BIT_COLOR,
        "overflow-wrap" | "overflowWrap" | "word-wrap" => BIT_OVERFLOW_WRAP,
        "word-break" | "wordBreak" => BIT_WORD_BREAK,
        "white-space" | "whiteSpace" => BIT_WHITE_SPACE,
        "line-height" | "lineHeight" => BIT_LINE_HEIGHT,
        "font-size" | "fontSize" => BIT_FONT_SIZE,
        "font-weight" | "fontWeight" => BIT_FONT_WEIGHT,
        "text-align" | "textAlign" => BIT_TEXT_ALIGN,
        "font-family" | "fontFamily" => BIT_FONT_FAMILY,
        "font-style" | "fontStyle" => BIT_FONT_STYLE,
        "text-decoration" | "textDecoration" | "text-decoration-line" => BIT_TEXT_DECORATION,
        _ => return None,
    })
}

/// Specificity rank keeping CSS shorthands (`0`) ahead of longhands (`1`) independent of
/// insertion/clear history.
#[inline]
pub fn css_key_rank(key: &str) -> u8 {
    match key {
        "padding" | "gap" | "overflow" | "flex" | "background" => 0,
        _ => 1,
    }
}

/// Returns the [`STYLE_LAYER_*`] bitmask for a standalone CSS/style attribute name (`0` if unknown).
#[inline]
pub fn css_attr_layer(key: &str) -> u8 {
    css_key_bits(key).map_or(0, prop_bits_to_layers)
}

/// Applies an inline `style="..."` string in a single pass, writing declarations in `layers`
/// and returning the canonical property bitmask of **all** valid declarations present in `style`.
pub fn apply_inline_style_layers_to_tx(
    tx: &mut Transaction<'_>,
    id: NodeId,
    style: &str,
    layers: u8,
) -> u64 {
    let mut accepted_mask = 0u64;
    for decl in style.split(';') {
        if let Some((rk, rv)) = decl.split_once(':') {
            let k = rk.trim();
            let v = rv.trim();
            if !k.is_empty()
                && !v.is_empty()
                && let Some((bits, Some(parsed))) = parse_css_decl(k, v)
            {
                accepted_mask |= bits;
                if prop_bits_to_layers(bits) & layers != 0 {
                    write_parsed_decl(tx, id, parsed);
                }
            }
        }
    }
    accepted_mask
}

/// Applies one CSS/RN inline style declaration or attribute directly into `NodeStore` word tables.
/// Returns `true` if `key` is recognized as a CSS style attribute.
pub fn apply_css_attr_to_tx(tx: &mut Transaction<'_>, id: NodeId, key: &str, value: &str) -> bool {
    apply_css_attr_layers_to_tx(tx, id, key, value, STYLE_LAYER_ALL)
}

/// Like [`apply_css_attr_to_tx`], but writes only declarations belonging to `layers`.
pub fn apply_css_attr_layers_to_tx(
    tx: &mut Transaction<'_>,
    id: NodeId,
    key: &str,
    value: &str,
    layers: u8,
) -> bool {
    !matches!(
        apply_css_attr_layers_status_to_tx(tx, id, key, value, layers),
        CssAttrResult::Unknown
    )
}

/// Like [`apply_css_attr_layers_to_tx`], returning the 3-state [`CssAttrResult`] so callers can
/// distinguish unknown attributes from recognized CSS properties with invalid values.
pub fn apply_css_attr_layers_status_to_tx(
    tx: &mut Transaction<'_>,
    id: NodeId,
    key: &str,
    value: &str,
    layers: u8,
) -> CssAttrResult {
    if key == "style" {
        let mask = apply_inline_style_layers_to_tx(tx, id, value, layers);
        return CssAttrResult::Applied(prop_bits_to_layers(mask));
    }
    let Some((bits, maybe_decl)) = parse_css_decl(key, value) else {
        return CssAttrResult::Unknown;
    };
    let layer = prop_bits_to_layers(bits);
    match maybe_decl {
        Some(parsed) => {
            if layer & layers != 0 {
                write_parsed_decl(tx, id, parsed);
            }
            CssAttrResult::Applied(layer)
        }
        None => CssAttrResult::Invalid(layer),
    }
}
