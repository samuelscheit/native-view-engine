//! Shared CSS/RN inline style parsers and bitmask-based style-layer diffing (`Layout`, `Visual`,
//! `Typography`) backed by a single-source property table.

use nve_abi::num::parse_f32;
use nve_abi::{ColorRgba, Dimension};
use nve_store::{Length, NodeId, Rgba, Transaction};

use crate::tables::{
    self, AlignItems, BoxShadows, Display, FlexDirection, FlexWrap, JustifyContent, Layout,
    Overflow, OverflowWrap, TextAlign, Typography, Visibility, Visual, WhiteSpace, WordBreak,
};

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
const LAYOUT_PROPS_MASK: u64 = (1 << 18) - 1;

const BIT_BACKGROUND: u64 = 1 << 18;
const BIT_BORDER_COLOR: u64 = 1 << 19;
const BIT_BORDER_WIDTH: u64 = 1 << 20;
const BIT_BORDER_RADIUS: u64 = 1 << 21;
const BIT_OPACITY: u64 = 1 << 22;
const BIT_VISIBILITY: u64 = 1 << 23;
const BIT_OVERFLOW_X: u64 = 1 << 24;
const BIT_OVERFLOW_Y: u64 = 1 << 25;
const BIT_BOX_SHADOW: u64 = 1 << 26;
const BIT_Z_INDEX: u64 = 1 << 27;
const VISUAL_PROPS_MASK: u64 = ((1 << 28) - 1) & !LAYOUT_PROPS_MASK;

const BIT_COLOR: u64 = 1 << 28;
const BIT_OVERFLOW_WRAP: u64 = 1 << 29;
const BIT_WORD_BREAK: u64 = 1 << 30;
const BIT_WHITE_SPACE: u64 = 1 << 31;
const BIT_LINE_HEIGHT: u64 = 1 << 32;
const BIT_FONT_SIZE: u64 = 1 << 33;
const BIT_FONT_WEIGHT: u64 = 1 << 34;
const BIT_TEXT_ALIGN: u64 = 1 << 35;
const TYPOGRAPHY_PROPS_MASK: u64 = ((1 << 36) - 1) & !(LAYOUT_PROPS_MASK | VISUAL_PROPS_MASK);

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

    let lower = s.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
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

    match lower.as_str() {
        "white" => Some(ColorRgba::WHITE),
        "black" => Some(ColorRgba::BLACK),
        "transparent" => Some(ColorRgba::TRANSPARENT),
        "red" => Some(ColorRgba::rgb(239, 68, 68)),
        "green" => Some(ColorRgba::rgb(34, 197, 94)),
        "blue" => Some(ColorRgba::rgb(59, 130, 246)),
        _ => None,
    }
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
        "gap" => BIT_ROW_GAP | BIT_COLUMN_GAP,
        "row-gap" | "rowGap" => BIT_ROW_GAP,
        "column-gap" | "columnGap" => BIT_COLUMN_GAP,
        "padding" => BIT_PADDING_H | BIT_PADDING_V,
        "padding-horizontal" | "paddingHorizontal" => BIT_PADDING_H,
        "padding-vertical" | "paddingVertical" => BIT_PADDING_V,
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

enum ParsedDecl {
    Width(Length),
    Height(Length),
    MinWidth(Length),
    MinHeight(Length),
    MaxWidth(Length),
    MaxHeight(Length),
    Display(Display),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    JustifyContent(JustifyContent),
    AlignItems(AlignItems),
    FlexGrow(f32),
    FlexShrink(f32),
    Gap(Length),
    RowGap(Length),
    ColumnGap(Length),
    Padding(Length),
    PaddingH(Length),
    PaddingV(Length),
    Margin(Length),
    Background(Rgba),
    BorderColor(Rgba),
    BorderWidth(f32),
    BorderRadius(f32),
    Opacity(f32),
    Visibility(Visibility),
    Overflow(Overflow),
    OverflowX(Overflow),
    OverflowY(Overflow),
    BoxShadow(Option<BoxShadows>),
    ZIndex(i32),
    Color(Rgba),
    OverflowWrap(OverflowWrap),
    WordBreak(WordBreak),
    WhiteSpace(WhiteSpace),
    LineHeight(Length),
    FontSize(f32),
    FontWeight(u16),
    TextAlign(TextAlign),
}

#[inline]
fn parse_overflow(value: &str) -> Option<Overflow> {
    match value.trim() {
        "visible" => Some(Overflow::Visible),
        "hidden" => Some(Overflow::Hidden),
        "scroll" => Some(Overflow::Scroll),
        "auto" => Some(Overflow::Auto),
        _ => None,
    }
}

#[inline]
fn parse_visibility(value: &str) -> Option<Visibility> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("hidden") || trimmed.eq_ignore_ascii_case("collapse") {
        Some(Visibility::Hidden)
    } else if trimmed.eq_ignore_ascii_case("visible") || trimmed.eq_ignore_ascii_case("initial") {
        Some(Visibility::Visible)
    } else if trimmed.eq_ignore_ascii_case("inherit")
        || trimmed.eq_ignore_ascii_case("unset")
        || trimmed.eq_ignore_ascii_case("revert")
        || trimmed.eq_ignore_ascii_case("revert-layer")
    {
        Some(Visibility::Inherit)
    } else {
        None
    }
}

#[inline]
fn parse_line_height(value: &str) -> Option<Length> {
    let trimmed = value.trim();
    if let Some(px) = trimmed
        .strip_suffix("px")
        .and_then(|s| s.trim().parse::<f32>().ok())
        && px > 0.0
    {
        Some(Length::points(px))
    } else if let Some(pct) = trimmed
        .strip_suffix('%')
        .and_then(|s| s.trim().parse::<f32>().ok())
        && pct > 0.0
    {
        Some(Length::percent(pct))
    } else if let Some(mult) = parse_f32(trimmed)
        && mult > 0.0
    {
        Some(Length::percent(mult * 100.0))
    } else {
        None
    }
}

/// Parses one CSS declaration into `(canonical_prop_bits, Option<ParsedDecl>)`.
/// Returns `None` iff `key` is unrecognized; returns `Some((bits, None))` when `key` is recognized
/// (preserving its layer bits) but `value` fails validation.
fn parse_css_decl(key: &str, value: &str) -> Option<(u64, Option<ParsedDecl>)> {
    let bits = css_key_bits(key)?;
    let v = value.trim();
    let parsed: Option<ParsedDecl> = match key {
        "width" => try_parse_dim(v).map(|d| ParsedDecl::Width(dim_to_length(d))),
        "height" => try_parse_dim(v).map(|d| ParsedDecl::Height(dim_to_length(d))),
        "min-width" | "minWidth" => try_parse_dim(v).map(|d| ParsedDecl::MinWidth(dim_to_length(d))),
        "min-height" | "minHeight" => try_parse_dim(v).map(|d| ParsedDecl::MinHeight(dim_to_length(d))),
        "max-width" | "maxWidth" => try_parse_dim(v).map(|d| ParsedDecl::MaxWidth(dim_to_length(d))),
        "max-height" | "maxHeight" => try_parse_dim(v).map(|d| ParsedDecl::MaxHeight(dim_to_length(d))),
        "display" => match v {
            "flex" => Some(Display::Flex),
            "block" => Some(Display::Block),
            "inline" => Some(Display::Inline),
            "inline-block" => Some(Display::InlineBlock),
            "inline-flex" => Some(Display::InlineFlex),
            "contents" => Some(Display::Contents),
            "grid" => Some(Display::Grid),
            "none" => Some(Display::None),
            _ => None,
        }
        .map(ParsedDecl::Display),
        "flex-direction" | "flexDirection" => match v {
            "column" => Some(FlexDirection::Column),
            "row" => Some(FlexDirection::Row),
            "column-reverse" | "columnReverse" => Some(FlexDirection::ColumnReverse),
            "row-reverse" | "rowReverse" => Some(FlexDirection::RowReverse),
            _ => None,
        }
        .map(ParsedDecl::FlexDirection),
        "flex-wrap" | "flexWrap" => match v {
            "nowrap" | "noWrap" => Some(FlexWrap::NoWrap),
            "wrap" => Some(FlexWrap::Wrap),
            "wrap-reverse" | "wrapReverse" => Some(FlexWrap::WrapReverse),
            _ => None,
        }
        .map(ParsedDecl::FlexWrap),
        "justify-content" | "justifyContent" => match v {
            "flex-start" | "start" | "flexStart" => Some(JustifyContent::FlexStart),
            "center" => Some(JustifyContent::Center),
            "flex-end" | "end" | "flexEnd" => Some(JustifyContent::FlexEnd),
            "space-between" | "spaceBetween" => Some(JustifyContent::SpaceBetween),
            "space-around" | "spaceAround" => Some(JustifyContent::SpaceAround),
            "space-evenly" | "spaceEvenly" => Some(JustifyContent::SpaceEvenly),
            _ => None,
        }
        .map(ParsedDecl::JustifyContent),
        "align-items" | "alignItems" => match v {
            "stretch" => Some(AlignItems::Stretch),
            "flex-start" | "start" | "flexStart" => Some(AlignItems::FlexStart),
            "center" => Some(AlignItems::Center),
            "flex-end" | "end" | "flexEnd" => Some(AlignItems::FlexEnd),
            "baseline" => Some(AlignItems::Baseline),
            _ => None,
        }
        .map(ParsedDecl::AlignItems),
        "flex" | "flex-grow" | "flexGrow" => parse_f32(v).map(ParsedDecl::FlexGrow),
        "flex-shrink" | "flexShrink" => parse_f32(v).map(ParsedDecl::FlexShrink),
        "gap" => parse_px(v).map(|px| ParsedDecl::Gap(Length::points(px))),
        "row-gap" | "rowGap" => parse_px(v).map(|px| ParsedDecl::RowGap(Length::points(px))),
        "column-gap" | "columnGap" => parse_px(v).map(|px| ParsedDecl::ColumnGap(Length::points(px))),
        "padding" => parse_px(v).map(|px| ParsedDecl::Padding(Length::points(px))),
        "padding-horizontal" | "paddingHorizontal" => parse_px(v).map(|px| ParsedDecl::PaddingH(Length::points(px))),
        "padding-vertical" | "paddingVertical" => parse_px(v).map(|px| ParsedDecl::PaddingV(Length::points(px))),
        "margin" => parse_px(v).map(|px| ParsedDecl::Margin(Length::points(px))),
        "background" | "background-color" | "backgroundColor" => parse_color(v).map(|c| ParsedDecl::Background(Rgba(c.to_u32()))),
        "color" => parse_color(v).map(|c| ParsedDecl::Color(Rgba(c.to_u32()))),
        "border-color" | "borderColor" => parse_color(v).map(|c| ParsedDecl::BorderColor(Rgba(c.to_u32()))),
        "border-width" | "borderWidth" => parse_px(v).map(ParsedDecl::BorderWidth),
        "border-radius" | "borderRadius" => parse_px(v).map(ParsedDecl::BorderRadius),
        "opacity" => parse_f32(v).map(|f| ParsedDecl::Opacity(f.clamp(0.0, 1.0))),
        "visibility" => parse_visibility(v).map(ParsedDecl::Visibility),
        "overflow" => parse_overflow(v).map(ParsedDecl::Overflow),
        "overflow-x" | "overflowX" => parse_overflow(v).map(ParsedDecl::OverflowX),
        "overflow-y" | "overflowY" => parse_overflow(v).map(ParsedDecl::OverflowY),
        "overflow-wrap" | "overflowWrap" | "word-wrap" => match v {
            "normal" => Some(OverflowWrap::Normal),
            "break-word" => Some(OverflowWrap::BreakWord),
            "anywhere" => Some(OverflowWrap::Anywhere),
            _ => None,
        }
        .map(ParsedDecl::OverflowWrap),
        "word-break" | "wordBreak" => match v {
            "normal" => Some(WordBreak::Normal),
            "break-all" => Some(WordBreak::BreakAll),
            "keep-all" => Some(WordBreak::KeepAll),
            "break-word" => Some(WordBreak::BreakWord),
            _ => None,
        }
        .map(ParsedDecl::WordBreak),
        "white-space" | "whiteSpace" => match v {
            "normal" => Some(WhiteSpace::Normal),
            "nowrap" => Some(WhiteSpace::NoWrap),
            "pre" => Some(WhiteSpace::Pre),
            "pre-wrap" => Some(WhiteSpace::PreWrap),
            "pre-line" => Some(WhiteSpace::PreLine),
            _ => None,
        }
        .map(ParsedDecl::WhiteSpace),
        "line-height" | "lineHeight" => parse_line_height(v).map(ParsedDecl::LineHeight),
        "box-shadow" | "boxShadow" => {
            if v.eq_ignore_ascii_case("none") {
                Some(ParsedDecl::BoxShadow(None))
            } else {
                let shadows = tables::parse_box_shadow(v);
                (!shadows.is_empty()).then(|| ParsedDecl::BoxShadow(Some(BoxShadows(shadows.into()))))
            }
        }
        "font-size" | "fontSize" => parse_px(v).map(ParsedDecl::FontSize),
        "font-weight" | "fontWeight" => match v.to_ascii_lowercase().as_str() {
            "thin" => Some(100),
            "light" => Some(300),
            "normal" | "regular" => Some(400),
            "medium" => Some(500),
            "semibold" => Some(600),
            "bold" => Some(700),
            "extrabold" => Some(800),
            "black" => Some(900),
            other => other.parse::<u16>().ok().map(|w| w.clamp(100, 900)),
        }
        .map(ParsedDecl::FontWeight),
        "text-align" | "textAlign" => match v {
            "left" | "start" => Some(TextAlign::Left),
            "center" => Some(TextAlign::Center),
            "right" | "end" => Some(TextAlign::Right),
            "justify" => Some(TextAlign::Justify),
            _ => None,
        }
        .map(ParsedDecl::TextAlign),
        "z-index" | "zIndex" => v.parse::<i32>().ok().map(ParsedDecl::ZIndex),
        _ => None,
    };
    Some((bits, parsed))
}

fn write_parsed_decl(tx: &mut Transaction<'_>, id: NodeId, decl: ParsedDecl) {
    match decl {
        ParsedDecl::Width(l) => { tx.edit::<Layout>(id).width(l); }
        ParsedDecl::Height(l) => { tx.edit::<Layout>(id).height(l); }
        ParsedDecl::MinWidth(l) => { tx.edit::<Layout>(id).min_width(l); }
        ParsedDecl::MinHeight(l) => { tx.edit::<Layout>(id).min_height(l); }
        ParsedDecl::MaxWidth(l) => { tx.edit::<Layout>(id).max_width(l); }
        ParsedDecl::MaxHeight(l) => { tx.edit::<Layout>(id).max_height(l); }
        ParsedDecl::Display(d) => { tx.edit::<Layout>(id).display(d); }
        ParsedDecl::FlexDirection(fd) => { tx.edit::<Layout>(id).flex_direction(fd); }
        ParsedDecl::FlexWrap(fw) => { tx.edit::<Layout>(id).flex_wrap(fw); }
        ParsedDecl::JustifyContent(jc) => { tx.edit::<Layout>(id).justify_content(jc); }
        ParsedDecl::AlignItems(ai) => { tx.edit::<Layout>(id).align_items(ai); }
        ParsedDecl::FlexGrow(v) => { tx.edit::<Layout>(id).flex_grow(v); }
        ParsedDecl::FlexShrink(v) => { tx.edit::<Layout>(id).flex_shrink(v); }
        ParsedDecl::Gap(l) => { tx.edit::<Layout>(id).row_gap(l).column_gap(l); }
        ParsedDecl::RowGap(l) => { tx.edit::<Layout>(id).row_gap(l); }
        ParsedDecl::ColumnGap(l) => { tx.edit::<Layout>(id).column_gap(l); }
        ParsedDecl::Padding(l) => {
            tx.edit::<Layout>(id)
                .padding_top(l)
                .padding_right(l)
                .padding_bottom(l)
                .padding_left(l);
        }
        ParsedDecl::PaddingH(l) => { tx.edit::<Layout>(id).padding_left(l).padding_right(l); }
        ParsedDecl::PaddingV(l) => { tx.edit::<Layout>(id).padding_top(l).padding_bottom(l); }
        ParsedDecl::Margin(l) => {
            tx.edit::<Layout>(id)
                .margin_top(l)
                .margin_right(l)
                .margin_bottom(l)
                .margin_left(l);
        }
        ParsedDecl::Background(c) => { tx.edit::<Visual>(id).background(c); }
        ParsedDecl::BorderColor(c) => { tx.edit::<Visual>(id).border_color(c); }
        ParsedDecl::BorderWidth(v) => { tx.edit::<Visual>(id).border_width(v); }
        ParsedDecl::BorderRadius(v) => { tx.edit::<Visual>(id).border_radius(v); }
        ParsedDecl::Opacity(v) => { tx.edit::<Visual>(id).opacity(v); }
        ParsedDecl::Visibility(vis) => { tx.edit::<Visual>(id).visibility(vis); }
        ParsedDecl::Overflow(ov) => { tx.edit::<Visual>(id).overflow_x(ov).overflow_y(ov); }
        ParsedDecl::OverflowX(ov) => { tx.edit::<Visual>(id).overflow_x(ov); }
        ParsedDecl::OverflowY(ov) => { tx.edit::<Visual>(id).overflow_y(ov); }
        ParsedDecl::BoxShadow(bs) => { tx.edit::<Visual>(id).box_shadows(bs); }
        ParsedDecl::ZIndex(z) => { tx.edit::<Visual>(id).z_index(z); }
        ParsedDecl::Color(c) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_COLOR;
            row.color(c).set(set);
        }
        ParsedDecl::OverflowWrap(ow) => { tx.edit::<Typography>(id).overflow_wrap(ow); }
        ParsedDecl::WordBreak(wb) => { tx.edit::<Typography>(id).word_break(wb); }
        ParsedDecl::WhiteSpace(ws) => { tx.edit::<Typography>(id).white_space(ws); }
        ParsedDecl::LineHeight(lh) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_LINE_HEIGHT;
            row.line_height(lh).set(set);
        }
        ParsedDecl::FontSize(v) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_FONT_SIZE;
            row.font_size(v).set(set);
        }
        ParsedDecl::FontWeight(w) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_FONT_WEIGHT;
            row.font_weight(w).set(set);
        }
        ParsedDecl::TextAlign(ta) => { tx.edit::<Typography>(id).text_align(ta); }
    }
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
