//! One CSS declaration: `ParsedDecl` value parsing (`parse_css_decl`) and its word-table write
//! (`write_parsed_decl`), split out of `css.rs` so the property table stays the single entry point.
//!
//! Values are sealed at ingress: every scalar is finite and within `MAX_STYLE_SCALAR`, box sizes,
//! padding, gaps, flex factors, border widths and radii are non-negative, `font-size` is positive,
//! and keywords match ASCII case-insensitively. An invalid value parses to `None` (all-or-nothing).

use nve_abi::num::{is_valid_style_scalar, parse_f32};
use nve_store::{Length, NodeId, Rgba, Transaction};

use super::css_key_bits;
use crate::css_shorthand::{
    parse_border, parse_border_color, parse_border_width_value, parse_edge, parse_edges, parse_flex,
    parse_flex_basis, BorderColor, FlexShorthand,
};
use crate::css_values::{
    dim_to_length, keyword, parse_color, parse_px, try_parse_dim, try_parse_font_weight,
};
use crate::tables::{
    self, text_decoration, AlignItems, BoxShadows, Display, FlexDirection, FlexWrap, FontStyle,
    JustifyContent, Layout, Overflow, OverflowWrap, TextAlign, Typography, Visibility, Visual,
    WhiteSpace, WordBreak,
};

pub(super) enum ParsedDecl<'a> {
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
    Flex(FlexShorthand),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Length),
    Gap(Length),
    RowGap(Length),
    ColumnGap(Length),
    /// `[top, right, bottom, left]`.
    Padding([f32; 4]),
    PaddingH(f32),
    PaddingV(f32),
    PaddingTop(f32),
    PaddingBottom(f32),
    PaddingLeft(f32),
    PaddingRight(f32),
    /// `[top, right, bottom, left]`.
    Margin([f32; 4]),
    MarginH(f32),
    MarginV(f32),
    MarginTop(f32),
    MarginBottom(f32),
    MarginLeft(f32),
    MarginRight(f32),
    Background(Rgba),
    Border(f32, BorderColor),
    BorderColor(BorderColor),
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
    FontFamily(&'a str),
    FontStyle(FontStyle),
    TextDecoration(u8),
}

impl ParsedDecl<'_> {
    /// `Some(true)` when this declaration sets the border color to `currentColor`, `Some(false)`
    /// for a concrete border color, `None` when it does not touch the border color.
    #[inline]
    pub(super) const fn border_color_source(&self) -> Option<bool> {
        match self {
            Self::Border(_, color) | Self::BorderColor(color) => Some(color.is_none()),
            _ => None,
        }
    }
}

const DISPLAY: &[(&str, Display)] = &[
    ("flex", Display::Flex),
    ("block", Display::Block),
    ("inline", Display::Inline),
    ("inline-block", Display::InlineBlock),
    ("inline-flex", Display::InlineFlex),
    ("contents", Display::Contents),
    ("grid", Display::Grid),
    ("none", Display::None),
];
const FLEX_DIRECTION: &[(&str, FlexDirection)] = &[
    ("column", FlexDirection::Column),
    ("row", FlexDirection::Row),
    ("column-reverse", FlexDirection::ColumnReverse),
    ("columnReverse", FlexDirection::ColumnReverse),
    ("row-reverse", FlexDirection::RowReverse),
    ("rowReverse", FlexDirection::RowReverse),
];
const FLEX_WRAP: &[(&str, FlexWrap)] = &[
    ("nowrap", FlexWrap::NoWrap),
    ("wrap", FlexWrap::Wrap),
    ("wrap-reverse", FlexWrap::WrapReverse),
    ("wrapReverse", FlexWrap::WrapReverse),
];
const JUSTIFY_CONTENT: &[(&str, JustifyContent)] = &[
    ("flex-start", JustifyContent::FlexStart),
    ("start", JustifyContent::FlexStart),
    ("flexStart", JustifyContent::FlexStart),
    ("center", JustifyContent::Center),
    ("flex-end", JustifyContent::FlexEnd),
    ("end", JustifyContent::FlexEnd),
    ("flexEnd", JustifyContent::FlexEnd),
    ("space-between", JustifyContent::SpaceBetween),
    ("spaceBetween", JustifyContent::SpaceBetween),
    ("space-around", JustifyContent::SpaceAround),
    ("spaceAround", JustifyContent::SpaceAround),
    ("space-evenly", JustifyContent::SpaceEvenly),
    ("spaceEvenly", JustifyContent::SpaceEvenly),
];
const ALIGN_ITEMS: &[(&str, AlignItems)] = &[
    ("stretch", AlignItems::Stretch),
    ("flex-start", AlignItems::FlexStart),
    ("start", AlignItems::FlexStart),
    ("flexStart", AlignItems::FlexStart),
    ("center", AlignItems::Center),
    ("flex-end", AlignItems::FlexEnd),
    ("end", AlignItems::FlexEnd),
    ("flexEnd", AlignItems::FlexEnd),
    ("baseline", AlignItems::Baseline),
];
const OVERFLOW: &[(&str, Overflow)] = &[
    ("visible", Overflow::Visible),
    ("hidden", Overflow::Hidden),
    ("scroll", Overflow::Scroll),
    ("auto", Overflow::Auto),
];
const VISIBILITY: &[(&str, Visibility)] = &[
    ("hidden", Visibility::Hidden),
    ("collapse", Visibility::Hidden),
    ("visible", Visibility::Visible),
    ("initial", Visibility::Visible),
    ("inherit", Visibility::Inherit),
    ("unset", Visibility::Inherit),
    ("revert", Visibility::Inherit),
    ("revert-layer", Visibility::Inherit),
];
const FONT_STYLE: &[(&str, FontStyle)] = &[
    ("normal", FontStyle::Normal),
    ("italic", FontStyle::Italic),
    ("oblique", FontStyle::Oblique),
];
const OVERFLOW_WRAP: &[(&str, OverflowWrap)] = &[
    ("normal", OverflowWrap::Normal),
    ("break-word", OverflowWrap::BreakWord),
    ("anywhere", OverflowWrap::Anywhere),
];
const WORD_BREAK: &[(&str, WordBreak)] = &[
    ("normal", WordBreak::Normal),
    ("break-all", WordBreak::BreakAll),
    ("keep-all", WordBreak::KeepAll),
    ("break-word", WordBreak::BreakWord),
];
const WHITE_SPACE: &[(&str, WhiteSpace)] = &[
    ("normal", WhiteSpace::Normal),
    ("nowrap", WhiteSpace::NoWrap),
    ("pre", WhiteSpace::Pre),
    ("pre-wrap", WhiteSpace::PreWrap),
    ("pre-line", WhiteSpace::PreLine),
];
const TEXT_ALIGN: &[(&str, TextAlign)] = &[
    ("left", TextAlign::Left),
    ("start", TextAlign::Left),
    ("center", TextAlign::Center),
    ("right", TextAlign::Right),
    ("end", TextAlign::Right),
    ("justify", TextAlign::Justify),
];

/// Non-negative `px` scalar (gaps, padding, border radius).
#[inline]
fn non_negative_px(v: &str) -> Option<f32> {
    parse_px(v).filter(|&px| px >= 0.0)
}

/// Non-negative flex factor within `MAX_STYLE_SCALAR`.
#[inline]
fn flex_factor(v: &str) -> Option<f32> {
    parse_f32(v).filter(|&f| is_valid_style_scalar(f) && f >= 0.0)
}

/// `line-height`: positive `px`, positive percentage, or a positive unitless multiplier.
fn parse_line_height(value: &str) -> Option<Length> {
    let v = value.trim();
    let positive = |x: f32| is_valid_style_scalar(x) && x > 0.0;
    if let Some(pct) = v.strip_suffix('%') {
        parse_f32(pct.trim()).filter(|&p| positive(p)).map(Length::percent)
    } else if let Some(mult) = parse_f32(v) {
        positive(mult * 100.0).then(|| Length::percent(mult * 100.0))
    } else {
        parse_px(v).filter(|&px| positive(px)).map(Length::points)
    }
}

/// `none` or a space-separated set of `underline` / `overline` / `line-through` lines.
fn parse_text_decoration(value: &str) -> Option<u8> {
    const LINES: &[(&str, u8)] = &[
        ("underline", text_decoration::UNDERLINE),
        ("overline", text_decoration::OVERLINE),
        ("line-through", text_decoration::LINE_THROUGH),
    ];
    if value.eq_ignore_ascii_case("none") {
        return Some(0);
    }
    value
        .split_ascii_whitespace()
        .try_fold(0u8, |bits, kw| Some(bits | keyword(kw, LINES)?))
}

#[inline]
fn color(v: &str) -> Option<Rgba> {
    parse_color(v).map(|c| Rgba(c.to_u32()))
}

#[inline]
fn dim(v: &str) -> Option<Length> {
    try_parse_dim(v).map(dim_to_length)
}

/// Parses one CSS declaration into `(canonical_prop_bits, Option<ParsedDecl>)`.
/// Returns `None` iff `key` is unrecognized; returns `Some((bits, None))` when `key` is recognized
/// (preserving its layer bits) but `value` fails validation.
pub(super) fn parse_css_decl<'a>(key: &str, value: &'a str) -> Option<(u64, Option<ParsedDecl<'a>>)> {
    use ParsedDecl as D;
    let bits = css_key_bits(key)?;
    let v = value.trim();
    let parsed: Option<ParsedDecl> = match key {
        "width" => dim(v).map(D::Width),
        "height" => dim(v).map(D::Height),
        "min-width" | "minWidth" => dim(v).map(D::MinWidth),
        "min-height" | "minHeight" => dim(v).map(D::MinHeight),
        "max-width" | "maxWidth" => dim(v).map(D::MaxWidth),
        "max-height" | "maxHeight" => dim(v).map(D::MaxHeight),
        "display" => keyword(v, DISPLAY).map(D::Display),
        "flex-direction" | "flexDirection" => keyword(v, FLEX_DIRECTION).map(D::FlexDirection),
        "flex-wrap" | "flexWrap" => keyword(v, FLEX_WRAP).map(D::FlexWrap),
        "justify-content" | "justifyContent" => keyword(v, JUSTIFY_CONTENT).map(D::JustifyContent),
        "align-items" | "alignItems" => keyword(v, ALIGN_ITEMS).map(D::AlignItems),
        "flex" => parse_flex(v).map(D::Flex),
        "flex-grow" | "flexGrow" => flex_factor(v).map(D::FlexGrow),
        "flex-shrink" | "flexShrink" => flex_factor(v).map(D::FlexShrink),
        "flex-basis" | "flexBasis" => parse_flex_basis(v).map(D::FlexBasis),
        "gap" => non_negative_px(v).map(|px| D::Gap(Length::points(px))),
        "row-gap" | "rowGap" => non_negative_px(v).map(|px| D::RowGap(Length::points(px))),
        "column-gap" | "columnGap" => non_negative_px(v).map(|px| D::ColumnGap(Length::points(px))),
        "padding" => parse_edges(v, false).map(D::Padding),
        "padding-horizontal" | "paddingHorizontal" => parse_edge(v, false).map(D::PaddingH),
        "padding-vertical" | "paddingVertical" => parse_edge(v, false).map(D::PaddingV),
        "padding-top" | "paddingTop" => parse_edge(v, false).map(D::PaddingTop),
        "padding-bottom" | "paddingBottom" => parse_edge(v, false).map(D::PaddingBottom),
        "padding-left" | "paddingLeft" => parse_edge(v, false).map(D::PaddingLeft),
        "padding-right" | "paddingRight" => parse_edge(v, false).map(D::PaddingRight),
        "margin" => parse_edges(v, true).map(D::Margin),
        "margin-horizontal" | "marginHorizontal" => parse_edge(v, true).map(D::MarginH),
        "margin-vertical" | "marginVertical" => parse_edge(v, true).map(D::MarginV),
        "margin-top" | "marginTop" => parse_edge(v, true).map(D::MarginTop),
        "margin-bottom" | "marginBottom" => parse_edge(v, true).map(D::MarginBottom),
        "margin-left" | "marginLeft" => parse_edge(v, true).map(D::MarginLeft),
        "margin-right" | "marginRight" => parse_edge(v, true).map(D::MarginRight),
        "font-family" | "fontFamily" => (!v.is_empty()).then_some(D::FontFamily(v)),
        "font-style" | "fontStyle" => keyword(v, FONT_STYLE).map(D::FontStyle),
        "text-decoration" | "textDecoration" | "text-decoration-line" => {
            parse_text_decoration(v).map(D::TextDecoration)
        }
        "background" | "background-color" | "backgroundColor" => color(v).map(D::Background),
        "color" => color(v).map(D::Color),
        "border" => parse_border(v).map(|(w, c)| D::Border(w, c)),
        "border-color" | "borderColor" => parse_border_color(v).map(D::BorderColor),
        "border-width" | "borderWidth" => parse_border_width_value(v).map(D::BorderWidth),
        "border-radius" | "borderRadius" => non_negative_px(v).map(D::BorderRadius),
        "opacity" => parse_f32(v).map(|f| D::Opacity(f.clamp(0.0, 1.0))),
        "visibility" => keyword(v, VISIBILITY).map(D::Visibility),
        "overflow" => keyword(v, OVERFLOW).map(D::Overflow),
        "overflow-x" | "overflowX" => keyword(v, OVERFLOW).map(D::OverflowX),
        "overflow-y" | "overflowY" => keyword(v, OVERFLOW).map(D::OverflowY),
        "overflow-wrap" | "overflowWrap" | "word-wrap" => keyword(v, OVERFLOW_WRAP).map(D::OverflowWrap),
        "word-break" | "wordBreak" => keyword(v, WORD_BREAK).map(D::WordBreak),
        "white-space" | "whiteSpace" => keyword(v, WHITE_SPACE).map(D::WhiteSpace),
        "line-height" | "lineHeight" => parse_line_height(v).map(D::LineHeight),
        "box-shadow" | "boxShadow" => {
            if v.eq_ignore_ascii_case("none") {
                Some(D::BoxShadow(None))
            } else {
                let shadows = tables::parse_box_shadow(v);
                (!shadows.is_empty()).then(|| D::BoxShadow(Some(BoxShadows(shadows.into()))))
            }
        }
        "font-size" | "fontSize" => parse_px(v).filter(|&px| px > 0.0).map(D::FontSize),
        "font-weight" | "fontWeight" => try_parse_font_weight(v).map(D::FontWeight),
        "text-align" | "textAlign" => keyword(v, TEXT_ALIGN).map(D::TextAlign),
        "z-index" | "zIndex" => v.parse::<i32>().ok().map(D::ZIndex),
        _ => None,
    };
    Some((bits, parsed))
}

/// Sets `field` explicit in `Typography::set`, then writes the field.
macro_rules! typography {
    ($tx:expr, $id:expr, $bit:expr, $setter:ident($v:expr)) => {{
        let row = $tx.edit::<Typography>($id);
        let set = row.get().set() | $bit;
        row.$setter($v).set(set);
    }};
}

pub(super) fn write_parsed_decl(tx: &mut Transaction<'_>, id: NodeId, decl: ParsedDecl<'_>) {
    use ParsedDecl as D;
    let pt = Length::points;
    match decl {
        D::Width(l) => { tx.edit::<Layout>(id).width(l); }
        D::Height(l) => { tx.edit::<Layout>(id).height(l); }
        D::MinWidth(l) => { tx.edit::<Layout>(id).min_width(l); }
        D::MinHeight(l) => { tx.edit::<Layout>(id).min_height(l); }
        D::MaxWidth(l) => { tx.edit::<Layout>(id).max_width(l); }
        D::MaxHeight(l) => { tx.edit::<Layout>(id).max_height(l); }
        D::Display(d) => { tx.edit::<Layout>(id).display(d); }
        D::FlexDirection(fd) => { tx.edit::<Layout>(id).flex_direction(fd); }
        D::FlexWrap(fw) => { tx.edit::<Layout>(id).flex_wrap(fw); }
        D::JustifyContent(jc) => { tx.edit::<Layout>(id).justify_content(jc); }
        D::AlignItems(ai) => { tx.edit::<Layout>(id).align_items(ai); }
        D::Flex(f) => {
            tx.edit::<Layout>(id).flex_grow(f.grow).flex_shrink(f.shrink).flex_basis(f.basis);
        }
        D::FlexGrow(v) => { tx.edit::<Layout>(id).flex_grow(v); }
        D::FlexShrink(v) => { tx.edit::<Layout>(id).flex_shrink(v); }
        D::FlexBasis(l) => { tx.edit::<Layout>(id).flex_basis(l); }
        D::Gap(l) => { tx.edit::<Layout>(id).row_gap(l).column_gap(l); }
        D::RowGap(l) => { tx.edit::<Layout>(id).row_gap(l); }
        D::ColumnGap(l) => { tx.edit::<Layout>(id).column_gap(l); }
        D::Padding([t, r, b, l]) => {
            tx.edit::<Layout>(id).padding_top(pt(t)).padding_right(pt(r)).padding_bottom(pt(b)).padding_left(pt(l));
        }
        D::PaddingH(v) => { tx.edit::<Layout>(id).padding_left(pt(v)).padding_right(pt(v)); }
        D::PaddingV(v) => { tx.edit::<Layout>(id).padding_top(pt(v)).padding_bottom(pt(v)); }
        D::PaddingTop(v) => { tx.edit::<Layout>(id).padding_top(pt(v)); }
        D::PaddingBottom(v) => { tx.edit::<Layout>(id).padding_bottom(pt(v)); }
        D::PaddingLeft(v) => { tx.edit::<Layout>(id).padding_left(pt(v)); }
        D::PaddingRight(v) => { tx.edit::<Layout>(id).padding_right(pt(v)); }
        D::Margin([t, r, b, l]) => {
            tx.edit::<Layout>(id).margin_top(pt(t)).margin_right(pt(r)).margin_bottom(pt(b)).margin_left(pt(l));
        }
        D::MarginH(v) => { tx.edit::<Layout>(id).margin_left(pt(v)).margin_right(pt(v)); }
        D::MarginV(v) => { tx.edit::<Layout>(id).margin_top(pt(v)).margin_bottom(pt(v)); }
        D::MarginTop(v) => { tx.edit::<Layout>(id).margin_top(pt(v)); }
        D::MarginBottom(v) => { tx.edit::<Layout>(id).margin_bottom(pt(v)); }
        D::MarginLeft(v) => { tx.edit::<Layout>(id).margin_left(pt(v)); }
        D::MarginRight(v) => { tx.edit::<Layout>(id).margin_right(pt(v)); }
        D::Background(c) => { tx.edit::<Visual>(id).background(c); }
        // A symbolic `currentColor` leaves the color to `resolve_current_color_border`.
        D::Border(w, c) => {
            let row = tx.edit::<Visual>(id).border_width(w);
            if let Some(c) = c {
                row.border_color(c);
            }
        }
        D::BorderColor(c) => {
            if let Some(c) = c {
                tx.edit::<Visual>(id).border_color(c);
            }
        }
        D::BorderWidth(v) => { tx.edit::<Visual>(id).border_width(v); }
        D::BorderRadius(v) => { tx.edit::<Visual>(id).border_radius(v); }
        D::Opacity(v) => { tx.edit::<Visual>(id).opacity(v); }
        D::Visibility(vis) => { tx.edit::<Visual>(id).visibility(vis); }
        D::Overflow(ov) => { tx.edit::<Visual>(id).overflow_x(ov).overflow_y(ov); }
        D::OverflowX(ov) => { tx.edit::<Visual>(id).overflow_x(ov); }
        D::OverflowY(ov) => { tx.edit::<Visual>(id).overflow_y(ov); }
        D::BoxShadow(bs) => { tx.edit::<Visual>(id).box_shadows(bs); }
        D::ZIndex(z) => { tx.edit::<Visual>(id).z_index(z); }
        D::Color(c) => typography!(tx, id, Typography::SET_COLOR, color(c)),
        D::OverflowWrap(ow) => { tx.edit::<Typography>(id).overflow_wrap(ow); }
        D::WordBreak(wb) => { tx.edit::<Typography>(id).word_break(wb); }
        D::WhiteSpace(ws) => { tx.edit::<Typography>(id).white_space(ws); }
        D::LineHeight(lh) => typography!(tx, id, Typography::SET_LINE_HEIGHT, line_height(lh)),
        D::FontSize(v) => typography!(tx, id, Typography::SET_FONT_SIZE, font_size(v)),
        D::FontWeight(w) => typography!(tx, id, Typography::SET_FONT_WEIGHT, font_weight(w)),
        D::TextAlign(ta) => { tx.edit::<Typography>(id).text_align(ta); }
        D::FontFamily(family) => typography!(tx, id, Typography::SET_FONT_FAMILY, font_family(family)),
        D::FontStyle(style) => typography!(tx, id, Typography::SET_FONT_STYLE, font_style(style)),
        D::TextDecoration(bits) => {
            typography!(tx, id, Typography::SET_TEXT_DECORATION, text_decoration(bits));
        }
    }
}
