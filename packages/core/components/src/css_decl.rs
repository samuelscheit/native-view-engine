//! One CSS declaration: `ParsedDecl` value parsing (`parse_css_decl`) and its word-table write
//! (`write_parsed_decl`), split out of `css.rs` so the property table stays the single entry point.

use nve_abi::num::parse_f32;
use nve_store::{Length, NodeId, Rgba, Transaction};

use super::{css_key_bits, dim_to_length, parse_color, parse_px, try_parse_dim, try_parse_font_weight};
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
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Length),
    Gap(Length),
    RowGap(Length),
    ColumnGap(Length),
    Padding(Length),
    PaddingH(Length),
    PaddingV(Length),
    PaddingTop(Length),
    PaddingBottom(Length),
    PaddingLeft(Length),
    PaddingRight(Length),
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
    FontFamily(&'a str),
    FontStyle(FontStyle),
    TextDecoration(u8),
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
pub(super) fn parse_css_decl<'a>(key: &str, value: &'a str) -> Option<(u64, Option<ParsedDecl<'a>>)> {
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
        "flex-basis" | "flexBasis" => try_parse_dim(v).map(|d| ParsedDecl::FlexBasis(dim_to_length(d))),
        "gap" => parse_px(v).map(|px| ParsedDecl::Gap(Length::points(px))),
        "row-gap" | "rowGap" => parse_px(v).map(|px| ParsedDecl::RowGap(Length::points(px))),
        "column-gap" | "columnGap" => parse_px(v).map(|px| ParsedDecl::ColumnGap(Length::points(px))),
        "padding" => parse_px(v).map(|px| ParsedDecl::Padding(Length::points(px))),
        "padding-horizontal" | "paddingHorizontal" => parse_px(v).map(|px| ParsedDecl::PaddingH(Length::points(px))),
        "padding-vertical" | "paddingVertical" => parse_px(v).map(|px| ParsedDecl::PaddingV(Length::points(px))),
        "padding-top" | "paddingTop" => parse_px(v).map(|px| ParsedDecl::PaddingTop(Length::points(px))),
        "padding-bottom" | "paddingBottom" => parse_px(v).map(|px| ParsedDecl::PaddingBottom(Length::points(px))),
        "padding-left" | "paddingLeft" => parse_px(v).map(|px| ParsedDecl::PaddingLeft(Length::points(px))),
        "padding-right" | "paddingRight" => parse_px(v).map(|px| ParsedDecl::PaddingRight(Length::points(px))),
        "font-family" | "fontFamily" => (!v.is_empty()).then_some(ParsedDecl::FontFamily(v)),
        "font-style" | "fontStyle" => match v {
            "normal" => Some(FontStyle::Normal),
            "italic" => Some(FontStyle::Italic),
            "oblique" => Some(FontStyle::Oblique),
            _ => None,
        }
        .map(ParsedDecl::FontStyle),
        "text-decoration" | "textDecoration" | "text-decoration-line" => parse_text_decoration(v).map(ParsedDecl::TextDecoration),
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
        "font-weight" | "fontWeight" => try_parse_font_weight(v).map(ParsedDecl::FontWeight),
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

/// `none` or a space-separated set of `underline` / `overline` / `line-through` lines.
fn parse_text_decoration(value: &str) -> Option<u8> {
    if value == "none" {
        return Some(0);
    }
    value.split_ascii_whitespace().try_fold(0u8, |bits, kw| {
        Some(bits | match kw {
            "underline" => text_decoration::UNDERLINE,
            "overline" => text_decoration::OVERLINE,
            "line-through" => text_decoration::LINE_THROUGH,
            _ => return None,
        })
    })
}

pub(super) fn write_parsed_decl(tx: &mut Transaction<'_>, id: NodeId, decl: ParsedDecl<'_>) {
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
        ParsedDecl::FlexBasis(l) => { tx.edit::<Layout>(id).flex_basis(l); }
        ParsedDecl::Gap(l) => { tx.edit::<Layout>(id).row_gap(l).column_gap(l); }
        ParsedDecl::RowGap(l) => { tx.edit::<Layout>(id).row_gap(l); }
        ParsedDecl::ColumnGap(l) => { tx.edit::<Layout>(id).column_gap(l); }
        ParsedDecl::Padding(l) => { tx.edit::<Layout>(id).padding_top(l).padding_right(l).padding_bottom(l).padding_left(l); }
        ParsedDecl::PaddingH(l) => { tx.edit::<Layout>(id).padding_left(l).padding_right(l); }
        ParsedDecl::PaddingV(l) => { tx.edit::<Layout>(id).padding_top(l).padding_bottom(l); }
        ParsedDecl::PaddingTop(l) => { tx.edit::<Layout>(id).padding_top(l); }
        ParsedDecl::PaddingBottom(l) => { tx.edit::<Layout>(id).padding_bottom(l); }
        ParsedDecl::PaddingLeft(l) => { tx.edit::<Layout>(id).padding_left(l); }
        ParsedDecl::PaddingRight(l) => { tx.edit::<Layout>(id).padding_right(l); }
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
        ParsedDecl::FontFamily(family) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_FONT_FAMILY;
            row.font_family(family).set(set);
        }
        ParsedDecl::FontStyle(style) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_FONT_STYLE;
            row.font_style(style).set(set);
        }
        ParsedDecl::TextDecoration(bits) => {
            let row = tx.edit::<Typography>(id);
            let set = row.get().set() | Typography::SET_TEXT_DECORATION;
            row.text_decoration(bits).set(set);
        }
    }
}
