//! Inline/shorthand CSS & RN attribute parsing into typed `LayoutProps` / `StyleProps`,
//! canonical typed & raw style key normalization, and CSS Color 4 parsing.

use nve_abi::num::{is_valid_style_scalar, parse_f32};
use nve_abi::{Dimension, LayoutProps, StyleProps};

use crate::css_shorthand;
use crate::css_values::{
    parse_color, parse_dim_checked, parse_flex_align, parse_flex_direction, parse_flex_justify,
    parse_px,
};

/// Returns the canonical kebab-case raw prop key for inline CSS declarations that are
/// forwarded via `raw_props` rather than stored in the binary ABI `LayoutProps`/`StyleProps`.
#[inline]
pub fn canonical_raw_style_key(key: &str) -> Option<&'static str> {
    match key {
        "display" => Some("display"),
        "font-weight" | "fontWeight" => Some("font-weight"),
        "font-family" | "fontFamily" => Some("font-family"),
        "font-style" | "fontStyle" => Some("font-style"),
        "text-align" | "textAlign" => Some("text-align"),
        "text-decoration" | "textDecoration" => Some("text-decoration"),
        "line-height" | "lineHeight" => Some("line-height"),
        "box-shadow" | "boxShadow" => Some("box-shadow"),
        "overflow" => Some("overflow"),
        "overflow-x" | "overflowX" => Some("overflow-x"),
        "overflow-wrap" | "overflowWrap" | "word-wrap" | "wordWrap" => Some("overflow-wrap"),
        "word-break" | "wordBreak" => Some("word-break"),
        "white-space" | "whiteSpace" => Some("white-space"),
        "vertical-align" | "verticalAlign" => Some("vertical-align"),
        "border-collapse" | "borderCollapse" => Some("border-collapse"),
        "flex-wrap" | "flexWrap" => Some("flex-wrap"),
        "object-fit" | "objectFit" => Some("object-fit"),
        _ => None,
    }
}

/// Canonical identifier for every standalone typed layout/style attribute handled by
/// [`apply_typed_attr_with_ext`]. Keeping one `from_key` table guarantees canonical key lookup
/// and parsing stay mechanically in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedAttrKind {
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    PaddingHorizontal,
    PaddingVertical,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    MarginHorizontal,
    MarginVertical,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    FlexDirection,
    JustifyContent,
    AlignItems,
    Flex,
    FlexBasis,
    FlexGrow,
    FlexShrink,
    Gap,
    Padding,
    Margin,
    Border,
    Background,
    Color,
    BorderColor,
    BorderWidth,
    BorderRadius,
    Opacity,
    FontSize,
    ZIndex,
}

impl TypedAttrKind {
    /// Resolves any supported kebab-case or camelCase standalone typed attribute name.
    #[inline]
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "padding-top" | "paddingTop" => Self::PaddingTop,
            "padding-right" | "paddingRight" => Self::PaddingRight,
            "padding-bottom" | "paddingBottom" => Self::PaddingBottom,
            "padding-left" | "paddingLeft" => Self::PaddingLeft,
            "padding-horizontal" | "paddingHorizontal" => Self::PaddingHorizontal,
            "padding-vertical" | "paddingVertical" => Self::PaddingVertical,
            "margin-top" | "marginTop" => Self::MarginTop,
            "margin-right" | "marginRight" => Self::MarginRight,
            "margin-bottom" | "marginBottom" => Self::MarginBottom,
            "margin-left" | "marginLeft" => Self::MarginLeft,
            "margin-horizontal" | "marginHorizontal" => Self::MarginHorizontal,
            "margin-vertical" | "marginVertical" => Self::MarginVertical,
            "width" => Self::Width,
            "height" => Self::Height,
            "min-width" | "minWidth" => Self::MinWidth,
            "min-height" | "minHeight" => Self::MinHeight,
            "max-width" | "maxWidth" => Self::MaxWidth,
            "max-height" | "maxHeight" => Self::MaxHeight,
            "flex-direction" | "flexDirection" => Self::FlexDirection,
            "justify-content" | "justifyContent" => Self::JustifyContent,
            "align-items" | "alignItems" => Self::AlignItems,
            "flex" => Self::Flex,
            "flex-basis" | "flexBasis" => Self::FlexBasis,
            "flex-grow" | "flexGrow" => Self::FlexGrow,
            "flex-shrink" | "flexShrink" => Self::FlexShrink,
            "gap" => Self::Gap,
            "padding" => Self::Padding,
            "margin" => Self::Margin,
            "border" => Self::Border,
            "background" | "background-color" | "backgroundColor" => Self::Background,
            "color" => Self::Color,
            "border-color" | "borderColor" => Self::BorderColor,
            "border-width" | "borderWidth" => Self::BorderWidth,
            "border-radius" | "borderRadius" => Self::BorderRadius,
            "opacity" => Self::Opacity,
            "font-size" | "fontSize" => Self::FontSize,
            "z-index" | "zIndex" => Self::ZIndex,
            _ => return None,
        })
    }
}

/// A parsed inline declaration that lives outside the binary `LayoutProps` / `StyleProps`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtStyleDecl<'a> {
    /// Typed [`crate::LayoutExt::flex_basis`] from `flex-basis` or the basis of a `flex` shorthand
    /// (`flex: none` / `auto` / `initial` yield `Dimension::Auto`).
    FlexBasis(Dimension),
    /// Whether `border-color` symbolically tracks `currentColor` (`true` for omitted or explicit
    /// `currentColor` in `border`/`border-color`; `false` for a concrete color).
    BorderCurrentColor(bool),
    /// Emitted when a valid `color` declaration explicitly sets `StyleProps::text_color`.
    ExplicitTextColor,
    /// Emitted when a valid `font-size` declaration explicitly sets `StyleProps::font_size`
    /// (including default-valued `16px`, which must block ancestor `Text` font-size inheritance).
    ExplicitFontSize,
    /// A recognized non-ABI `style` declaration (`font-weight`, `text-align`, `box-shadow`,
    /// `overflow`, ...) under its canonical kebab-case key, forwarded via `raw_props`.
    Raw(&'static str, &'a str),
}

/// Parses inline/shorthand CSS-like or RN-like attribute key/value pairs directly into typed
/// `LayoutProps` and `StyleProps` — avoiding `folly::dynamic` string parsing in C++. Extended
/// declarations ([`ExtStyleDecl`]) are dropped; use [`apply_typed_attr_with_ext`] to keep them.
pub fn apply_typed_attr(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
) -> bool {
    apply_typed_attr_with_ext(layout, style, key, value, |_| {})
}

/// Applies one standalone [`TypedAttrKind`] declaration into `(layout, style)`, invoking `on_ext`
/// for extended declarations. Returns `true` iff `value` parsed cleanly and applied (malformed CSS
/// values leave targets untouched and return `false`).
pub fn apply_typed_attr_kind<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    kind: TypedAttrKind,
    value: &str,
    mut on_ext: F,
) -> bool
where
    F: FnMut(ExtStyleDecl<'_>),
{
    let mut border_uses_current_color: Option<bool> = None;
    let mut explicit_text_color = false;
    let applied = apply_typed_attr_kind_inner(
        layout,
        style,
        kind,
        value,
        &mut border_uses_current_color,
        &mut explicit_text_color,
        &mut on_ext,
    );
    emit_style_ext(
        style,
        border_uses_current_color,
        explicit_text_color,
        applied && kind == TypedAttrKind::FontSize,
        &mut on_ext,
    );
    applied
}

/// Parses inline/shorthand CSS-like or RN-like attributes into typed `LayoutProps` and
/// `StyleProps`, invoking `on_ext` for every recognized declaration outside the binary ABI
/// (typed [`ExtStyleDecl::FlexBasis`], symbolic `currentColor` / explicit `color` /
/// explicit `font-size` signals, or a raw [`ExtStyleDecl::Raw`] declaration).
///
/// Returns `true` whenever `key` is `"style"` or a recognized [`TypedAttrKind`] (even if `value`
/// is invalid so callers do not leak typed keys to raw props); use [`apply_typed_attr_kind`] when
/// you need to distinguish whether `value` parsed cleanly.
pub fn apply_typed_attr_with_ext<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    key: &str,
    value: &str,
    mut on_ext: F,
) -> bool
where
    F: FnMut(ExtStyleDecl<'_>),
{
    let mut border_uses_current_color: Option<bool> = None;
    let mut explicit_text_color = false;
    let mut explicit_font_size = false;
    if key == "style" {
        for decl in value.split(';') {
            if let Some((raw_k, raw_v)) = decl.split_once(':') {
                let k = raw_k.trim();
                let v = raw_v.trim();
                if k.is_empty() || v.is_empty() {
                    continue;
                }
                if let Some(kind) = TypedAttrKind::from_key(k) {
                    if apply_typed_attr_kind_inner(
                        layout,
                        style,
                        kind,
                        v,
                        &mut border_uses_current_color,
                        &mut explicit_text_color,
                        &mut on_ext,
                    ) && kind == TypedAttrKind::FontSize
                    {
                        explicit_font_size = true;
                    }
                } else if let Some(canonical) = canonical_raw_style_key(k) {
                    on_ext(ExtStyleDecl::Raw(canonical, v));
                }
            }
        }
        emit_style_ext(
            style,
            border_uses_current_color,
            explicit_text_color,
            explicit_font_size,
            &mut on_ext,
        );
        return true;
    }
    let Some(kind) = TypedAttrKind::from_key(key) else {
        return false;
    };
    if apply_typed_attr_kind_inner(
        layout,
        style,
        kind,
        value,
        &mut border_uses_current_color,
        &mut explicit_text_color,
        &mut on_ext,
    ) && kind == TypedAttrKind::FontSize
    {
        explicit_font_size = true;
    }
    emit_style_ext(
        style,
        border_uses_current_color,
        explicit_text_color,
        explicit_font_size,
        &mut on_ext,
    );
    true
}

#[inline]
fn emit_style_ext<F>(
    style: &mut StyleProps,
    border_uses_current_color: Option<bool>,
    explicit_text_color: bool,
    explicit_font_size: bool,
    on_ext: &mut F,
) where
    F: FnMut(ExtStyleDecl<'_>),
{
    if explicit_font_size {
        on_ext(ExtStyleDecl::ExplicitFontSize);
    }
    if explicit_text_color {
        on_ext(ExtStyleDecl::ExplicitTextColor);
    }
    if let Some(uses_current) = border_uses_current_color {
        if uses_current {
            style.border_color = style.text_color;
        }
        on_ext(ExtStyleDecl::BorderCurrentColor(uses_current));
    }
}

fn apply_typed_attr_kind_inner<F>(
    layout: &mut LayoutProps,
    style: &mut StyleProps,
    kind: TypedAttrKind,
    value: &str,
    border_uses_current_color: &mut Option<bool>,
    explicit_text_color: &mut bool,
    on_ext: &mut F,
) -> bool
where
    F: FnMut(ExtStyleDecl<'_>),
{
    use TypedAttrKind as K;
    match kind {
        K::PaddingTop => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.top = v;
        }
        K::PaddingRight => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.right = v;
        }
        K::PaddingBottom => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.bottom = v;
        }
        K::PaddingLeft => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.left = v;
        }
        K::PaddingHorizontal => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.left = v;
            layout.padding.right = v;
        }
        K::PaddingVertical => {
            let Some(v) = css_shorthand::parse_edge(value, false) else { return false };
            layout.padding.top = v;
            layout.padding.bottom = v;
        }
        K::MarginTop => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.top = v;
        }
        K::MarginRight => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.right = v;
        }
        K::MarginBottom => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.bottom = v;
        }
        K::MarginLeft => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.left = v;
        }
        K::MarginHorizontal => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.left = v;
            layout.margin.right = v;
        }
        K::MarginVertical => {
            let Some(v) = css_shorthand::parse_edge(value, true) else { return false };
            layout.margin.top = v;
            layout.margin.bottom = v;
        }
        K::Width => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.width = d;
        }
        K::Height => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.height = d;
        }
        K::MinWidth => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.min_width = d;
        }
        K::MinHeight => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.min_height = d;
        }
        K::MaxWidth => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.max_width = d;
        }
        K::MaxHeight => {
            let Some(d) = parse_dim_checked(value) else { return false };
            layout.max_height = d;
        }
        K::FlexDirection => {
            let Some(dir) = parse_flex_direction(value) else { return false };
            layout.flex_direction = dir;
        }
        K::JustifyContent => {
            let Some(just) = parse_flex_justify(value) else { return false };
            layout.justify_content = just;
        }
        K::AlignItems => {
            let Some(align) = parse_flex_align(value) else { return false };
            layout.align_items = align;
        }
        K::Flex => {
            let Some(flex) = css_shorthand::parse_flex(value) else { return false };
            layout.flex_grow = flex.grow;
            layout.flex_shrink = flex.shrink;
            on_ext(ExtStyleDecl::FlexBasis(flex.basis));
        }
        K::FlexBasis => {
            let Some(basis) = css_shorthand::parse_flex_basis(value) else { return false };
            on_ext(ExtStyleDecl::FlexBasis(basis));
        }
        K::FlexGrow => {
            let Some(v) = parse_f32(value.trim()).filter(|&v| is_valid_style_scalar(v) && v >= 0.0)
            else {
                return false;
            };
            layout.flex_grow = v;
        }
        K::FlexShrink => {
            let Some(v) = parse_f32(value.trim()).filter(|&v| is_valid_style_scalar(v) && v >= 0.0)
            else {
                return false;
            };
            layout.flex_shrink = v;
        }
        K::Gap => {
            let Some(v) = parse_px(value).filter(|v| *v >= 0.0) else { return false };
            layout.gap = v;
        }
        K::Padding => {
            let Some(edges) = css_shorthand::parse_edges(value, false) else { return false };
            layout.padding = edges;
        }
        K::Margin => {
            let Some(edges) = css_shorthand::parse_edges(value, true) else { return false };
            layout.margin = edges;
        }
        K::Border => {
            let Some((width, color)) = css_shorthand::parse_border(value) else { return false };
            style.border_width = width;
            match color {
                Some(c) => {
                    style.border_color = c;
                    *border_uses_current_color = Some(false);
                }
                None => *border_uses_current_color = Some(true),
            }
        }
        K::Background => {
            let Some(c) = parse_color(value) else { return false };
            style.background = c;
        }
        K::Color => {
            let Some(c) = parse_color(value) else { return false };
            style.text_color = c;
            *explicit_text_color = true;
        }
        K::BorderColor => {
            if value.trim().eq_ignore_ascii_case("currentcolor") {
                *border_uses_current_color = Some(true);
            } else if let Some(c) = parse_color(value) {
                style.border_color = c;
                *border_uses_current_color = Some(false);
            } else {
                return false;
            }
        }
        K::BorderWidth => {
            let Some(v) = css_shorthand::parse_border_width_value(value) else { return false };
            style.border_width = v;
        }
        K::BorderRadius => {
            let Some(v) = parse_px(value).filter(|v| *v >= 0.0) else { return false };
            style.border_radius = v;
        }
        K::Opacity => {
            let Some(v) = parse_f32(value.trim()) else { return false };
            style.opacity = v.clamp(0.0, 1.0);
        }
        K::FontSize => {
            let Some(v) = parse_px(value).filter(|v| *v > 0.0) else { return false };
            style.font_size = v;
        }
        K::ZIndex => {
            let Ok(v) = value.trim().parse::<i32>() else { return false };
            style.z_index = v;
        }
    }
    true
}
