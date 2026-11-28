//! Shared CSS/RN inline style parsers and bitmask-based style-layer diffing (`Layout`, `Visual`,
//! `Typography`) backed by a single-source property table.

use nve_store::{NodeId, NodeStore, Rgba, Transaction};

use crate::tables::{Typography, Visual};

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

/// Value-level flag in the mask returned by [`apply_inline_style_layers_to_tx`] (outside every
/// property bit): the inline style's final border color is the symbolic `currentColor`.
pub const CSS_FLAG_BORDER_CURRENT_COLOR: u64 = 1 << 63;

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
        "flex" => BIT_FLEX_GROW | BIT_FLEX_SHRINK | BIT_FLEX_BASIS,
        "flex-grow" | "flexGrow" => BIT_FLEX_GROW,
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
        "margin" | "margin-top" | "marginTop" | "margin-right" | "marginRight"
        | "margin-bottom" | "marginBottom" | "margin-left" | "marginLeft"
        | "margin-horizontal" | "marginHorizontal" | "margin-vertical" | "marginVertical" => {
            BIT_MARGIN
        }
        "background" | "background-color" | "backgroundColor" => BIT_BACKGROUND,
        "border" => BIT_BORDER_WIDTH | BIT_BORDER_COLOR,
        "border-color" | "borderColor" => BIT_BORDER_COLOR,
        "border-width" | "borderWidth" => BIT_BORDER_WIDTH,
        "border-radius" | "borderRadius" => BIT_BORDER_RADIUS,
        "opacity" => BIT_OPACITY,
        "visibility" => BIT_VISIBILITY,
        "overflow" => BIT_OVERFLOW_X | BIT_OVERFLOW_Y,
        "overflow-x" | "overflowX" => BIT_OVERFLOW_X,
        "overflow-y" | "overflowY" => BIT_OVERFLOW_Y,
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

/// Precedence tier of a standalone attribute, independent of insertion/clear history: shorthands
/// (`0`) < axis shorthands (`1`, e.g. `padding-horizontal`) < longhands (`2`), so `padding` +
/// `padding-horizontal` + `padding-left` compose the same in any attribute order.
#[inline]
pub fn css_key_rank(key: &str) -> u8 {
    match key {
        "padding" | "margin" | "gap" | "overflow" | "flex" | "background" | "border" => 0,
        "padding-horizontal" | "paddingHorizontal" | "padding-vertical" | "paddingVertical"
        | "margin-horizontal" | "marginHorizontal" | "margin-vertical" | "marginVertical" => 1,
        _ => 2,
    }
}

/// Whether a declaration of `key: value` sets the border color, and to what: `Some(true)` for the
/// symbolic `currentColor` (explicit, or the omitted color of `border`), `Some(false)` for a
/// concrete color, `None` when it does not set a border color (other key, or an invalid value).
#[inline]
pub fn css_border_color_source(key: &str, value: &str) -> Option<bool> {
    parse_css_decl(key, value).and_then(|(_, d)| d?.border_color_source())
}

/// [`css_border_color_source`] of a whole inline style from its [`apply_inline_style_layers_to_tx`]
/// mask (`None` when it declares no valid border color).
#[inline]
pub const fn mask_border_color_source(mask: u64) -> Option<bool> {
    if mask & BIT_BORDER_COLOR == 0 {
        None
    } else {
        Some(mask & CSS_FLAG_BORDER_CURRENT_COLOR != 0)
    }
}

/// Explicit text color of one `Typography` row (`None` when inherited).
#[inline]
fn explicit_color(t: crate::tables::TypographyRef<'_>) -> Option<Rgba> {
    t.is_color_explicit().then(|| t.color())
}

/// The CSS `currentColor` of `id`: its nearest explicit `Typography` color, self included, or the
/// default black (one O(depth) walk, zero allocations).
pub fn effective_text_color(store: &NodeStore, id: NodeId) -> Rgba {
    let mut cur = Some(id);
    while let Some(nid) = cur.filter(|&n| store.is_live(n)) {
        if let Some(c) = store.try_row::<Typography>(nid).and_then(explicit_color) {
            return c;
        }
        cur = store.parent(nid);
    }
    Rgba::BLACK
}

/// Resolves `id`'s symbolic `currentColor` border to its current [`effective_text_color`],
/// writing only on change.
pub fn resolve_current_color_border(tx: &mut Transaction<'_>, id: NodeId) {
    let color = effective_text_color(tx.store(), id);
    if tx.store().try_row::<Visual>(id).is_none_or(|v| v.border_color() != color) {
        tx.edit::<Visual>(id).border_color(color);
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
    let mut border_current = None;
    for decl in style.split(';') {
        if let Some((rk, rv)) = decl.split_once(':') {
            let k = rk.trim();
            let v = rv.trim();
            if !k.is_empty()
                && !v.is_empty()
                && let Some((bits, Some(parsed))) = parse_css_decl(k, v)
            {
                accepted_mask |= bits;
                border_current = parsed.border_color_source().or(border_current);
                if prop_bits_to_layers(bits) & layers != 0 {
                    write_parsed_decl(tx, id, parsed);
                }
            }
        }
    }
    // `currentColor` resolves after the whole list, so `border: 1px solid; color: red` and
    // `color: red; border: 1px solid` agree.
    if border_current == Some(true) {
        accepted_mask |= CSS_FLAG_BORDER_CURRENT_COLOR;
        if layers & STYLE_LAYER_VISUAL != 0 {
            resolve_current_color_border(tx, id);
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
                let current = parsed.border_color_source() == Some(true);
                write_parsed_decl(tx, id, parsed);
                if current {
                    resolve_current_color_border(tx, id);
                }
            }
            CssAttrResult::Applied(layer)
        }
        None => CssAttrResult::Invalid(layer),
    }
}
